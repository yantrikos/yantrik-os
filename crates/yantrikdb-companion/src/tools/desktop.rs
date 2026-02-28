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
                "description": "Open a URL in the user's web browser.",
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
        match std::process::Command::new("xdg-open").arg(url).spawn() {
            Ok(_) => format!("Opened: {url}"),
            Err(e) => format!("Failed to open URL: {e}"),
        }
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
                "description": "Read the current contents of the user's clipboard.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        match std::process::Command::new("wl-paste")
            .arg("--no-newline")
            .output()
        {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout);
                if text.is_empty() {
                    "Clipboard is empty.".to_string()
                } else {
                    let truncated = if text.len() > 1000 { &text[..1000] } else { &text };
                    format!("Clipboard contents:\n{truncated}")
                }
            }
            Ok(output) => {
                let err = String::from_utf8_lossy(&output.stderr);
                format!("Clipboard read failed: {err}")
            }
            Err(e) => format!("Failed to read clipboard (wl-paste not available?): {e}"),
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
                "description": "Write text to the user's clipboard.",
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
                "description": "List files in a directory on the user's system.",
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
                "description": "Read the contents of a text file. Limited to first 2000 characters.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path to read"}
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
                if content.len() > 2000 {
                    format!("{}\n... (truncated, {} total bytes)", &content[..2000], content.len())
                } else {
                    content
                }
            }
            Err(e) => format!("Error reading file: {e}"),
        }
    }
}

// ── Run Command ──

pub struct RunCommandTool;

/// Safe command allowlist — only read-only, harmless commands.
const SAFE_COMMANDS: &[&str] = &[
    "ls", "cat", "head", "tail", "date", "uptime", "df", "free",
    "whoami", "pwd", "echo", "wc", "file", "stat", "uname",
    "hostname", "id", "which", "env", "printenv",
    "sha256sum", "md5sum", "du", "sort", "grep", "find", "diff", "bc", "cal",
];

impl Tool for RunCommandTool {
    fn name(&self) -> &'static str { "run_command" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "desktop" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "run_command",
                "description": "Run a simple shell command. Only safe, read-only commands are allowed (ls, cat, date, uptime, df, free, whoami, pwd, echo).",
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

        let base_cmd = command.split_whitespace().next().unwrap_or("");
        if !SAFE_COMMANDS.contains(&base_cmd) {
            return format!(
                "Error: '{}' is not in the safe command list. Allowed: {}",
                base_cmd,
                SAFE_COMMANDS.join(", ")
            );
        }

        // Block shell metacharacters
        if command.contains('|') || command.contains(';') || command.contains('&')
            || command.contains('`') || command.contains('$') || command.contains('>')
            || command.contains('<')
        {
            return "Error: shell metacharacters (|;&`$><) are not allowed".to_string();
        }

        match std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .output()
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let mut result = String::new();
                if !stdout.is_empty() {
                    let truncated = if stdout.len() > 2000 { &stdout[..2000] } else { &stdout };
                    result.push_str(truncated);
                }
                if !stderr.is_empty() {
                    result.push_str(&format!("\nStderr: {}", &stderr[..stderr.len().min(500)]));
                }
                if result.is_empty() {
                    "(no output)".to_string()
                } else {
                    result
                }
            }
            Err(e) => format!("Failed to run command: {e}"),
        }
    }
}
