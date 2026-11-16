//! What an MCP server may be asked, and what of its answer is shown, when files are in it (#1247,
//! review R11-M4). The server is the person's own, but a proxy tool raises no card for the companion and
//! `mcp_security`'s scan is a list of words (`.ssh`, `id_rsa`, `credential`), which `read_file
//! ~/backup/tokens.json` and `~/.kube/config` pass. A string argument that reads as a path is judged by
//! the rule every other file tool has, and a listing the server answers does not name a protected path.

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

/// `answer` without the lines that are a protected path alone, as a listing prints them (`tokens.json`,
/// `[FILE] tokens.json`, `- /home/p/.kube/config`), counted and never named. A line with more in it is
/// prose or a file's content, and stays.
pub fn without_protected_entries(answer: &str) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let (mut kept, mut hidden) = (Vec::new(), 0usize);
    for line in answer.lines() {
        let entry = line.trim().trim_start_matches(['-', '*']).trim();
        let entry = ["[FILE]", "[DIR]", "[file]", "[dir]"].iter().find_map(|mark| entry.strip_prefix(mark)).unwrap_or(entry).trim();
        if !entry.is_empty() && !entry.contains(char::is_whitespace) && is_protected(&as_path(entry, &home)) {
            hidden += 1;
        } else {
            kept.push(line);
        }
    }
    let mut out = kept.join("\n");
    if hidden > 0 {
        out.push_str(&format!("\n{}", yantrik_companion_tools::safe_command_listing::hidden_note(hidden)));
    }
    out
}

#[cfg(test)]
#[path = "mcp_paths_tests.rs"]
mod tests;
