//! Terminal tools — read scrollback buffer from the active terminal.
//!
//! Strategies (tried in order):
//! 1. Foot scrollback pipe file (`/tmp/yantrik-scrollback.txt`)
//! 2. tmux capture-pane (if running inside tmux)
//! 3. Recent shell history as fallback

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ReadTerminalBufferTool));
}

pub struct ReadTerminalBufferTool;

impl Tool for ReadTerminalBufferTool {
    fn name(&self) -> &'static str { "read_terminal_buffer" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "terminal" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_terminal_buffer",
                "description": "Read the recent output from the active terminal (scrollback buffer). Use this when the user asks you to analyze an error, fix a command, or look at terminal output. Returns the last N lines.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "lines": {
                            "type": "integer",
                            "description": "Number of lines to retrieve (default: 50, max: 200)"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let lines = args.get("lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(50)
            .min(200) as usize;

        // Strategy 1: Foot terminal scrollback pipe file.
        // Yantrik configures Foot with: pipe-scrollback=[/bin/sh -c "cat > /tmp/yantrik-scrollback.txt"]
        // bound to a hotkey or triggered automatically.
        let scrollback_path = "/tmp/yantrik-scrollback.txt";
        if let Ok(metadata) = std::fs::metadata(scrollback_path) {
            // Only use if file was modified in the last 60 seconds (fresh dump)
            if let Ok(modified) = metadata.modified() {
                let age = modified.elapsed().unwrap_or_default();
                if age.as_secs() < 60 {
                    if let Ok(content) = std::fs::read_to_string(scrollback_path) {
                        let all_lines: Vec<&str> = content.lines().collect();
                        let start = all_lines.len().saturating_sub(lines);
                        let tail = &all_lines[start..];
                        return format!(
                            "Terminal scrollback (last {} of {} lines):\n{}",
                            tail.len(), all_lines.len(), tail.join("\n")
                        );
                    }
                }
            }
        }

        // Strategy 2: tmux capture-pane (works if user is in a tmux session)
        if let Ok(output) = std::process::Command::new("tmux")
            .args(["capture-pane", "-p", "-S", &format!("-{}", lines)])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                let trimmed = text.trim_end();
                if !trimmed.is_empty() {
                    return format!("Terminal buffer (tmux, last {} lines):\n{}", lines, trimmed);
                }
            }
        }

        // Strategy 3: Read Foot scrollback regardless of age (stale is better than nothing)
        if let Ok(content) = std::fs::read_to_string(scrollback_path) {
            if !content.is_empty() {
                let all_lines: Vec<&str> = content.lines().collect();
                let start = all_lines.len().saturating_sub(lines);
                let tail = &all_lines[start..];
                return format!(
                    "Terminal scrollback (last {} of {} lines, may be stale):\n{}",
                    tail.len(), all_lines.len(), tail.join("\n")
                );
            }
        }

        // Strategy 4: Recent shell history as last resort
        let home = std::env::var("HOME").unwrap_or_default();
        for hist_file in &[".ash_history", ".bash_history", ".zsh_history"] {
            let path = format!("{}/{}", home, hist_file);
            if let Ok(content) = std::fs::read_to_string(&path) {
                let all_lines: Vec<&str> = content.lines().collect();
                let count = lines.min(30);
                let start = all_lines.len().saturating_sub(count);
                let recent = &all_lines[start..];
                return format!(
                    "No terminal buffer available. Recent commands from {} ({} entries):\n{}",
                    hist_file, recent.len(), recent.join("\n")
                );
            }
        }

        "No terminal buffer available. Open a Foot terminal and try again.".to_string()
    }
}
