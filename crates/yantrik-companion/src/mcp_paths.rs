//! What an MCP server may be asked when files are in it (#1247, review R11-M4). The server is the
//! person's own, but a proxy tool raises no card for the companion and `mcp_security`'s scan is a list of
//! words (`.ssh`, `id_rsa`, `credential`), which `read_file ~/backup/tokens.json` and `~/.kube/config`
//! pass. A string argument that reads as a path is judged by the rule every other file tool has.
//!
//! What a server answers is **not** filtered: a listing or a file it returns passes through as it is. That
//! is a known limit, named in SECURITY.md, and it is closed by #1266 (a card for each MCP call).

use std::path::{Path, PathBuf};

use serde_json::Value;
use yantrik_ipc_contracts::home_paths::is_protected;

/// The rule's own words.
pub const REFUSAL: &str = "Access denied: path resolves to a protected location";

/// The path a string argument names: `~` and `~/..` are the home, a `file://` link is its path; anything
/// else is taken as written, so a bare `tokens.json` or `.kube/config` is judged by its name and its places.
fn as_path(text: &str, home: &Path) -> PathBuf {
    let text = text.strip_prefix("file://").unwrap_or(text);
    yantrik_ipc_contracts::home_paths::expand(text, home).unwrap_or_else(|| PathBuf::from(text))
}

/// Whether any string in `args`, at any depth (a path argument, or one in a list of them), names a
/// protected place or file.
fn names_a_protected_path(args: &Value, home: &Path) -> bool {
    match args {
        Value::String(text) => !text.is_empty() && text.len() <= 4096 && is_protected(&as_path(text, home)),
        Value::Array(items) => items.iter().any(|item| names_a_protected_path(item, home)),
        Value::Object(fields) => fields.values().any(|value| names_a_protected_path(value, home)),
        _ => false,
    }
}

/// `Some(sentence)` when a request must not be sent: it names a protected path.
pub fn refusal(args: &Value) -> Option<&'static str> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    names_a_protected_path(args, &home).then_some(REFUSAL)
}

#[cfg(test)]
#[path = "mcp_paths_tests.rs"]
mod tests;
