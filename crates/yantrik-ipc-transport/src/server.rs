//! JSON-RPC server — Unix domain sockets (Linux) or TCP localhost (Windows dev).

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::protocol::{RpcRequest, RpcResponse, RPC_METHOD_NOT_FOUND, RPC_PARSE_ERROR};

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
    #[cfg(unix)]
    pub fn default_address(service_id: &str) -> String {
        format!("/run/yantrik/{}.sock", service_id)
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
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let listener = UnixListener::bind(&self.address)?;
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
