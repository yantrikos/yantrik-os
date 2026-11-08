//! MCP (Model Context Protocol) stdio client.
//!
//! Spawns an MCP server process, communicates via JSON-RPC 2.0 over stdin/stdout.
//! Supports `initialize`, `tools/list`, and `tools/call`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// An active MCP server connection.
pub struct McpConnection {
    child: Child,
    request_id: AtomicU64,
}

/// A tool exposed by an MCP server.
#[derive(Debug, Clone)]
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Result from calling an MCP tool.
#[derive(Debug)]
pub struct McpToolResult {
    pub content: String,
    pub is_error: bool,
}

impl McpConnection {
    /// Spawn an MCP server process and perform the initialize handshake.
    pub fn start(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        cwd: Option<&str>,
    ) -> Result<Self, String> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .envs(env);

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let child = cmd.spawn().map_err(|e| {
            format!("Failed to spawn MCP server '{command}': {e}")
        })?;

        let mut conn = McpConnection {
            child,
            request_id: AtomicU64::new(1),
        };

        // Send initialize request
        let init_result = conn.send_request("initialize", serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "yantrik",
                "version": "0.1.0"
            }
        }))?;

        tracing::info!(
            server_name = init_result.get("serverInfo")
                .and_then(|s| s.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("unknown"),
            "MCP server initialized"
        );

        // Send initialized notification (no response expected)
        conn.send_notification("notifications/initialized", serde_json::json!({}))?;

        Ok(conn)
    }

    /// List tools available from the MCP server.
    pub fn list_tools(&mut self) -> Result<Vec<McpToolDef>, String> {
        let result = self.send_request("tools/list", serde_json::json!({}))?;

        let tools = result.get("tools")
            .and_then(|t| t.as_array())
            .ok_or("MCP server returned no tools array")?;

        let mut defs = Vec::new();
        for tool in tools {
            let name = tool.get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_string();
            let description = tool.get("description")
                .and_then(|d| d.as_str())
                .unwrap_or_default()
                .to_string();
            let input_schema = tool.get("inputSchema")
                .cloned()
                .unwrap_or(serde_json::json!({"type": "object", "properties": {}}));

            defs.push(McpToolDef { name, description, input_schema });
        }

        Ok(defs)
    }

    /// Call a tool on the MCP server.
    pub fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<McpToolResult, String> {
        let result = self.send_request("tools/call", serde_json::json!({
            "name": tool_name,
            "arguments": arguments
        }))?;

        let is_error = result.get("isError")
            .and_then(|e| e.as_bool())
            .unwrap_or(false);

        // Extract text content from the content array
        let content = if let Some(content_arr) = result.get("content").and_then(|c| c.as_array()) {
            let mut text_parts = Vec::new();
            for item in content_arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    text_parts.push(text.to_string());
                }
            }
            text_parts.join("\n")
        } else {
            result.to_string()
        };

        Ok(McpToolResult { content, is_error })
    }

    /// Send a JSON-RPC request and wait for the response.
    fn send_request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let id = self.request_id.fetch_add(1, Ordering::Relaxed);

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        self.write_message(&request)?;
        self.read_response(id)
    }

    /// Send a JSON-RPC notification (no response expected).
    fn send_notification(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), String> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        self.write_message(&notification)
    }

    /// Write a JSON-RPC message to the server's stdin.
    fn write_message(&mut self, msg: &serde_json::Value) -> Result<(), String> {
        let stdin = self.child.stdin.as_mut()
            .ok_or("MCP server stdin not available")?;

        let json_str = serde_json::to_string(msg)
            .map_err(|e| format!("JSON serialize error: {e}"))?;

        stdin.write_all(json_str.as_bytes())
            .map_err(|e| format!("Failed to write to MCP server: {e}"))?;
        stdin.write_all(b"\n")
            .map_err(|e| format!("Failed to write newline: {e}"))?;
        stdin.flush()
            .map_err(|e| format!("Failed to flush MCP server stdin: {e}"))?;

        Ok(())
    }

    /// Read a JSON-RPC response matching the given request ID.
    /// Skips notifications and other messages until the matching response is found.
    fn read_response(&mut self, expected_id: u64) -> Result<serde_json::Value, String> {
        let stdout = self.child.stdout.as_mut()
            .ok_or("MCP server stdout not available")?;

        let mut reader = BufReader::new(stdout);
        let mut line = String::new();

        // Read lines until we get the matching response (timeout after 30s)
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);

        loop {
            if std::time::Instant::now() > deadline {
                return Err("MCP server response timeout (30s)".to_string());
            }

            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => return Err("MCP server closed stdout".to_string()),
                Ok(_) => {}
                Err(e) => return Err(format!("Failed to read from MCP server: {e}")),
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let msg: serde_json::Value = serde_json::from_str(trimmed)
                .map_err(|e| format!("Invalid JSON from MCP server: {e} — line: {trimmed}"))?;

            // Check if this is our response
            if let Some(id) = msg.get("id").and_then(|i| i.as_u64()) {
                if id == expected_id {
                    // Check for error
                    if let Some(error) = msg.get("error") {
                        let code = error.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
                        let message = error.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error");
                        return Err(format!("MCP error {code}: {message}"));
                    }
                    return msg.get("result").cloned()
                        .ok_or_else(|| "MCP response missing 'result' field".to_string());
                }
            }

            // Skip notifications and non-matching responses
            tracing::trace!(msg = trimmed, "Skipping non-matching MCP message");
        }
    }

    /// Check if the server process is still running.
    pub fn is_alive(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => true,
            _ => false,
        }
    }
}

impl Drop for McpConnection {
    fn drop(&mut self) {
        // Try to kill the server process gracefully
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
