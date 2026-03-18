//! Synchronous JSON-RPC client — for use in non-async contexts (UI wire modules).
//!
//! Uses std::net TCP / Unix streams. No tokio dependency.

use std::io::{BufRead, BufReader, Write};

use crate::protocol::{RpcError, RpcRequest, RpcResponse};
use crate::server::RpcServer;

/// Blocking RPC client for calling services from synchronous code.
pub struct SyncRpcClient {
    address: String,
    next_id: std::sync::atomic::AtomicU64,
}

impl SyncRpcClient {
    /// Create a client targeting the given address.
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
        self.next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// Call a method synchronously and return the result.
    pub fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let id = self.next_id();
        let req = RpcRequest::new(method, params, id);

        let mut req_json = serde_json::to_string(&req).map_err(|e| RpcError {
            code: -32000,
            message: format!("Serialize error: {e}"),
            data: None,
        })?;
        req_json.push('\n');

        let resp_line = self.send_receive(&req_json)?;

        let resp: RpcResponse = serde_json::from_str(&resp_line).map_err(|e| RpcError {
            code: -32000,
            message: format!("Response parse error: {e}"),
            data: None,
        })?;

        if let Some(err) = resp.error {
            Err(err)
        } else {
            Ok(resp.result.unwrap_or(serde_json::Value::Null))
        }
    }

    /// Convenience: call with typed params and typed result.
    pub fn call_typed<P: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: &P,
    ) -> Result<R, RpcError> {
        let params_value = serde_json::to_value(params).map_err(|e| RpcError {
            code: -32000,
            message: format!("Param serialize error: {e}"),
            data: None,
        })?;
        let result = self.call(method, params_value)?;
        serde_json::from_value(result).map_err(|e| RpcError {
            code: -32000,
            message: format!("Result deserialize error: {e}"),
            data: None,
        })
    }

    /// Check if the service is reachable.
    pub fn ping(&self) -> bool {
        self.call("rpc.ping", serde_json::Value::Null).is_ok()
    }

    #[cfg(unix)]
    fn send_receive(&self, req_json: &str) -> Result<String, RpcError> {
        use std::os::unix::net::UnixStream;

        let mut stream = UnixStream::connect(&self.address).map_err(|e| RpcError {
            code: -32000,
            message: format!("Connection failed ({}): {e}", self.address),
            data: None,
        })?;

        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(30)))
            .ok();

        stream.write_all(req_json.as_bytes()).map_err(|e| RpcError {
            code: -32000,
            message: format!("Write error: {e}"),
            data: None,
        })?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| RpcError {
            code: -32000,
            message: format!("Read error: {e}"),
            data: None,
        })?;

        if line.is_empty() {
            return Err(RpcError {
                code: -32000,
                message: "Connection closed before response".into(),
                data: None,
            });
        }

        Ok(line)
    }

    #[cfg(windows)]
    fn send_receive(&self, req_json: &str) -> Result<String, RpcError> {
        use std::net::TcpStream;

        let mut stream = TcpStream::connect(&self.address).map_err(|e| RpcError {
            code: -32000,
            message: format!("Connection failed ({}): {e}", self.address),
            data: None,
        })?;

        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(30)))
            .ok();

        stream.write_all(req_json.as_bytes()).map_err(|e| RpcError {
            code: -32000,
            message: format!("Write error: {e}"),
            data: None,
        })?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| RpcError {
            code: -32000,
            message: format!("Read error: {e}"),
            data: None,
        })?;

        if line.is_empty() {
            return Err(RpcError {
                code: -32000,
                message: "Connection closed before response".into(),
                data: None,
            });
        }

        Ok(line)
    }
}
