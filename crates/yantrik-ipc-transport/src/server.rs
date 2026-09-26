//! JSON-RPC server — Unix domain sockets (Linux) or TCP localhost (Windows dev).

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::protocol::{RpcRequest, RpcResponse, RPC_INTERNAL_ERROR, RPC_METHOD_NOT_FOUND, RPC_PARSE_ERROR};

/// Directory holding this session's service sockets.
///
/// Picks the first candidate we can actually create and write, rather than
/// trusting one path. `$XDG_RUNTIME_DIR` is the correct answer on a normal
/// desktop session, but it is routinely unset — or set to a path that does not
/// exist — in containers, WSL without systemd, and bare `ssh` sessions. The
/// previous hardcoded `/run/yantrik` was unwritable for a user-run service, so
/// every service died on bind.
///
/// Order: `$XDG_RUNTIME_DIR/yantrik` → `/run/yantrik` (root/system service) →
/// `/tmp/yantrik-<uid>` (last resort, always writable).
#[cfg(unix)]
pub fn socket_dir() -> std::path::PathBuf {
    use std::path::PathBuf;

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        let dir = dir.trim();
        if !dir.is_empty() {
            candidates.push(PathBuf::from(dir).join("yantrik"));
        }
    }
    candidates.push(PathBuf::from("/run/yantrik"));
    // SAFETY: getuid() is always safe — it cannot fail and touches no memory.
    let uid = unsafe { libc::getuid() };
    let last_resort = PathBuf::from(format!("/tmp/yantrik-{uid}"));
    candidates.push(last_resort.clone());

    // Why each rejection is logged: this function silently falls through to the last candidate,
    // so a failure on the *first* one surfaces later as a bind error naming the *last* one.
    // perception-service died with "cannot create socket directory /tmp/yantrik-0" while holding
    // a perfectly good /run/user/1000/yantrik, and the message sent the diagnosis to the wrong
    // directory entirely. A fallback chain that does not say why it fell back is a chain that
    // lies about where the problem is.
    for dir in &candidates {
        // Only create it if it is not already there.
        //
        // `create_dir_all` looks idempotent and is not, under Landlock. On an existing directory
        // it still issues `mkdir`, and a ruleset without MAKE_DIR denies that with EACCES —
        // before the kernel ever reaches the EEXIST that `std` would have translated into "fine,
        // it exists". So a service that resolves this directory once, applies a ruleset over it,
        // and resolves it again gets a permission error on the directory it just made itself, and
        // falls through to candidates it can create even less. That is exactly how
        // perception-service died pointing at /tmp/yantrik-0.
        if !dir.is_dir() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                tracing::debug!(dir = %dir.display(), error = %e, "socket dir candidate: cannot create");
                continue;
            }
        }
        if let Err(e) = harden(dir) {
            tracing::debug!(dir = %dir.display(), error = %e, "socket dir candidate: cannot harden");
            continue;
        }
        tracing::debug!(dir = %dir.display(), "socket dir chosen");
        return dir.clone();
    }
    tracing::warn!(
        candidates = ?candidates,
        "no socket directory could be prepared; falling back to the last candidate, which will \
         almost certainly fail to bind"
    );
    last_resort
}

/// Restrict a socket directory to its owner.
///
/// Matters most for the `/tmp` fallback: `/tmp` is world-writable, and these
/// sockets expose system-monitor, network and notification control. Without
/// this, any local user could drive them.
#[cfg(unix)]
fn harden(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::metadata(dir)?.permissions();
    // Already private: say so and touch nothing.
    //
    // This early return is what makes `socket_dir` idempotent, and that matters more than it
    // looks. perception-service calls it once to learn where its socket goes, applies a Landlock
    // ruleset over that directory, and then the service SDK calls it again to bind. The second
    // call used to re-issue this chmod — which Landlock denies, because the ruleset grants writes
    // *inside* the directory and not the right to change the directory itself. So the second call
    // failed on a directory the first call had just successfully created, fell through to
    // candidates it could not create either, and reported the last one's error. The service died
    // with "cannot create socket directory /tmp/yantrik-0" while holding a perfectly good
    // /run/user/1000/yantrik it had made moments earlier.
    if perms.mode() & 0o777 == 0o700 {
        return Ok(());
    }
    let mut perms = perms;
    perms.set_mode(0o700);
    std::fs::set_permissions(dir, perms)
}

/// Take the world bits off a socket we just bound.
///
/// `bind` creates the socket node under the process umask, which on every machine this ships to
/// is 022 — so the file came out `srwxr-xr-x`. Connecting to a unix socket needs *write* on the
/// node, and `x` on a socket means nothing, so 0755 was never the thing letting another user in;
/// the directory's 0700 was the thing keeping them out. This is the belt to that braces, written
/// down as a decision in design/approvals-2026-09-21.md and taken here.
///
/// There is a window between `bind` and this `chmod` in which the node exists at 0755. It is not
/// closed, and it does not need to be: for the whole of that window the node is inside a
/// directory no other uid may traverse, so nobody else can name it, let alone open it. Closing it
/// properly would mean `umask(0o177)` around the bind — process-global state, in a process that
/// binds sockets from several threads, which would be a worse bug than the one being fixed.
///
/// Best-effort on purpose. A failure here leaves the node exactly as it was before this function
/// existed — 0755 inside a 0700 directory, which is what every shipped machine has been running.
/// Returning the error instead would take down a service over a defence-in-depth measure, and a
/// desktop that will not start is a worse outcome than a socket mode that is merely no better
/// than yesterday's.
#[cfg(unix)]
fn private_socket_file(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        tracing::warn!(
            socket = %path.display(),
            error = %e,
            "could not take the group and world bits off this socket; it stays at the umask \
             default. The directory's 0700 is still what keeps other users out."
        );
    }
}

/// Who opened this connection, as the kernel says it — not as the caller says it.
///
/// Every other fact a service has about its caller arrives inside the request, which means the
/// caller chose it. These three did not: `SO_PEERCRED` is filled in by the kernel at `connect`
/// time from the peer's own process, and nothing the peer writes on the socket can change them.
/// That is the whole reason this exists (issue #43) — an approval card that names whoever is
/// asking was naming a string the asker supplied.
///
/// Best-effort and deliberately optional: a TCP connection on the Windows dev path has no peer
/// process, and a peer that exits between `accept` and the `getsockopt` still leaves a pid that
/// no longer resolves. A service that cannot learn this must still work; none of them may
/// *refuse* on it, because policy belongs to the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerCred {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

/// Trait for service method dispatch. Implement this in each service.
pub trait ServiceHandler: Send + Sync + 'static {
    /// Service identifier (e.g. "weather", "notes").
    fn service_id(&self) -> &str;

    /// Dispatch an RPC method call. Returns the result as JSON value.
    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, yantrik_ipc_contracts::email::ServiceError>;

    /// The same dispatch, told who is on the other end of the socket.
    ///
    /// Defaulted so that every existing `ServiceHandler` — fourteen apps and every service —
    /// compiles and behaves exactly as before: the default throws the credentials away and calls
    /// [`ServiceHandler::handle`]. Only a handler that has something honest to do with the
    /// caller's identity overrides it.
    fn handle_from(
        &self,
        method: &str,
        params: serde_json::Value,
        peer: Option<PeerCred>,
    ) -> Result<serde_json::Value, yantrik_ipc_contracts::email::ServiceError> {
        let _ = peer;
        self.handle(method, params)
    }
}

/// JSON-RPC server.
pub struct RpcServer {
    address: String,
    /// Whether this server bound the socket at `address`. Only then is the file its to remove on
    /// drop: a server that was refused the name (see [`crate::owner::claim`]) must not delete the
    /// socket of the process that owns it on the way out, which would leave that process listening
    /// on a file nobody can find.
    #[cfg_attr(not(unix), allow(dead_code))]
    bound: bool,
}

impl RpcServer {
    /// Create a new server. On Linux, `address` is a Unix socket path.
    /// On Windows, `address` is a TCP address (e.g. "127.0.0.1:9500").
    pub fn new(address: &str) -> Self {
        Self {
            address: address.to_string(),
            bound: false,
        }
    }

    /// Default address for a service.
    ///
    /// Prefers the per-user runtime directory. `/run` is root-owned, so a
    /// desktop session running services as the logged-in user cannot create
    /// `/run/yantrik` — every service then failed to bind with a bare ENOENT.
    /// `$XDG_RUNTIME_DIR` is the standard location for exactly this, and is
    /// already per-user, tmpfs-backed, and cleaned up on logout.
    ///
    /// Falls back to `/run/yantrik` for the system-service case (running as
    /// root, no session, no XDG_RUNTIME_DIR).
    ///
    /// Client and server both call this, so they cannot disagree.
    #[cfg(unix)]
    pub fn default_address(service_id: &str) -> String {
        format!("{}/{}.sock", socket_dir().display(), service_id)
    }

    #[cfg(windows)]
    pub fn default_address(service_id: &str) -> String {
        // Map service names to dev ports
        let port = match service_id {
            "weather" => 9501,
            "system-monitor" => 9502,
            "network" => 9503,
            "music" => 9504,
            "email" => 9505,
            "notes" => 9506,
            "calendar" => 9507,
            "notifications" => 9508,
            "companion" => 9509,
            _ => 9500,
        };
        format!("127.0.0.1:{}", port)
    }

    /// Run the server, dispatching requests to the handler.
    pub async fn serve(self, handler: Arc<dyn ServiceHandler>) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            self.serve_unix(handler).await
        }
        #[cfg(windows)]
        {
            self.serve_tcp(handler).await
        }
    }

    #[cfg(unix)]
    async fn serve_unix(mut self, handler: Arc<dyn ServiceHandler>) -> std::io::Result<()> {
        use tokio::net::UnixListener;
        use std::path::Path;

        let address = self.address.clone();
        let path = Path::new(&address);
        // The name is owned by whoever is answering on it. This used to unlink whatever was at
        // the path, so a second copy of an app or a service took the name from the running one,
        // which went on listening on a file nobody could reach. A live socket is refused with a
        // sentence naming it; only one nobody is listening on — a crashed run's — is removed.
        // Blocking, and bounded by `owner::CLAIM_PING`: it happens once, before anything is served.
        crate::owner::claim(path)?;
        // Do NOT swallow this. When it failed silently (`/run` is root-owned),
        // the real cause — a permission error — surfaced later as a bare
        // ENOENT from bind(), which reads like a missing binary and sent
        // debugging in the wrong direction entirely.
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Err(std::io::Error::new(
                    e.kind(),
                    format!(
                        "cannot create socket directory {}: {e} \
                         (set XDG_RUNTIME_DIR to a writable per-user path)",
                        parent.display()
                    ),
                ));
            }
        }

        let listener = UnixListener::bind(&self.address).map_err(|e| {
            std::io::Error::new(e.kind(), format!("cannot bind {}: {e}", self.address))
        })?;
        self.bound = true;
        private_socket_file(path);
        tracing::info!(socket = %self.address, service = handler.service_id(), "RPC server listening (UDS)");

        loop {
            let (stream, _) = listener.accept().await?;
            // Read at accept, not when somebody asks. The peer of these sockets is routinely a
            // short-lived process — `yos` runs one JSON-RPC call and exits — so by the time a
            // handler wants to know who called, the pid may already be gone or, worse, reused.
            // Asking here narrows that window to the connection's own lifetime.
            let peer = stream
                .peer_cred()
                .ok()
                .map(|c| PeerCred { pid: c.pid().unwrap_or(0), uid: c.uid(), gid: c.gid() });
            let handler = handler.clone();
            tokio::spawn(async move {
                let (reader, writer) = stream.into_split();
                handle_connection(BufReader::new(reader), writer, &handler, peer).await;
            });
        }
    }

    #[cfg(windows)]
    async fn serve_tcp(self, handler: Arc<dyn ServiceHandler>) -> std::io::Result<()> {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind(&self.address).await?;
        tracing::info!(addr = %self.address, service = handler.service_id(), "RPC server listening (TCP dev)");

        loop {
            let (stream, _) = listener.accept().await?;
            let handler = handler.clone();
            tokio::spawn(async move {
                let (reader, writer) = stream.into_split();
                // TCP has no peer process to ask about. This path is the Windows dev loop only.
                handle_connection(BufReader::new(reader), writer, &handler, None).await;
            });
        }
    }
}

async fn handle_connection<R, W>(
    reader: BufReader<R>,
    mut writer: W,
    handler: &Arc<dyn ServiceHandler>,
    peer: Option<PeerCred>,
) where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<RpcRequest>(&line) {
            Ok(req) => {
                tracing::debug!(method = %req.method, peer = ?peer, "RPC request");
                // Handlers are synchronous and may hold a call for as long as it takes — the
                // companion's `ask` runs for ninety seconds. Run on a runtime worker, that call
                // also held the I/O driver whenever its worker was the last to poll it: the other
                // workers slept on their condvars, nothing polled the socket, and the next
                // connection was not even accepted until the slow call ended. The blocking pool
                // is where a call that blocks belongs.
                let handler = handler.clone();
                let id = req.id.clone();
                match tokio::task::spawn_blocking(move || dispatch(&handler, req, peer)).await {
                    Ok(response) => response,
                    Err(e) => {
                        tracing::error!(error = %e, "RPC handler panicked");
                        RpcResponse::error(id, RPC_INTERNAL_ERROR, "the service failed while answering".into())
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to parse RPC request");
                RpcResponse::error(
                    serde_json::Value::Null,
                    RPC_PARSE_ERROR,
                    format!("Parse error: {}", e),
                )
            }
        };

        let mut resp_json = serde_json::to_string(&response).unwrap_or_default();
        resp_json.push('\n');
        if writer.write_all(resp_json.as_bytes()).await.is_err() {
            break;
        }
    }
}

fn dispatch(
    handler: &Arc<dyn ServiceHandler>,
    req: RpcRequest,
    peer: Option<PeerCred>,
) -> RpcResponse {
    match req.method.as_str() {
        "rpc.ping" => {
            return RpcResponse::success(req.id, serde_json::json!("pong"));
        }
        "rpc.service_id" => {
            return RpcResponse::success(req.id, serde_json::json!(handler.service_id()));
        }
        _ => {}
    }

    match handler.handle_from(&req.method, req.params, peer) {
        Ok(result) => RpcResponse::success(req.id, result),
        Err(e) => {
            if e.code == -1 {
                RpcResponse::error(req.id, RPC_METHOD_NOT_FOUND, e.message)
            } else {
                RpcResponse::error(req.id, e.code, e.message)
            }
        }
    }
}

#[cfg(unix)]
impl Drop for RpcServer {
    fn drop(&mut self) {
        if self.bound {
            let _ = std::fs::remove_file(&self.address);
        }
    }
}

#[cfg(all(test, unix))]
mod socket_dir_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn hardening_an_already_private_directory_changes_nothing() {
        // The property perception-service depends on: asking twice must be safe. The second ask
        // happens after a Landlock ruleset is in force, and a chmod at that point is denied — so
        // if this is not a no-op the service cannot bind the socket it already made room for.
        let dir = std::env::temp_dir().join(format!("yantrik-harden-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        harden(&dir).expect("first harden");
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);

        // Make it read-only so any *attempted* chmod would be visible as a change, then prove the
        // second call does not attempt one.
        harden(&dir).expect("second harden must be a no-op, not a second chmod");
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A socket this crate binds is not readable, writable or anything else to another user.
    ///
    /// Both halves matter and the second is the one that makes this change safe to ship: a unix
    /// socket needs *write* permission to `connect`, and 0600 keeps the owner's write bit, so the
    /// session's own clients — `yos`, the conformance suite, every app — connect exactly as
    /// before. A mode that locked out the owner would be indistinguishable from a dead service.
    #[test]
    fn a_bound_socket_is_private_and_its_owner_can_still_connect() {
        use std::os::unix::net::{UnixListener, UnixStream};

        let dir = std::env::temp_dir().join(format!("yantrik-sockmode-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("probe.sock");

        let listener = UnixListener::bind(&path).expect("bind");
        // What `serve_unix` does immediately after its own bind.
        private_socket_file(&path);

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "a freshly bound socket came out {mode:o}. Under the default umask bind(2) makes it \
             0755, and group/world have no business with a socket that drives this session's \
             network, notifications and files."
        );

        UnixStream::connect(&path).expect(
            "the owner of a 0600 socket must still be able to connect to it — connect(2) needs \
             write, and the owner has it",
        );
        drop(listener);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_world_readable_directory_is_still_tightened() {
        // The other half: the early return must not make `harden` stop hardening. /tmp is
        // world-writable and these sockets drive system-monitor, network and notifications.
        let dir = std::env::temp_dir().join(format!("yantrik-loose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut perms = std::fs::metadata(&dir).unwrap().permissions();
        perms.set_mode(0o777);
        std::fs::set_permissions(&dir, perms).unwrap();

        harden(&dir).expect("harden");
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700,
            "a world-writable socket directory must be tightened, not waved through"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn asking_twice_returns_the_same_directory() {
        assert_eq!(socket_dir(), socket_dir());
    }

    struct Named(&'static str);

    impl ServiceHandler for Named {
        fn service_id(&self) -> &str {
            self.0
        }
        fn handle(
            &self,
            method: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, yantrik_ipc_contracts::email::ServiceError> {
            Ok(serde_json::json!({ "answered_by": self.0, "method": method }))
        }
    }

    fn one_line(path: &std::path::Path, line: &str) -> serde_json::Value {
        use std::io::{BufRead, BufReader, Write};
        let mut stream = std::os::unix::net::UnixStream::connect(path).expect("connect");
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        stream.write_all(format!("{line}\n").as_bytes()).unwrap();
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).expect("one JSON object per line")
    }

    /// Owned names, end to end through `serve`: a second server on a live name is refused, says
    /// whose it is, and leaves the first one answering — including after the refused server is
    /// dropped, whose `Drop` used to delete the socket file it had never bound.
    #[test]
    fn a_second_server_on_a_live_name_is_refused_and_the_first_keeps_it() {
        let dir = std::env::temp_dir().join(format!("yantrik-owned-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app-first.sock");
        let address = path.to_string_lossy().to_string();

        let first = address.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let _ = rt.block_on(RpcServer::new(&first).serve(Arc::new(Named("app-first"))));
        });
        for _ in 0..200 {
            if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let err = rt
            .block_on(RpcServer::new(&address).serve(Arc::new(Named("app-second"))))
            .expect_err("the name is taken by a live server");
        assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
        assert!(err.to_string().contains("another instance owns") && err.to_string().contains("`app-first`"), "{err}");

        let reply = one_line(&path, r#"{"jsonrpc":"2.0","id":7,"method":"app.describe"}"#);
        assert_eq!(reply["result"]["answered_by"], "app-first", "{reply}");
        assert_eq!(reply["id"], 7);

        // Framing, pinned where the spec states it: a request with no `id` is not a request this
        // transport serves, and the answer says so as a parse error with a null id.
        let reply = one_line(&path, r#"{"jsonrpc":"2.0","method":"rpc.ping"}"#);
        assert_eq!(reply["error"]["code"], RPC_PARSE_ERROR, "{reply}");
        assert!(reply["id"].is_null());
        let reply = one_line(&path, r#"{"jsonrpc":"2.0","id":"a","method":"rpc.ping"}"#);
        assert_eq!(reply["result"], "pong");
        assert_eq!(reply["id"], "a", "the id comes back as it was sent, string or number");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Holds `slow` until released, panics on `boom`, and answers everything else at once.
    struct Held {
        entered: std::sync::mpsc::SyncSender<()>,
        release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl ServiceHandler for Held {
        fn service_id(&self) -> &str {
            "held"
        }
        fn handle(
            &self,
            method: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, yantrik_ipc_contracts::email::ServiceError> {
            match method {
                "slow" => {
                    let _ = self.entered.send(());
                    let _ = self.release.lock().unwrap().recv_timeout(std::time::Duration::from_secs(30));
                }
                "boom" => panic!("a handler that fails outright"),
                _ => {}
            }
            Ok(serde_json::json!({ "method": method }))
        }
    }

    /// A call held in its handler must not hold up anyone else's. On a current-thread runtime the
    /// handler used to run on the one thread the accept loop needs, so this was certain to fail;
    /// on the companion's four-worker runtime it failed only when the held call's worker was the
    /// last to poll the socket, which made it a CI flake (`a_slow_call_in_flight_…`, twice on
    /// main) rather than the ninety-second freeze it was on a person's desktop.
    #[test]
    fn a_call_held_in_its_handler_does_not_hold_up_another() {
        let dir = std::env::temp_dir().join(format!("yantrik-held-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("held.sock");
        let address = path.to_string_lossy().to_string();

        let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let handler = Arc::new(Held { entered: entered_tx, release: std::sync::Mutex::new(release_rx) });
        let serving = address.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let _ = rt.block_on(RpcServer::new(&serving).serve(handler));
        });
        for _ in 0..200 {
            if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let slow_path = path.clone();
        let slow = std::thread::spawn(move || one_line(&slow_path, r#"{"jsonrpc":"2.0","id":1,"method":"slow"}"#));
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the slow call reached its handler");

        // `one_line` waits five seconds; the slow call is held for thirty.
        let fast = one_line(&path, r#"{"jsonrpc":"2.0","id":2,"method":"fast"}"#);
        assert_eq!(fast["result"]["method"], "fast", "{fast}");

        // A handler that panics costs its caller an answer, not the server: the caller hears an
        // internal error under its own id, and the next caller is served as usual.
        let boom = one_line(&path, r#"{"jsonrpc":"2.0","id":3,"method":"boom"}"#);
        assert_eq!(boom["error"]["code"], RPC_INTERNAL_ERROR, "{boom}");
        assert_eq!(boom["id"], 3);
        let after = one_line(&path, r#"{"jsonrpc":"2.0","id":4,"method":"fast"}"#);
        assert_eq!(after["result"]["method"], "fast", "{after}");

        let _ = release_tx.send(());
        let slow = slow.join().expect("the slow caller");
        assert_eq!(slow["result"]["method"], "slow", "{slow}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
