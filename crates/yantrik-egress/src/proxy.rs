//! The proxy: one connection, from its head to the tunnel.
//!
//! In order, each step able to end it with a sentence:
//!
//! 1. the caller's account, read from the kernel, must be the mind's;
//! 2. the head, read whole (bounded in size and time), must be a request this proxy serves;
//! 3. what can be decided without a lookup is (Private mode; in enforce, whether any rule covers
//!    the name) — a lookup is a message to whoever serves the name, so nothing is resolved that
//!    may not be reached;
//! 4. the name is resolved here, the mind resolves nothing itself, and every address it gave is
//!    classed: this machine's own, loopback and the like go nowhere;
//! 5. the policy's verdict, counted either way;
//! 6. the connection, then bytes both ways until either end closes or both are quiet for
//!    [`IDLE`]. A plain-HTTP request forwards exactly the body its head declared, and no second
//!    request after it.

use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

use crate::ledger::Outcome;
use crate::policy::{place_of, Place, Verdict};
use crate::request::{self, Target};
use crate::state::State;

/// How long a caller has to send its head, and a name to resolve, and an upstream to answer.
const HEAD_TIME: Duration = Duration::from_secs(10);
const RESOLVE_TIME: Duration = Duration::from_secs(10);
const CONNECT_TIME: Duration = Duration::from_secs(15);
/// A tunnel with nothing through it either way for this long is closed: a pooled connection
/// nobody uses, or one whose far end vanished, gives its place back.
pub const IDLE: Duration = Duration::from_secs(600);
/// The most connections at once.
pub const MOST_OPEN: usize = 256;

pub struct Proxy {
    pub state: Arc<Mutex<State>>,
    /// The one account served: the mind's.
    pub serve_uid: u32,
    /// Where the proxy listens, for finding the caller's socket.
    pub local: SocketAddr,
}

pub async fn serve(listener: TcpListener, proxy: Arc<Proxy>) {
    let open = Arc::new(Semaphore::new(MOST_OPEN));
    loop {
        let (mut stream, peer) = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                // Out of file descriptors, say: back off rather than spin.
                tracing::warn!(error = %e, "accept failed");
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        let Ok(permit) = open.clone().try_acquire_owned() else {
            let _ = reply(&mut stream, 503, "the proxy is at its limit of open connections; try again").await;
            continue;
        };
        let proxy = proxy.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(why) = one(&proxy, &mut stream, peer).await {
                tracing::debug!(%peer, why, "connection ended");
            }
        });
    }
}

async fn reply(stream: &mut TcpStream, code: u16, why: &str) -> std::io::Result<()> {
    let reason = match code {
        400 => "Bad Request",
        403 => "Forbidden",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let body = format!("{why}\n");
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nX-Yantrik-Egress: refused\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// TCP keepalive, probing after a minute of quiet, so a far end that vanished is noticed well
/// inside [`IDLE`] rather than after the kernel's two hours.
fn keepalive(s: &TcpStream) {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let set = |level: libc::c_int, name: libc::c_int, value: libc::c_int| {
            // SAFETY: setsockopt on a socket we own, with a pointer to an int that outlives the call.
            let rc = unsafe {
                libc::setsockopt(
                    s.as_raw_fd(),
                    level,
                    name,
                    &value as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                )
            };
            if rc != 0 {
                tracing::debug!(name, "a keepalive option was not taken");
            }
        };
        set(libc::SOL_SOCKET, libc::SO_KEEPALIVE, 1);
        #[cfg(target_os = "linux")]
        {
            set(libc::IPPROTO_TCP, libc::TCP_KEEPIDLE, 60);
            set(libc::IPPROTO_TCP, libc::TCP_KEEPINTVL, 15);
            set(libc::IPPROTO_TCP, libc::TCP_KEEPCNT, 4);
        }
    }
}

impl Proxy {
    fn record(&self, host: &str, port: u16, outcome: Outcome, lan: bool, http: bool, why: &str) {
        if let Ok(mut s) = self.state.lock() {
            s.ledger.record(host, port, outcome, lan, http, why, now());
        }
    }
}

async fn one(proxy: &Proxy, stream: &mut TcpStream, peer: SocketAddr) -> Result<(), &'static str> {
    // 1. Who, before anything is read: a caller that is not the mind is told so and nothing more.
    let local = stream.local_addr().unwrap_or(proxy.local);
    let uid = tokio::task::spawn_blocking(move || crate::peer::uid_of(peer, local)).await.ok().flatten();
    if uid != Some(proxy.serve_uid) {
        let _ = reply(stream, 403, "this proxy serves the mind account only").await;
        return Err("not the mind");
    }

    // 2. The head.
    let mut buf = Vec::with_capacity(2048);
    let head_len = tokio::time::timeout(HEAD_TIME, async {
        let mut chunk = [0u8; 4096];
        loop {
            if let Some(end) = request::head_end(&buf) {
                return Ok(end);
            }
            if buf.len() >= request::MOST_HEAD {
                return Err("the head is too long");
            }
            let n = stream.read(&mut chunk).await.map_err(|_| "read failed")?;
            if n == 0 {
                return Err("closed before its head ended");
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    })
    .await
    .map_err(|_| "the head took too long")
    .and_then(|r| r);
    let head_len = match head_len {
        Ok(n) => n,
        Err(why) => {
            let _ = reply(stream, 400, why).await;
            return Err(why);
        }
    };
    let target = match request::parse(&buf[..head_len]) {
        Ok(t) => t,
        Err(request::Bad(why)) => {
            let _ = reply(stream, 400, why).await;
            return Err(why);
        }
    };
    let rest = buf[head_len..].to_vec();
    let (host, port, http) = match &target {
        Target::Connect { host, port } => (host.clone(), *port, false),
        Target::Http { host, port, .. } => (host.clone(), *port, true),
    };

    // 3. What needs no lookup.
    let early = {
        let Ok(s) = proxy.state.lock() else { return Err("state poisoned") };
        s.policy.before_resolve(&host, port, http, s.private)
    };
    if let Some(Verdict::Refuse(why)) = early {
        proxy.record(&host, port, Outcome::Refused, false, http, &why);
        let _ = reply(stream, 403, &why).await;
        return Err("refused before resolving");
    }

    // 4. Resolved here — or not at all, for an address — and every address classed.
    let addrs: Vec<SocketAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => match tokio::time::timeout(RESOLVE_TIME, tokio::net::lookup_host((host.as_str(), port))).await {
            Ok(Ok(a)) => a.collect(),
            _ => Vec::new(),
        },
    };
    if addrs.is_empty() {
        let why = format!("{host} did not resolve");
        proxy.record(&host, port, Outcome::Refused, false, http, &why);
        let _ = reply(stream, 502, &why).await;
        return Err("did not resolve");
    }
    let own = crate::local::addresses();
    if own.is_none() {
        tracing::warn!("this machine's own addresses could not be read; every local-network address is refused");
    }
    let class = |a: &SocketAddr| {
        if crate::local::is_own(a.ip(), own.as_deref()) {
            Place::Forbidden
        } else {
            place_of(a.ip())
        }
    };
    let usable: Vec<SocketAddr> = addrs.iter().copied().filter(|a| class(a) != Place::Forbidden).collect();
    let place = if usable.is_empty() {
        Place::Forbidden
    } else if usable.iter().all(|a| class(a) == Place::Lan) {
        Place::Lan
    } else {
        Place::Internet
    };
    // A name that gives both kinds is reached at its internet addresses only: the verdict is for
    // the place it is actually reached at.
    let usable: Vec<SocketAddr> = match place {
        Place::Internet => usable.into_iter().filter(|a| class(a) == Place::Internet).collect(),
        _ => usable,
    };

    // 5. The verdict.
    let verdict = {
        let Ok(s) = proxy.state.lock() else { return Err("state poisoned") };
        s.policy.decide(&host, port, http, place, s.private)
    };
    let (outcome, why) = match &verdict {
        Verdict::Allow { audit: true } => (Outcome::Audited, ""),
        Verdict::Allow { audit: false } => (Outcome::Allowed, ""),
        Verdict::Refuse(why) => (Outcome::Refused, why.as_str()),
    };
    proxy.record(&host, port, outcome, place == Place::Lan, http, why);
    if let Verdict::Refuse(why) = verdict {
        tracing::info!(host, port, why, "refused");
        let _ = reply(stream, 403, &why).await;
        return Err("refused");
    }

    // 6. The connection.
    let mut upstream = None;
    for addr in &usable {
        if let Ok(Ok(s)) = tokio::time::timeout(CONNECT_TIME, TcpStream::connect(addr)).await {
            upstream = Some(s);
            break;
        }
    }
    let Some(mut upstream) = upstream else {
        let _ = reply(stream, 502, &format!("{host}:{port} did not answer")).await;
        return Err("upstream did not answer");
    };
    keepalive(stream);
    keepalive(&upstream);

    match target {
        Target::Connect { .. } => {
            stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.map_err(|_| "write failed")?;
            if !rest.is_empty() {
                upstream.write_all(&rest).await.map_err(|_| "write failed")?;
            }
            splice(stream, &mut upstream).await;
        }
        Target::Http { head, body, .. } => {
            upstream.write_all(&head).await.map_err(|_| "write failed")?;
            // Exactly the declared body: what arrived with the head, then the rest from the
            // caller — and nothing after it, which would be a second request.
            let first = rest.len().min(body as usize);
            upstream.write_all(&rest[..first]).await.map_err(|_| "write failed")?;
            let mut left = body - first as u64;
            let mut chunk = vec![0u8; 16 * 1024];
            while left > 0 {
                let want = chunk.len().min(left as usize);
                let n = match tokio::time::timeout(IDLE, stream.read(&mut chunk[..want])).await {
                    Ok(Ok(n)) if n > 0 => n,
                    _ => return Err("the body did not arrive"),
                };
                upstream.write_all(&chunk[..n]).await.map_err(|_| "write failed")?;
                left -= n as u64;
            }
            // Not half-closed: some servers drop a request whose sender closed its side before
            // the answer (found live, with example.com). `Connection: close` ends it, and nothing
            // more is read from the caller, so nothing more can follow. The answer, to its end.
            loop {
                let n = match tokio::time::timeout(IDLE, upstream.read(&mut chunk)).await {
                    Ok(Ok(n)) if n > 0 => n,
                    _ => break,
                };
                if stream.write_all(&chunk[..n]).await.is_err() {
                    break;
                }
            }
        }
    }
    Ok(())
}

/// Bytes both ways — each direction on its own, so neither waits on the other, and a side that
/// finishes sending leaves the other still answering — until both are done or nothing has moved
/// either way for [`IDLE`].
async fn splice(client: &mut TcpStream, upstream: &mut TcpStream) {
    let start = Instant::now();
    let last = Arc::new(AtomicU64::new(0));
    let mut c = Watched { inner: client, last: last.clone(), start };
    let mut u = Watched { inner: upstream, last: last.clone(), start };
    let watchdog = async {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let quiet = start.elapsed().saturating_sub(Duration::from_millis(last.load(Ordering::Relaxed)));
            if quiet >= IDLE {
                return;
            }
        }
    };
    tokio::select! {
        _ = tokio::io::copy_bidirectional(&mut c, &mut u) => {}
        _ = watchdog => { tracing::debug!("a tunnel closed after being quiet"); }
    }
}

/// A stream that notes when bytes last moved through it.
struct Watched<'a> {
    inner: &'a mut TcpStream,
    last: Arc<AtomicU64>,
    start: Instant,
}

impl Watched<'_> {
    fn touch(&self) {
        self.last.store(self.start.elapsed().as_millis() as u64, Ordering::Relaxed);
    }
}

impl AsyncRead for Watched<'_> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let r = Pin::new(&mut *self.inner).poll_read(cx, buf);
        if buf.filled().len() > before {
            self.touch();
        }
        r
    }
}

impl AsyncWrite for Watched<'_> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, data: &[u8]) -> Poll<std::io::Result<usize>> {
        let r = Pin::new(&mut *self.inner).poll_write(cx, data);
        if matches!(r, Poll::Ready(Ok(n)) if n > 0) {
            self.touch();
        }
        r
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.inner).poll_shutdown(cx)
    }
}
