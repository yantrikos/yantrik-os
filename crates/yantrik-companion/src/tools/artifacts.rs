//! Fix artifacts — generate, list, and read shareable fix summaries.
//!
//! Tools: generate_fix_summary, list_fixes, read_fix.
//! Writes Markdown files to ~/fixes/.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path, expand_home};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(GenerateFixSummaryTool));
    reg.register(Box::new(ListFixesTool));
    reg.register(Box::new(ReadFixTool));
}

// ── Generate Fix Summary ──

pub struct GenerateFixSummaryTool;

impl Tool for GenerateFixSummaryTool {
    fn name(&self) -> &'static str { "generate_fix_summary" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "artifacts" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "generate_fix_summary",
                "description": "Generate a Markdown fix document and save it to ~/fixes/",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": {"type": "string", "description": "Short title for the fix"},
                        "problem": {"type": "string", "description": "What went wrong"},
                        "solution": {"type": "string", "description": "How it was fixed"},
                        "tags": {"type": "string", "description": "Comma-separated tags (e.g. 'rust,compile,icu')"}
                    },
                    "required": ["title", "problem", "solution"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let title = args.get("title").and_then(|v| v.as_str()).unwrap_or_default();
        let problem = args.get("problem").and_then(|v| v.as_str()).unwrap_or_default();
        let solution = args.get("solution").and_then(|v| v.as_str()).unwrap_or_default();
        let tags = args.get("tags").and_then(|v| v.as_str()).unwrap_or("");

        if title.is_empty() || problem.is_empty() || solution.is_empty() {
            return "Error: title, problem, and solution are all required".to_string();
        }

        // Create ~/fixes/ directory
        let fixes_dir = expand_home("~/fixes");
        if let Err(e) = std::fs::create_dir_all(&fixes_dir) {
            return format!("Error creating ~/fixes/: {e}");
        }

        // Slugify title for filename
        let slug: String = title
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_string();
        let slug = slug.replace("--", "-");

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let filename = format!("{}-{}.md", slug, ts);
        let filepath = format!("{}/{}", fixes_dir, filename);

        // Build markdown content
        let tag_line = if tags.is_empty() {
            String::new()
        } else {
            format!("**Tags**: {}\n\n", tags)
        };

        let content = format!(
            "# {title}\n\n{tag_line}## Problem\n\n{problem}\n\n## Solution\n\n{solution}\n"
        );

        if let Err(e) = std::fs::write(&filepath, &content) {
            return format!("Error writing fix file: {e}");
        }

        // Store in memory for future recall
        let memory_text = format!("Fix: {title} — {problem} → {solution}");
        let _ = ctx.db.record_text(
            &memory_text,
            "semantic",
            0.7,
            0.0,
            0.0, // no expiry
            &serde_json::json!({"tags": tags, "file": filepath}),
            "default",
            0.9,
            "fixes",
            "self",
            None,
        );

        format!("Fix saved to ~/fixes/{filename}")
    }
}

// ── List Fixes ──

pub struct ListFixesTool;

impl Tool for ListFixesTool {
    fn name(&self) -> &'static str { "list_fixes" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "artifacts" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_fixes",
                "description": "List saved fix documents from ~/fixes/",
                "parameters": {"type": "object", "properties": {}}
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let fixes_dir = expand_home("~/fixes");
        let entries = match std::fs::read_dir(&fixes_dir) {
            Ok(e) => e,
            Err(_) => return "No fixes directory found. Generate a fix first.".to_string(),
        };

        let mut fixes = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".md") {
                continue;
            }
            // Read first line as title
            let title = std::fs::read_to_string(entry.path())
                .ok()
                .and_then(|c| c.lines().next().map(|l| l.trim_start_matches("# ").to_string()))
                .unwrap_or_else(|| name.clone());
            fixes.push(format!("- {} ({})", title, name));
        }

        if fixes.is_empty() {
            "No fix documents found in ~/fixes/.".to_string()
        } else {
            format!("Fix documents ({}):\n{}", fixes.len(), fixes.join("\n"))
        }
    }
}

// ── Read Fix ──

pub struct ReadFixTool;

impl Tool for ReadFixTool {
    fn name(&self) -> &'static str { "read_fix" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "artifacts" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_fix",
                "description": "Read a specific fix document from ~/fixes/",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "filename": {"type": "string", "description": "Fix filename (e.g. 'rustc-ice-fix-1709123456.md')"}
                    },
                    "required": ["filename"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let filename = args.get("filename").and_then(|v| v.as_str()).unwrap_or_default();
        if filename.is_empty() {
            return "Error: filename is required".to_string();
        }

        let path = format!("~/fixes/{}", filename);
        match validate_path(&path) {
            Ok(expanded) => match std::fs::read_to_string(&expanded) {
                Ok(content) => {
                    if content.len() > 4000 {
                        format!("{}\n\n[Truncated — {} bytes total]", &content[..content.floor_char_boundary(4000)], content.len())
                    } else {
                        content
                    }
                }
                Err(e) => format!("Error reading fix: {e}"),
            },
            Err(e) => format!("Path error: {e}"),
        }
    }
}
