//! Terminal analysis tools — error detection, history search, error explanation.
//!
//! Tools: detect_terminal_errors, search_terminal_history, explain_last_error.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, expand_home};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(DetectTerminalErrorsTool));
    reg.register(Box::new(SearchTerminalHistoryTool));
    reg.register(Box::new(ExplainLastErrorTool));
}

/// Read the scrollback file if it exists and is fresh (< max_age seconds).
fn read_scrollback(max_age_secs: u64) -> Option<String> {
    let path = "/tmp/yantrik-scrollback.txt";
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let age = modified.elapsed().unwrap_or_default();
    if age.as_secs() >= max_age_secs {
        return None;
    }
    std::fs::read_to_string(path).ok()
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
                "description": "Scan the terminal scrollback for errors and warnings.",
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
                "description": "Search shell command history for a pattern.",
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

        // Try history files in order
        let history_files = [
            expand_home("~/.ash_history"),
            expand_home("~/.bash_history"),
            expand_home("~/.zsh_history"),
        ];

        for hist_path in &history_files {
            if let Ok(content) = std::fs::read_to_string(hist_path) {
                let matches: Vec<&str> = content
                    .lines()
                    .filter(|line| line.to_lowercase().contains(&pattern_lower))
                    .collect();

                let start = matches.len().saturating_sub(max_results);
                let recent = &matches[start..];

                if recent.is_empty() {
                    return format!("No commands matching '{}' found in {}.", pattern, hist_path);
                }

                let mut out = format!(
                    "Found {} matches for '{}' (showing last {}):\n",
                    matches.len(), pattern, recent.len()
                );
                for line in recent {
                    out.push_str(&format!("  {}\n", line));
                }
                return out;
            }
        }

        "No shell history files found.".to_string()
    }
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
                "description": "Get the last terminal error with surrounding context for analysis.",
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
