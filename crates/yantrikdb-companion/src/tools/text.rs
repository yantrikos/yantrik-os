//! Text tools — word_count, diff_files, hash_file.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path, format_size};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(WordCountTool));
    reg.register(Box::new(DiffFilesTool));
    reg.register(Box::new(HashFileTool));
}

// ── Word Count ──

pub struct WordCountTool;

impl Tool for WordCountTool {
    fn name(&self) -> &'static str { "word_count" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "text" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "word_count",
                "description": "Count lines, words, and characters in a file.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path"}
                    },
                    "required": ["path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        if path.is_empty() {
            return "Error: path is required".to_string();
        }

        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        match std::fs::read_to_string(&expanded) {
            Ok(content) => {
                let lines = content.lines().count();
                let words = content.split_whitespace().count();
                let chars = content.len();
                let size = std::fs::metadata(&expanded)
                    .map(|m| format_size(m.len()))
                    .unwrap_or_else(|_| "unknown".to_string());
                format!("{path}: {lines} lines, {words} words, {chars} chars ({size})")
            }
            Err(e) => format!("Error reading file: {e}"),
        }
    }
}

// ── Diff Files ──

pub struct DiffFilesTool;

impl Tool for DiffFilesTool {
    fn name(&self) -> &'static str { "diff_files" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "text" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "diff_files",
                "description": "Show differences between two text files.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path_a": {"type": "string", "description": "First file"},
                        "path_b": {"type": "string", "description": "Second file"}
                    },
                    "required": ["path_a", "path_b"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path_a = args.get("path_a").and_then(|v| v.as_str()).unwrap_or_default();
        let path_b = args.get("path_b").and_then(|v| v.as_str()).unwrap_or_default();

        if path_a.is_empty() || path_b.is_empty() {
            return "Error: path_a and path_b are required".to_string();
        }

        let expanded_a = match validate_path(path_a) {
            Ok(p) => p,
            Err(e) => return format!("Error (path_a): {e}"),
        };
        let expanded_b = match validate_path(path_b) {
            Ok(p) => p,
            Err(e) => return format!("Error (path_b): {e}"),
        };

        match std::process::Command::new("diff")
            .args(["-u", &expanded_a, &expanded_b])
            .output()
        {
            Ok(output) => {
                let diff = String::from_utf8_lossy(&output.stdout);
                if diff.is_empty() {
                    "Files are identical.".to_string()
                } else {
                    let truncated = if diff.len() > 3000 {
                        format!("{}...\n(truncated, {} total chars)", &diff[..3000], diff.len())
                    } else {
                        diff.to_string()
                    };
                    truncated
                }
            }
            Err(e) => format!("Error (diff not available?): {e}"),
        }
    }
}

// ── Hash File ──

pub struct HashFileTool;

impl Tool for HashFileTool {
    fn name(&self) -> &'static str { "hash_file" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "text" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "hash_file",
                "description": "Compute SHA-256 hash of a file.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path"}
                    },
                    "required": ["path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        if path.is_empty() {
            return "Error: path is required".to_string();
        }

        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        match std::process::Command::new("sha256sum")
            .arg(&expanded)
            .output()
        {
            Ok(output) if output.status.success() => {
                let out = String::from_utf8_lossy(&output.stdout);
                // sha256sum output: "hash  filename"
                if let Some(hash) = out.split_whitespace().next() {
                    format!("{path}: {hash}")
                } else {
                    out.trim().to_string()
                }
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                format!("sha256sum failed: {err}")
            }
            Err(e) => format!("Error (sha256sum not available?): {e}"),
        }
    }
}
