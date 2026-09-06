//! Reach the companion from an app.
//!
//! The LLM, the memory and the bond live in the shell process. An app talks to them the same way
//! it talks to any service — over the socket bus — so an AI action is a normal RPC call and not
//! a stub.
//!
//! Every call answers `Err` when the shell is not running, which is a state an app must handle
//! rather than hide: these binaries are meant to run on their own too.

use std::time::Duration;

use yantrik_ipc_transport::SyncRpcClient;

/// How long to wait for an answer. A local model on a cold cache is slow, and a caller that
/// gives up early leaves the user with nothing.
const ASK_TIMEOUT: Duration = Duration::from_secs(90);

fn client() -> SyncRpcClient {
    SyncRpcClient::for_service("companion").with_timeout(ASK_TIMEOUT)
}

/// Ask the companion something and get the finished answer.
pub fn ask(prompt: &str) -> Result<String, String> {
    let response = client()
        .call(
            "companion.ask",
            serde_json::json!({ "prompt": prompt, "timeout_ms": ASK_TIMEOUT.as_millis() as u64 }),
        )
        .map_err(|e| format!("[{}] {}", e.code, e.message))?;
    response
        .get("text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "companion returned no text".to_string())
}

/// One memory the companion recalled.
#[derive(Debug, Clone)]
pub struct Recalled {
    pub rid: String,
    pub text: String,
    pub score: f64,
}

/// Search the companion's memory.
pub fn recall(query: &str, limit: usize) -> Result<Vec<Recalled>, String> {
    let response = client()
        .call("companion.recall", serde_json::json!({ "query": query, "limit": limit }))
        .map_err(|e| format!("[{}] {}", e.code, e.message))?;
    let rows = response
        .get("results")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "companion returned no results".to_string())?;
    Ok(rows
        .iter()
        .map(|r| Recalled {
            rid: r.get("rid").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            text: r.get("text").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            score: r.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0),
        })
        .collect())
}

/// Run one of the companion's tools by name, without a model in the loop.
///
/// The companion carries ~178 of them — files, windows, browser, containers, packages — behind
/// the permission ceiling in its config. An app that wants one thing done should ask for that
/// thing rather than describe it in a prompt and hope: this is a function call, and it works
/// when the model is unavailable.
///
/// The result is the tool's own prose, which is what tools return; a caller wanting structure
/// parses it.
pub fn tool(name: &str, args: serde_json::Value) -> Result<String, String> {
    let response = client()
        .call(
            "companion.tool",
            serde_json::json!({
                "name": name,
                "args": args,
                "timeout_ms": ASK_TIMEOUT.as_millis() as u64,
            }),
        )
        .map_err(|e| format!("[{}] {}", e.code, e.message))?;
    response
        .get("result")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "companion returned no result".to_string())
}

/// Whether the companion is reachable and its backend answered last time.
///
/// Cheap: an app can call this before offering an AI action.
pub fn is_online() -> bool {
    client()
        .call("companion.status", serde_json::json!({}))
        .ok()
        .and_then(|v| v.get("online").and_then(|o| o.as_bool()))
        .unwrap_or(false)
}
