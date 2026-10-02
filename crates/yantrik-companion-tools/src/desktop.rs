//! Desktop tools — open_url, read_clipboard, write_clipboard,
//! list_files, read_file, run_command.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel, validate_path, glob_match, format_size};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(OpenUrlTool));
    reg.register(Box::new(ReadClipboardTool));
    reg.register(Box::new(WriteClipboardTool));
    reg.register(Box::new(ListFilesTool));
    reg.register(Box::new(ReadFileTool));
    reg.register(Box::new(RunCommandTool));
}

// ── Open URL ──

pub struct OpenUrlTool;

impl Tool for OpenUrlTool {
    fn name(&self) -> &'static str { "open_url" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "desktop" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "open_url",
                "description": "Open URL in user's default browser/app, outside session",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "The URL to open"}
                    },
                    "required": ["url"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
        if url.is_empty() {
            return "Error: url is required".to_string();
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return "Error: URL must start with http:// or https://".to_string();
        }
        // Not `xdg-open`: that opens the person's default browser on the person's desktop. A
        // mind's page opens in the companion's own browser, on the display the shell says minds'
        // windows go to (Mind View), or is refused (PR #582 review, B3).
        super::browser::open_url_for_mind(url)
    }
}

// ── Read Clipboard ──

pub struct ReadClipboardTool;

impl Tool for ReadClipboardTool {
    fn name(&self) -> &'static str { "read_clipboard" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "desktop" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_clipboard",
                "description": "Read the current contents of the user's clipboard",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        if let Some(refused) = crate::provider_keys::clipboard_refusal() {
            return refused;
        }
        // The one bounded reader, which never hands back a provider's key.
        let text = crate::clipboard::read_clipboard_text();
        if text.is_empty() {
            "Clipboard is empty.".to_string()
        } else {
            let truncated = if text.len() > 1000 { &text[..text.floor_char_boundary(1000)] } else { &text };
            format!("Clipboard contents:\n{truncated}")
        }
    }
}

// ── Write Clipboard ──

pub struct WriteClipboardTool;

impl Tool for WriteClipboardTool {
    fn name(&self) -> &'static str { "write_clipboard" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "desktop" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "write_clipboard",
                "description": "Write text to the user's clipboard",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "description": "Text to copy to clipboard"}
                    },
                    "required": ["text"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        // Not while a key is on its way: a model's text there could be pasted as a key.
        if let Some(refused) = crate::provider_keys::clipboard_refusal() {
            return refused;
        }
        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        if text.is_empty() {
            return "Error: text is required".to_string();
        }
        let mut child = match std::process::Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return format!("Failed to write clipboard (wl-copy not available?): {e}"),
        };
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(text.as_bytes());
        }
        match child.wait() {
            Ok(s) if s.success() => "Copied to clipboard.".to_string(),
            Ok(s) => format!("wl-copy exited with: {s}"),
            Err(e) => format!("Failed to write clipboard: {e}"),
        }
    }
}

// ── List Files ──

pub struct ListFilesTool;

impl Tool for ListFilesTool {
    fn name(&self) -> &'static str { "list_files" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "desktop" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_files",
                "description": "List files in a directory; no content search",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Directory path (e.g. ~/Downloads)"},
                        "pattern": {"type": "string", "description": "Optional glob pattern (e.g. *.pdf)"}
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

        let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("*");

        let dir = std::path::Path::new(&expanded);
        if !dir.is_dir() {
            return format!("Error: '{}' is not a directory", path);
        }

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => return format!("Error reading directory: {e}"),
        };

        let mut files = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if pattern == "*" || glob_match(pattern, &name) {
                let meta = entry.metadata().ok();
                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                let suffix = if is_dir { "/" } else { "" };
                files.push(format!("  {}{} ({})", name, suffix, format_size(size)));
            }
        }

        if files.is_empty() {
            format!("No files matching '{}' in {}", pattern, path)
        } else {
            files.sort();
            let count = files.len();
            files.truncate(50);
            let mut result = format!("Files in {} ({} items):\n", path, count);
            result.push_str(&files.join("\n"));
            if count > 50 {
                result.push_str(&format!("\n  ... and {} more", count - 50));
            }
            result
        }
    }
}

// ── Read File ──

pub struct ReadFileTool;

impl Tool for ReadFileTool {
    fn name(&self) -> &'static str { "read_file" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "desktop" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read text file contents",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path to read"},
                        "offset": {"type": "integer", "description": "Start reading from this line number (1-based, default: 1)"},
                        "limit": {"type": "integer", "description": "Maximum number of lines to read (default: 200)"}
                    },
                    "required": ["path"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(1).max(1) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(200) as usize;

        if path.is_empty() {
            return "Error: path is required".to_string();
        }

        let expanded = match validate_path(path) {
            Ok(p) => p,
            Err(e) => return format!("Error: {e}"),
        };

        match std::fs::read_to_string(&expanded) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let total_lines = lines.len();

                if total_lines == 0 {
                    return "(empty file)".to_string();
                }

                let start = (offset - 1).min(total_lines);
                let end = (start + limit).min(total_lines);

                let mut result = String::new();
                for i in start..end {
                    result.push_str(&format!("{:>4}\t{}\n", i + 1, lines[i]));
                }

                if end < total_lines {
                    result.push_str(&format!(
                        "\n... ({} more lines, {} total. Use offset={} to continue)",
                        total_lines - end, total_lines, end + 1
                    ));
                }

                result
            }
            Err(e) => format!("Error reading file: {e}"),
        }
    }
}

// ── Run Command ──

pub struct RunCommandTool;

impl Tool for RunCommandTool {
    fn name(&self) -> &'static str { "run_command" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "desktop" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "run_command",
                "description": "Run shell command only when no specialized tool fits",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string", "description": "The command to run"}
                    },
                    "required": ["command"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let command = args.get("command").and_then(|v| v.as_str()).unwrap_or_default();
        if command.is_empty() {
            return "Error: command is required".to_string();
        }

        // Read-only programs, started directly with their words: no shell (crate::safe_command).
        let argv = match crate::safe_command::words(command) {
            Ok(w) => w,
            Err(e) => return format!("Error: {e}"),
        };
        let home = std::env::var("HOME").unwrap_or_default();
        let argv = crate::safe_command::expand(&argv, &home);
        if let Err(e) = crate::safe_command::check(&argv, &home) {
            return format!("Error: {e}");
        }
        // Defense in depth, as before.
        if let Some(reason) = crate::sanitize::detect_harmful_command(command) {
            return format!("Error: blocked — {reason}");
        }
        match crate::safe_command::run(&argv) {
            Ok((stdout, stderr)) => {
                let mut result = String::new();
                if !stdout.is_empty() {
                    let truncated = if stdout.len() > 2000 { &stdout[..stdout.floor_char_boundary(2000)] } else { &stdout };
                    result.push_str(truncated);
                }
                if !stderr.is_empty() {
                    let end = stderr.floor_char_boundary(stderr.len().min(500));
                    result.push_str(&format!("
Stderr: {}", &stderr[..end]));
                }
                if result.is_empty() {
                    "(no output)".to_string()
                } else {
                    result
                }
            }
            Err(e) => format!("Error: {e}"),
        }
    }
}
