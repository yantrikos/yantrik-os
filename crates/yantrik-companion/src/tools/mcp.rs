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
        // Another party's tool, called in the person's name and possibly acting on an account or
        // a service of theirs: asked about first, with the server, the tool and its arguments
        // whole (#1054). Whether it only reads is the server's word, which is not enough to skip.
        let to = format!("MCP server {} ({})", self.server_id, self.mcp_tool_name);
        let what = args.to_string();
        yantrik_companion_tools::outward::acting(
            &yantrik_companion_tools::outward::OutwardAct { tool: self.tool_name, to: &to, what: &what, more: "" },
            || self.call(args),
        )
    }
}

impl McpProxyTool {
    /// The call itself, after the request was scanned and the person said yes.
    fn call(&self, args: &Value) -> String {
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

// Re-export McpServerEntry from core config
pub use yantrik_companion_core::config::McpServerEntry;

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

#[cfg(all(test, unix))]
mod tests {
    //! An MCP tool is another party's tool called in the person's name (#1054): it asks first,
    //! with the server, the tool and its whole arguments, and a no calls nothing.
    use super::*;
    use yantrik_companion_tools::outward_testing::{actions, first_card, installed, shown, with_ctx};

    /// A server that answers the handshake and every `tools/call` with "done", and leaves a file
    /// behind when a call reaches it.
    fn server(marker: &std::path::Path) -> McpConnection {
        let script = r#"
            while IFS= read -r line; do
              id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
              case "$line" in
                *'"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"serverInfo":{"name":"t"}}}\n' "$id" ;;
                *'"tools/call"'*) : > "$MARK"; printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"done"}]}}\n' "$id" ;;
              esac
            done"#;
        let env = std::collections::HashMap::from([("MARK".to_string(), marker.to_string_lossy().to_string())]);
        McpConnection::start("sh", &["-c".to_string(), script.to_string()], &env, None).expect("the fake server starts")
    }

    fn proxy(marker: &std::path::Path) -> McpProxyTool {
        McpProxyTool {
            tool_name: "mcp__fake_post",
            mcp_tool_name: "post".to_string(),
            server_id: "fake".to_string(),
            tool_description: "posts",
            input_schema: serde_json::json!({}),
            connection: std::sync::Arc::new(Mutex::new(server(marker))),
        }
    }

    fn marker(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("yantrik-mcp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn a_call_raises_a_card_with_the_server_the_tool_and_the_arguments_and_a_no_calls_nothing() {
        let (_shell, heard) = installed("mcp-no", "pending", "denied");
        let mark = marker("no");
        let said = with_ctx(|ctx| proxy(&mark).execute(ctx, &serde_json::json!({ "channel": "#general", "text": "hi; everyone" })));

        assert!(said.contains("said no") && said.contains("Nothing was done"), "{said}");
        assert!(!mark.exists(), "the server was called after a no");
        let card = first_card(&heard);
        assert_eq!(card["args_json"]["tool"], "mcp__fake_post");
        assert_eq!(shown(&card, "to"), "MCP server fake (post)");
        assert_eq!(shown(&card, "what"), r##"{"channel":"#general","text":"hi; everyone"}"##);
        assert!(!actions(&heard).contains(&"consume_approval".to_string()));
    }

    #[test]
    fn an_allow_is_spent_once_and_the_call_is_made_once() {
        let (_shell, heard) = installed("mcp-yes", "pending", "granted");
        let mark = marker("yes");
        let said = with_ctx(|ctx| proxy(&mark).execute(ctx, &serde_json::json!({ "text": "hi" })));

        assert_eq!(said, "done");
        assert!(mark.exists());
        assert_eq!(actions(&heard).iter().filter(|a| *a == "consume_approval").count(), 1);
        let _ = std::fs::remove_file(&mark);
    }
}
