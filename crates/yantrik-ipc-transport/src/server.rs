//! JSON-RPC server — Unix domain sockets (Linux) or TCP localhost (Windows dev).

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::protocol::{RpcRequest, RpcResponse, RPC_METHOD_NOT_FOUND, RPC_PARSE_ERROR};

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

    for dir in &candidates {
        if std::fs::create_dir_all(dir).is_ok() && harden(dir).is_ok() {
            return dir.clone();
        }
    }
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
    let mut perms = std::fs::metadata(dir)?.permissions();
    perms.set_mode(0o700);
    std::fs::set_permissions(dir, perms)
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
}

/// JSON-RPC server.
pub struct RpcServer {
    address: String,
}

impl RpcServer {
    /// Create a new server. On Linux, `address` is a Unix socket path.
    /// On Windows, `address` is a TCP address (e.g. "127.0.0.1:9500").
    pub fn new(address: &str) -> Self {
        Self {
            address: address.to_string(),
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
    async fn serve_unix(self, handler: Arc<dyn ServiceHandler>) -> std::io::Result<()> {
        use tokio::net::UnixListener;
        use std::path::Path;

        let path = Path::new(&self.address);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
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
        tracing::info!(socket = %self.address, service = handler.service_id(), "RPC server listening (UDS)");

        loop {
            let (stream, _) = listener.accept().await?;
            let handler = handler.clone();
            tokio::spawn(async move {
                let (reader, writer) = stream.into_split();
                handle_connection(BufReader::new(reader), writer, &handler).await;
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
                handle_connection(BufReader::new(reader), writer, &handler).await;
            });
        }
    }
}

async fn handle_connection<R, W>(reader: BufReader<R>, mut writer: W, handler: &Arc<dyn ServiceHandler>)
where
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
                tracing::debug!(method = %req.method, "RPC request");
                dispatch(handler, req)
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

fn dispatch(handler: &Arc<dyn ServiceHandler>, req: RpcRequest) -> RpcResponse {
    match req.method.as_str() {
        "rpc.ping" => {
            return RpcResponse::success(req.id, serde_json::json!("pong"));
        }
        "rpc.service_id" => {
            return RpcResponse::success(req.id, serde_json::json!(handler.service_id()));
        }
        _ => {}
    }

    match handler.handle(&req.method, req.params) {
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
        let _ = std::fs::remove_file(&self.address);
    }
}
