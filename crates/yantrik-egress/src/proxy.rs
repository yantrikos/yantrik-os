//! The proxy: one connection, from its head to the tunnel.
//!
//! In order, each step able to end it with a sentence:
//!
//! 1. the caller's account, read from the kernel, must be the mind's;
//! 2. the head, read whole (bounded in size and time), must be a request this proxy serves;
//! 3. what can be decided without a lookup is (Private mode; in enforce, whether any rule covers
//!    the name; on the public door, whether a `lan` rule names it — `crate::door`) — a lookup is
//!    a message to whoever serves the name, so nothing is resolved that may not be reached;
//! 4. the name is resolved here, the mind resolves nothing itself, and every address it gave is
//!    classed: this machine's own, loopback and the like go nowhere; an address on a prefix this
//!    machine is on, or one of its routers, is the local network whatever its range
//!    (`crate::local::place`) — the network read after the lookup, no older than a second, and
//!    when it cannot be read and never was, nothing is reached (503);
//! 5. the door's verdict (the public door: the internet only), then the policy's, counted
//!    either way;
//! 6. the connection — left at once if it leaves from an address the network read did not know
//!    (a network that came up in the last second), when the network read again then makes the
//!    destination the local network or this machine and the verdict on that is a refusal
//!    ([`after_connect`]) — then bytes both ways until either end closes or both are quiet for
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

use crate::door::Door;
use crate::ledger::Outcome;
use crate::policy::{Place, Verdict};
use crate::request::{self, Target};
use crate::state::State;

/// How long a caller has to send its head, and a name to resolve, and an upstream to answer.
const HEAD_TIME: Duration = Duration::from_secs(10);
const RESOLVE_TIME: Duration = Duration::from_secs(10);
const CONNECT_TIME: Duration = Duration::from_secs(if cfg!(test) { 2 } else { 15 });
/// A tunnel with nothing through it either way for this long is closed: a pooled connection
/// nobody uses, or one whose far end vanished, gives its place back.
pub const IDLE: Duration = Duration::from_secs(600);
/// The most connections at once.
pub const MOST_OPEN: usize = 256;
/// What a refusal added to by [`crate::local::Net::assumed`] says opens the destination: a `lan`
/// rule, which only the endpoint door ever consults, so this is said on that door alone — on the
/// public one it would send the person to a rule that can never help.
const LAN_RULE: &str = " A rule with `lan: true` reaches it.";

pub struct Proxy {
    pub state: Arc<Mutex<State>>,
    /// The one account served: the mind's.
    pub serve_uid: u32,
    /// Where the proxy listens, for finding the caller's socket.
    pub local: SocketAddr,
    /// Which door this listener is.
    pub door: Door,
    /// The kernel's word on this machine's network, shared by both doors (`crate::local::Watch`).
    pub net: Arc<crate::local::Watch>,
    /// How names are looked up.
    pub world: World,
}

/// What the proxy asks of the world around it, other than its network: for a test, names it
/// answers itself.
#[derive(Clone, Copy)]
pub struct World {
    /// `Some` is the answer (empty: did not resolve); `None` asks the system's resolver.
    pub names: fn(&str) -> Option<Vec<IpAddr>>,
}

fn system_names(_: &str) -> Option<Vec<IpAddr>> {
    None
}

impl World {
    pub const REAL: World = World { names: system_names };
}

/// Serve one listener. `open` is shared by both doors: [`MOST_OPEN`] is for the proxy as a whole.
pub async fn serve(listener: TcpListener, proxy: Arc<Proxy>, open: Arc<Semaphore>) {
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
                tracing::debug!(door = proxy.door.name(), %peer, why, "connection ended");
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

    // 3. What needs no lookup. An address that is never a destination is refused as that, in
    // every mode: no rule could reach it, so it is never a proposal. Only then is the network
    // read here: with nothing to resolve, there is nothing to read it after.
    let early = {
        let Ok(s) = proxy.state.lock() else { return Err("state poisoned") };
        let early = s.policy.before_resolve(&host, port, http, s.private).or_else(|| proxy.door.before_resolve(&s.policy, &host));
        early.map(|v| (v, s.private))
    };
    let early = match (early, host.parse::<IpAddr>()) {
        (Some((Verdict::Refuse(why), false)), Ok(ip)) => {
            // Without a network to judge by, only the ranges say what is never a destination; the
            // rest is counted as refused, which a rule might answer.
            let forbidden = match proxy.net.now() {
                Some(net) => classify(&[SocketAddr::new(ip, port)], Some(&net)).0 == Place::Forbidden,
                None => crate::policy::place_of(ip) == Place::Forbidden,
            };
            if forbidden {
                let Ok(s) = proxy.state.lock() else { return Err("state poisoned") };
                Some((s.policy.decide(&host, port, http, Place::Forbidden, false), Outcome::Never))
            } else {
                Some((Verdict::Refuse(why), Outcome::Refused))
            }
        }
        (early, _) => early.map(|(v, _)| (v, Outcome::Refused)),
    };
    if let Some((Verdict::Refuse(why), outcome)) = early {
        tracing::info!(door = proxy.door.name(), host, port, why, "refused");
        proxy.record(&host, port, outcome, false, http, &why);
        let _ = reply(stream, 403, &why).await;
        return Err("refused before resolving");
    }

    // 4. Resolved here — or not at all, for an address — and every address classed.
    let addrs: Vec<SocketAddr> = match (host.parse::<IpAddr>(), (proxy.world.names)(&host)) {
        (Ok(ip), _) => vec![SocketAddr::new(ip, port)],
        (Err(_), Some(ips)) => ips.into_iter().map(|ip| SocketAddr::new(ip, port)).collect(),
        (Err(_), None) => match tokio::time::timeout(RESOLVE_TIME, tokio::net::lookup_host((host.as_str(), port))).await {
            Ok(Ok(a)) => a.collect(),
            _ => Vec::new(),
        },
    };
    if addrs.is_empty() {
        // Counted, but never a proposal: no rule makes a name resolve.
        let why = format!("{host} did not resolve");
        tracing::info!(door = proxy.door.name(), host, port, why, "refused");
        proxy.record(&host, port, Outcome::Never, false, http, &why);
        let _ = reply(stream, 502, &why).await;
        return Err("did not resolve");
    }
    // The network as it is now, after the lookup: no older than a second, or the last one read.
    // Never read at all: where anything leads cannot be told, so nothing is reached — refused
    // as never a destination when the ranges alone say so, else 503.
    let net = proxy.net.now();
    let by_range = addrs.iter().all(|a| crate::policy::place_of(a.ip()) == Place::Forbidden);
    let Some(net) = net.or_else(|| by_range.then(crate::local::Net::default)) else {
        let why = "this machine's network could not be read, so where this leads cannot be told; try again";
        tracing::warn!(door = proxy.door.name(), host, port, why, "refused");
        proxy.record(&host, port, Outcome::Refused, false, http, why);
        let _ = reply(stream, 503, why).await;
        return Err("no network");
    };
    let (place, usable) = classify(&addrs, Some(&net));

    // 5. The verdict: the door's, then the policy's. Private mode is the policy's word, first.
    let (verdict, private) = verdict(proxy, &host, port, http, place);
    // The local network only by the /56 guessed around this machine's address: said, so the
    // person knows why, and on this door what opens it.
    let verdict = match verdict {
        Verdict::Refuse(why) if place == Place::Lan && !private => match usable.iter().map(|a| net.assumed(a.ip())).collect::<Option<Vec<_>>>() {
            Some(by) if !by.is_empty() => {
                // Said with the guess it follows, not at the end of the whole refusal: the ledger
                // keeps only the first 300 characters of a why.
                let guess = if proxy.door == Door::Endpoint { format!("{}{LAN_RULE}", by[0]) } else { by[0].clone() };
                Verdict::Refuse(format!("{guess} {why}"))
            }
            _ => Verdict::Refuse(why),
        },
        v => v,
    };
    if let Verdict::Refuse(why) = &verdict {
        let outcome = Outcome::of(&verdict, place, private);
        proxy.record(&host, port, outcome, place == Place::Lan, http, why);
        tracing::info!(door = proxy.door.name(), host, port, ?outcome, why, "decided");
        let _ = reply(stream, 403, why).await;
        return Err("refused");
    }

    // 6. The connection, counted once it is made, or found not to be.
    let mut upstream = None;
    for addr in &usable {
        if let Ok(Ok(s)) = tokio::time::timeout(CONNECT_TIME, TcpStream::connect(addr)).await {
            upstream = Some((s, *addr));
            break;
        }
    }
    let outcome = Outcome::of(&verdict, place, private);
    let Some((mut upstream, to)) = upstream else {
        proxy.record(&host, port, outcome, place == Place::Lan, http, "");
        tracing::info!(door = proxy.door.name(), host, port, ?outcome, "decided");
        let _ = reply(stream, 502, &format!("{host}:{port} did not answer")).await;
        return Err("upstream did not answer");
    };
    let from = upstream.local_addr().map(|a| a.ip()).ok();
    if let Err(late) = after_connect(proxy, &host, port, http, place, &net, to, from) {
        drop(upstream);
        proxy.record(&host, port, late.outcome, late.place == Place::Lan, http, &late.why);
        tracing::warn!(door = proxy.door.name(), host, port, outcome = ?late.outcome, why = late.why, "refused once connected");
        let _ = reply(stream, late.code, &late.why).await;
        return Err("refused once connected");
    }
    proxy.record(&host, port, outcome, place == Place::Lan, http, "");
    tracing::info!(door = proxy.door.name(), host, port, ?outcome, "decided");
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

/// The door's verdict, then the policy's, on `host` reached at `place`; and whether Private mode
/// is on.
fn verdict(proxy: &Proxy, host: &str, port: u16, http: bool, place: Place) -> (Verdict, bool) {
    let Ok(s) = proxy.state.lock() else {
        return (Verdict::Refuse("the proxy's state is poisoned".into()), true);
    };
    let verdict = match s.policy.decide(host, port, http, place, s.private) {
        Verdict::Refuse(why) => Verdict::Refuse(why),
        allow => proxy.door.after_resolve(host, place).unwrap_or(allow),
    };
    (verdict, s.private)
}

/// A connection refused after it was made: the answer, and how it is counted.
#[derive(Debug)]
pub struct Late {
    pub code: u16,
    pub why: String,
    pub outcome: Outcome,
    pub place: Place,
}

/// Whether a connection made to `to`, judged `place` on `net`, still stands. The network was read
/// at most a second before; a network that came up since brought a new address of this machine
/// and its prefix, which `net` does not know. The connection says so: it left from `from`. Then
/// the network is read again and `to` judged on it, and when it is now the local network or this
/// machine, the verdict on that place is the one that counts — a refusal ends the connection
/// before a byte has gone through it. A network that cannot be read again then ends it too: where
/// it leads cannot be told.
#[allow(clippy::too_many_arguments)]
pub fn after_connect(proxy: &Proxy, host: &str, port: u16, http: bool, place: Place, net: &crate::local::Net, to: SocketAddr, from: Option<IpAddr>) -> Result<(), Late> {
    if from.is_some_and(|f| crate::local::is_own(f, Some(&net.own))) {
        return Ok(());
    }
    let Some(now) = proxy.net.refresh() else {
        let why = "this machine's network changed while connecting and could not be read again, so where this leads cannot be told; try again";
        return Err(Late { code: 503, why: why.into(), outcome: Outcome::Refused, place });
    };
    let (moved, _) = classify(&[to], Some(&now));
    if moved == place || moved == Place::Internet {
        return Ok(());
    }
    match verdict(proxy, host, port, http, moved) {
        (Verdict::Allow { .. }, _) => Ok(()),
        (Verdict::Refuse(why), private) => {
            let refusal = Verdict::Refuse(format!("{why} (this machine's network changed while connecting)"));
            let outcome = Outcome::of(&refusal, moved, private);
            let Verdict::Refuse(why) = refusal else { return Ok(()) };
            Err(Late { code: 403, why, outcome, place: moved })
        }
    }
}

/// Where `addrs` lead from this machine on `net`, and which of them to connect to. Nothing that
/// is never a destination (this machine's own addresses included); an address on one of its
/// prefixes, or one of its routers, is the local network; a name that gives both kinds is reached
/// at its internet addresses only, so the verdict is for the place it is actually reached at.
pub fn classify(addrs: &[SocketAddr], net: Option<&crate::local::Net>) -> (Place, Vec<SocketAddr>) {
    let class = |a: &SocketAddr| crate::local::place(a.ip(), net);
    let usable: Vec<SocketAddr> = addrs.iter().copied().filter(|a| class(a) != Place::Forbidden).collect();
    let place = if usable.is_empty() {
        Place::Forbidden
    } else if usable.iter().all(|a| class(a) == Place::Lan) {
        Place::Lan
    } else {
        Place::Internet
    };
    let usable = match place {
        Place::Internet => usable.into_iter().filter(|a| class(a) == Place::Internet).collect(),
        _ => usable,
    };
    (place, usable)
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
