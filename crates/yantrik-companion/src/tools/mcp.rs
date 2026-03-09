//! MCP tool bridge — connects MCP servers to the Yantrik tool registry.
//!
//! For each configured MCP server:
//! 1. Spawns the server process
//! 2. Discovers available tools via `tools/list`
//! 3. Creates Tool trait implementations that proxy calls to the MCP server
//! 4. Registers them into the ToolRegistry
//!
//! **Security**: Every request and response is scanned by McpSecurityScanner.
//! Tools are named `mcp__{server}_{tool}` to avoid collisions with built-in tools.

use std::sync::Mutex;
use serde_json::Value;
use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};
use crate::mcp_client::McpConnection;
use crate::mcp_security::{McpSecurityScanner, McpTrustLevel};

/// Shared security scanner + MCP connections (thread-safe).
static MCP_SCANNER: std::sync::LazyLock<Mutex<McpSecurityScanner>> =
    std::sync::LazyLock::new(|| Mutex::new(McpSecurityScanner::new()));

/// A tool that proxies calls to an MCP server with security scanning.
struct McpProxyTool {
    /// Tool name in registry: mcp__{server}_{tool}
    tool_name: &'static str,
    /// Original tool name on the MCP server
    mcp_tool_name: String,
    /// Server identifier (for logging and security tracking)
    server_id: String,
    /// Tool description from MCP server
    tool_description: &'static str,
    /// Input schema from MCP server
    input_schema: Value,
    /// Shared connection to the MCP server (Mutex for interior mutability)
    connection: std::sync::Arc<Mutex<McpConnection>>,
}

impl Tool for McpProxyTool {
    fn name(&self) -> &'static str { self.tool_name }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "mcp" }

    fn definition(&self) -> Value {
        let params = if self.input_schema.get("type").is_some() {
            self.input_schema.clone()
        } else {
            serde_json::json!({
                "type": "object",
                "properties": self.input_schema,
            })
        };

        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.tool_name,
                "description": self.tool_description,
                "parameters": params
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &Value) -> String {
        // ── Security: scan request BEFORE sending to MCP server ──
        {
            let scanner = MCP_SCANNER.lock().unwrap();
            let scan = scanner.scan_request(&self.server_id, &self.mcp_tool_name, args);
            if !scan.safe {
                tracing::warn!(
                    server = %self.server_id,
                    tool = %self.mcp_tool_name,
                    reason = %scan.reason,
                    "MCP request blocked by security scanner"
                );
                return format!("Security: {}", scan.reason);
            }
        }

        // ── Execute the MCP tool call ──
        let mut conn = match self.connection.lock() {
            Ok(c) => c,
            Err(e) => return format!("Error: MCP connection lock poisoned: {e}"),
        };

        if !conn.is_alive() {
            return format!("Error: MCP server '{}' is no longer running", self.server_id);
        }

        let result = match conn.call_tool(&self.mcp_tool_name, args) {
            Ok(result) => {
                if result.is_error {
                    format!("MCP error: {}", result.content)
                } else {
                    result.content
                }
            }
            Err(e) => return format!("Error calling MCP tool '{}': {e}", self.mcp_tool_name),
        };

        // Drop connection lock before acquiring scanner lock
        drop(conn);

        // ── Security: scan response BEFORE returning to LLM ──
        {
            let mut scanner = MCP_SCANNER.lock().unwrap();
            let scan = scanner.scan_response(&self.server_id, &self.mcp_tool_name, &result);
            if !scan.safe {
                tracing::warn!(
                    server = %self.server_id,
                    tool = %self.mcp_tool_name,
                    reason = %scan.reason,
                    "MCP response blocked by security scanner"
                );
                return format!("Security: {}", scan.reason);
            }
            // Return the sanitized version
            scan.sanitized
        }
    }
}

/// Configuration for an MCP server to connect to.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct McpServerEntry {
    /// Unique server identifier (used in tool name prefix).
    pub id: String,
    /// Command to start the server.
    pub command: String,
    /// Arguments for the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// Working directory.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Trust level override (default: "untrusted").
    /// Options: "untrusted", "approved", "trusted"
    #[serde(default = "default_trust")]
    pub trust: String,
}

fn default_trust() -> String { "untrusted".to_string() }

/// Connect to configured MCP servers and register their tools.
///
/// Each server's tools are prefixed with `mcp__{server_id}_` to avoid collisions.
/// Failed connections are logged and skipped (non-fatal).
pub fn register_mcp_servers(reg: &mut ToolRegistry, servers: &[McpServerEntry]) {
    for server in servers {
        // Register security policy for this server
        let trust_level = match server.trust.as_str() {
            "trusted" => McpTrustLevel::Trusted,
            "approved" => McpTrustLevel::Approved,
            _ => McpTrustLevel::Untrusted,
        };

        if let Ok(mut scanner) = MCP_SCANNER.lock() {
            scanner.register_server(&server.id, trust_level);
        }

        match connect_and_register(reg, server) {
            Ok(count) => {
                tracing::info!(
                    server = %server.id,
                    tools = count,
                    trust = %server.trust,
                    "MCP server connected"
                );
            }
            Err(e) => {
                tracing::warn!(
                    server = %server.id,
                    error = %e,
                    "Failed to connect MCP server — skipping"
                );
            }
        }
    }
}

fn connect_and_register(
    reg: &mut ToolRegistry,
    server: &McpServerEntry,
) -> Result<usize, String> {
    let mut conn = McpConnection::start(
        &server.command,
        &server.args,
        &server.env,
        server.cwd.as_deref(),
    )?;

    let tools = conn.list_tools()?;
    let tool_count = tools.len();

    if tools.is_empty() {
        return Ok(0);
    }

    let shared_conn = std::sync::Arc::new(Mutex::new(conn));

    for tool_def in tools {
        let prefixed_name = format!("mcp__{}_{}", server.id, tool_def.name);
        let leaked_name: &'static str = Box::leak(prefixed_name.into_boxed_str());
        let leaked_desc: &'static str = Box::leak(tool_def.description.clone().into_boxed_str());

        let proxy = McpProxyTool {
            tool_name: leaked_name,
            mcp_tool_name: tool_def.name,
            server_id: server.id.clone(),
            tool_description: leaked_desc,
            input_schema: tool_def.input_schema,
            connection: shared_conn.clone(),
        };

        reg.register(Box::new(proxy));
    }

    Ok(tool_count)
}

/// Get the security scanner for external use (e.g., follow-up action blocking).
pub fn security_scanner() -> &'static Mutex<McpSecurityScanner> {
    &MCP_SCANNER
}

/// List active MCP server tools (for diagnostics / discover_tools).
pub fn list_mcp_tools_summary(reg: &ToolRegistry, max_permission: PermissionLevel) -> Vec<String> {
    reg.list_metadata(max_permission)
        .iter()
        .filter(|m| m.category == "mcp")
        .map(|m| format!("{}: {}", m.name, m.description))
        .collect()
}
