//! What an MCP server may be asked, and what of its answer is shown, when files are in it (#1247,
//! review R11-M4). The server is the person's own, but a proxy tool raises no card for the companion and
//! `mcp_security`'s scan is a list of words (`.ssh`, `id_rsa`, `credential`), which `read_file
//! ~/backup/tokens.json` and `~/.kube/config` pass. A string argument that reads as a path is judged by
//! the rule every other file tool has, and a listing or a tree the server answers does not name a secret.

use std::path::{Path, PathBuf};

use serde_json::Value;
use yantrik_ipc_contracts::home_paths::{is_protected, title_names_a_secret};

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

/// Whether a line, or a string in a JSON answer, names a secret: a protected path alone (`tokens.json`,
/// `[FILE] tokens.json`, `- ~/.kube/config`) or any text that holds one, by the judge every other
/// surface uses for free text (`title_names_a_secret`: copy forms with a space, sizes after a name, names
/// inside other text). A sentence that names a secret is hidden too: the rule is that no secret file's
/// name is shown (#1247, N-M2).
fn names_a_secret(text: &str, home: &Path) -> bool {
    let entry = text.trim().trim_start_matches(['-', '*']).trim();
    let entry = ["[FILE]", "[DIR]", "[file]", "[dir]"].iter().find_map(|mark| entry.strip_prefix(mark)).unwrap_or(entry).trim();
    (!entry.is_empty() && !entry.contains(char::is_whitespace) && is_protected(&as_path(entry, home))) || title_names_a_secret(text)
}

/// `value` without every object or string that names a secret (`None` when it is one itself); `hidden`
/// counts what was left out. An object holding a hit in any of its own strings goes whole, as a
/// `directory_tree` entry does.
fn scrubbed(value: Value, home: &Path, hidden: &mut usize) -> Option<Value> {
    match value {
        Value::String(text) if names_a_secret(&text, home) => {
            *hidden += 1;
            None
        }
        Value::Array(items) => Some(Value::Array(items.into_iter().filter_map(|item| scrubbed(item, home, hidden)).collect())),
        Value::Object(fields) => {
            if fields.values().any(|v| v.as_str().is_some_and(|text| names_a_secret(text, home))) {
                *hidden += 1;
                return None;
            }
            Some(Value::Object(fields.into_iter().filter_map(|(key, v)| scrubbed(v, home, hidden).map(|v| (key, v))).collect()))
        }
        other => Some(other),
    }
}

/// `answer` without what names a protected file, counted and never named: a text answer line by line,
/// and one that parses as JSON (`directory_tree`) object by object.
pub fn without_protected_entries(answer: &str) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let mut hidden = 0usize;
    let mut out = match serde_json::from_str::<Value>(answer) {
        Ok(json @ (Value::Array(_) | Value::Object(_))) => scrubbed(json, &home, &mut hidden).map(|v| v.to_string()).unwrap_or_default(),
        _ => answer.lines().filter(|line| !names_a_secret(line, &home) || { hidden += 1; false }).collect::<Vec<_>>().join("\n"),
    };
    if hidden > 0 {
        out.push_str(&format!("\n{}", yantrik_companion_tools::safe_command_listing::hidden_note(hidden)));
    }
    out
}

#[cfg(test)]
#[path = "mcp_paths_tests.rs"]
mod tests;
