//! edit_file — surgical string replacement in files.
//!
//! Inspired by Claude Code's Edit tool: precise old→new replacement
//! with uniqueness checking to prevent ambiguous edits.

use serde_json::Value;
use super::{Tool, ToolContext, PermissionLevel, validate_path};

pub fn register(reg: &mut super::ToolRegistry) {
    reg.register(Box::new(EditFileTool));
}

struct EditFileTool;

impl Tool for EditFileTool {
    fn name(&self) -> &'static str { "edit_file" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "files" }

    fn definition(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "edit_file",
                "description": "Perform a surgical text replacement in a file. Finds old_string and replaces it with new_string. The old_string must be unique in the file (unless replace_all is true). Use read_file first to see the current content.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File path to edit"
                        },
                        "old_string": {
                            "type": "string",
                            "description": "The exact text to find and replace. Must match exactly including whitespace and indentation."
                        },
                        "new_string": {
                            "type": "string",
                            "description": "The replacement text"
                        },
                        "replace_all": {
                            "type": "boolean",
                            "description": "Replace all occurrences instead of requiring uniqueness (default: false)"
                        }
                    },
                    "required": ["path", "old_string", "new_string"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let old_string = args.get("old_string").and_then(|v| v.as_str()).unwrap_or_default();
        let new_string = args.get("new_string").and_then(|v| v.as_str()).unwrap_or_default();
        let replace_all = args.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);

        if path.is_empty() || old_string.is_empty() {
            return "Error: path and old_string are required".to_string();
        }

        if old_string == new_string {
            return "Error: old_string and new_string are identical".to_string();
        }

        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        // Read current content
        let content = match std::fs::read_to_string(&expanded) {
            Ok(c) => c,
            Err(e) => return format!("Error reading file: {e}"),
        };

        // Count occurrences
        let count = content.matches(old_string).count();

        if count == 0 {
            // Provide helpful context: show what the file contains near the expected location
            let lines: Vec<&str> = content.lines().collect();
            let preview = if lines.len() <= 10 {
                content.clone()
            } else {
                format!("{}\n... ({} total lines)",
                    lines[..10].join("\n"), lines.len())
            };
            return format!(
                "Error: old_string not found in {path}. The file has {} lines.\nFirst 10 lines:\n{preview}",
                lines.len()
            );
        }

        if count > 1 && !replace_all {
            // Find the line numbers of each occurrence for context
            let mut positions = Vec::new();
            let mut search_start = 0;
            while let Some(pos) = content[search_start..].find(old_string) {
                let abs_pos = search_start + pos;
                let line_num = content[..abs_pos].matches('\n').count() + 1;
                positions.push(line_num);
                search_start = abs_pos + 1;
            }
            return format!(
                "Error: old_string found {} times in {path} (at lines {}). \
                 Provide more surrounding context to make it unique, or set replace_all=true.",
                count,
                positions.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ")
            );
        }

        // Perform replacement
        let new_content = if replace_all {
            content.replace(old_string, new_string)
        } else {
            content.replacen(old_string, new_string, 1)
        };

        // Write back
        match std::fs::write(&expanded, &new_content) {
            Ok(_) => {
                let replaced = if replace_all { count } else { 1 };
                format!("Edited {path}: replaced {} occurrence(s). File is now {} bytes.",
                    replaced, new_content.len())
            }
            Err(e) => format!("Error writing file: {e}"),
        }
    }
}
