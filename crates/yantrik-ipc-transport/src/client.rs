//! JSON-RPC client — connects to services via Unix sockets (Linux) or TCP (Windows dev).

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::protocol::{RpcRequest, RpcResponse, RpcError};
use crate::server::RpcServer;

/// RPC client connected to a service.
pub struct RpcClient {
    address: String,
    next_id: std::sync::atomic::AtomicU64,
}

impl RpcClient {
    /// Create a new client targeting the given address.
    pub fn new(address: &str) -> Self {
        Self {
            address: address.to_string(),
            next_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Connect to the default address for a service.
    pub fn for_service(service_id: &str) -> Self {
        Self::new(&RpcServer::default_address(service_id))
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// Call a method and return the result.
    pub async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let id = self.next_id();
        let req = RpcRequest::new(method, params, id);

        let mut req_json = serde_json::to_string(&req).map_err(|e| RpcError {
            code: -32000,
            message: format!("Serialize error: {}", e),
            data: None,
        })?;
        req_json.push('\n');

        let resp_line = self.send_receive(&req_json).await?;

        let resp: RpcResponse = serde_json::from_str(&resp_line).map_err(|e| RpcError {
            code: -32000,
            message: format!("Response parse error: {}", e),
            data: None,
        })?;

        if let Some(err) = resp.error {
            Err(err)
        } else {
            Ok(resp.result.unwrap_or(serde_json::Value::Null))
        }
    }

    #[cfg(unix)]
    async fn send_receive(&self, req_json: &str) -> Result<String, RpcError> {
        use tokio::net::UnixStream;

        let stream = UnixStream::connect(&self.address).await.map_err(|e| RpcError {
            code: -32000,
            message: format!("Connection failed ({}): {}", self.address, e),
            data: None,
        })?;
        let (reader, mut writer) = stream.into_split();

        writer.write_all(req_json.as_bytes()).await.map_err(|e| RpcError {
            code: -32000,
            message: format!("Write error: {}", e),
            data: None,
        })?;

        let mut lines = BufReader::new(reader).lines();
        lines.next_line().await.map_err(|e| RpcError {
            code: -32000,
            message: format!("Read error: {}", e),
            data: None,
        })?.ok_or_else(|| RpcError {
            code: -32000,
            message: "Connection closed before response".into(),
            data: None,
        })
    }

    #[cfg(windows)]
    async fn send_receive(&self, req_json: &str) -> Result<String, RpcError> {
        use tokio::net::TcpStream;

        let stream = TcpStream::connect(&self.address).await.map_err(|e| RpcError {
            code: -32000,
            message: format!("Connection failed ({}): {}", self.address, e),
            data: None,
        })?;
        let (reader, mut writer) = stream.into_split();

        writer.write_all(req_json.as_bytes()).await.map_err(|e| RpcError {
            code: -32000,
            message: format!("Write error: {}", e),
            data: None,
        })?;

        let mut lines = BufReader::new(reader).lines();
        lines.next_line().await.map_err(|e| RpcError {
            code: -32000,
            message: format!("Read error: {}", e),
            data: None,
        })?.ok_or_else(|| RpcError {
            code: -32000,
            message: "Connection closed before response".into(),
            data: None,
        })
    }

    /// Convenience: call with typed params and typed result.
    pub async fn call_typed<P: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: &P,
    ) -> Result<R, RpcError> {
        let params_value = serde_json::to_value(params).map_err(|e| RpcError {
            code: -32000,
            message: format!("Param serialize error: {}", e),
            data: None,
        })?;
        let result = self.call(method, params_value).await?;
        serde_json::from_value(result).map_err(|e| RpcError {
            code: -32000,
            message: format!("Result deserialize error: {}", e),
            data: None,
        })
    }

    /// Check if the service is reachable.
    pub async fn ping(&self) -> bool {
        self.call("rpc.ping", serde_json::Value::Null).await.is_ok()
    }
}
