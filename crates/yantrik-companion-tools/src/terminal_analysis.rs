//! Terminal analysis tools — error detection, history search, error explanation.
//!
//! Tools: detect_terminal_errors, search_terminal_history, explain_last_error.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};
// The same dump `read_terminal_buffer` reads, from the same private place.
use super::terminal::{is_withheld_note, read_scrollback};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(DetectTerminalErrorsTool));
    reg.register(Box::new(SearchTerminalHistoryTool));
    reg.register(Box::new(ExplainLastErrorTool));
}

/// Error patterns to scan for (all lowercase).
const ERROR_PATTERNS: &[&str] = &[
    "error:", "error[", "failed", "fatal:", "panic", "traceback",
    "exception", "command not found", "no such file", "permission denied",
    "killed", "oom", "segmentation fault", "cannot find", "undefined",
];

/// Warning patterns to scan for (all lowercase).
const WARNING_PATTERNS: &[&str] = &["warning:", "warn["];

// ── Detect Terminal Errors ──

pub struct DetectTerminalErrorsTool;

impl Tool for DetectTerminalErrorsTool {
    fn name(&self) -> &'static str { "detect_terminal_errors" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "terminal" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "detect_terminal_errors",
                "description": "Find likely errors in terminal output",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let content = match read_scrollback(120) {
            Some(c) => c,
            None => return "No fresh scrollback available (file missing or older than 120s).".to_string(),
        };

        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        for line in content.lines() {
            let lower = line.to_lowercase();
            let is_error = ERROR_PATTERNS.iter().any(|p| lower.contains(p));
            let is_warning = WARNING_PATTERNS.iter().any(|p| lower.contains(p));

            if is_error {
                errors.push(line);
            } else if is_warning {
                warnings.push(line);
            }
        }

        let mut out = String::new();
        out.push_str(&format!("Errors ({}):\n", errors.len()));
        if errors.is_empty() {
            out.push_str("  (none)\n");
        } else {
            for e in &errors {
                out.push_str(&format!("- {}\n", e));
            }
        }

        out.push_str(&format!("\nWarnings ({}):\n", warnings.len()));
        if warnings.is_empty() {
            out.push_str("  (none)\n");
        } else {
            for w in &warnings {
                out.push_str(&format!("- {}\n", w));
            }
        }

        out
    }
}

// ── Search Terminal History ──

pub struct SearchTerminalHistoryTool;

impl Tool for SearchTerminalHistoryTool {
    fn name(&self) -> &'static str { "search_terminal_history" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "terminal" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "search_terminal_history",
                "description": "Search past terminal commands or output",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": {"type": "string", "description": "Pattern to search for"},
                        "max_results": {"type": "integer", "description": "Maximum results to return (default: 20)"}
                    },
                    "required": ["pattern"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or_default();
        let max_results = args.get("max_results").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

        if pattern.is_empty() {
            return "Error: pattern is required".to_string();
        }

        let pattern_lower = pattern.to_lowercase();

        // The shell's history files are a protected place, so they are not searched (#1247); the
        // terminal's own scrollback is, with the lines that name a secret left out.
        let Some(content) = read_scrollback(u64::MAX) else {
            return "No terminal scrollback found.".to_string();
        };
        return search_lines(&content, &pattern_lower, pattern, max_results);
    }
}
/// The lines of `content` (a scrollback that has been through `without_secret_lines`) holding
/// `pattern_lower`; the note for withheld lines is never a match, and the answer says when lines
/// were withheld.
fn search_lines(content: &str, pattern_lower: &str, pattern: &str, max_results: usize) -> String {
    let matches: Vec<&str> = content.lines().filter(|line| !is_withheld_note(line) && line.to_lowercase().contains(pattern_lower)).collect();
    let withheld_note = if content.lines().any(is_withheld_note) { " Part of the scrollback is withheld: it follows a line that names a protected file." } else { "" };
    let recent = &matches[matches.len().saturating_sub(max_results)..];
    if recent.is_empty() {
        return format!("No lines matching '{}' found in the shown part of the terminal scrollback.{}", pattern, withheld_note);
    }
    let mut out = format!("Found {} matches for '{}' (showing last {}):\n", matches.len(), pattern, recent.len());
    for line in recent {
        out.push_str(&format!("  {}\n", line));
    }
    out.push_str(withheld_note);
    out
}

// ── Explain Last Error ──

pub struct ExplainLastErrorTool;

impl Tool for ExplainLastErrorTool {
    fn name(&self) -> &'static str { "explain_last_error" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "terminal" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "explain_last_error",
                "description": "Explain the most recent terminal error",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let content = match read_scrollback(120) {
            Some(c) => c,
            None => return "No fresh scrollback available (file missing or older than 120s).".to_string(),
        };

        let all_lines: Vec<&str> = content.lines().collect();

        // Extract error lines
        let errors: Vec<&str> = all_lines
            .iter()
            .filter(|line| {
                let lower = line.to_lowercase();
                ERROR_PATTERNS.iter().any(|p| lower.contains(p))
            })
            .copied()
            .collect();

        // Last 20 lines for context
        let tail_start = all_lines.len().saturating_sub(20);
        let tail: Vec<&str> = all_lines[tail_start..].to_vec();

        let mut out = String::new();
        if errors.is_empty() {
            out.push_str("Last terminal errors:\n  (no errors detected)\n");
        } else {
            out.push_str(&format!("Last terminal errors ({}):\n", errors.len()));
            // Show last 10 errors max
            let err_start = errors.len().saturating_sub(10);
            for e in &errors[err_start..] {
                out.push_str(&format!("  {}\n", e));
            }
        }

        out.push_str(&format!("\nContext (last {} lines):\n", tail.len()));
        for line in &tail {
            out.push_str(&format!("  {}\n", line));
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_only_in_a_withheld_line_is_not_found_and_the_answer_says_lines_were_withheld() {
        let shown = super::super::terminal::without_secret_lines("$ ls\n$ cat tokens.json\nzebra-word\n$ echo after");
        let out = search_lines(&shown, "zebra", "zebra", 20);
        assert!(!out.contains("zebra-word") && out.contains("withheld"), "{out}");
        let out = search_lines(&shown, "protected", "protected", 20);
        assert!(out.starts_with("No lines matching"), "{out}");
        assert!(search_lines("a\nb", "zebra", "zebra", 20).find("withheld").is_none());
    }
}
