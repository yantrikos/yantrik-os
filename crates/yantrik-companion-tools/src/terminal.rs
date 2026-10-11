//! Terminal tools — read scrollback buffer from the active terminal.
//!
//! Strategies (tried in order):
//! 1. Foot scrollback pipe file (`yantrik-scrollback.txt` in the private scratch dir)
//! 2. tmux capture-pane (if running inside tmux)
//! 3. Recent shell history as fallback

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ReadTerminalBufferTool));
}

/// The scrollback dump, if there is one younger than `max_age_secs` (`u64::MAX` for any age) —
/// read by this tool and by `terminal_analysis`, so both look in the same place.
///
/// The writers are the foot `pipe-scrollback` binding and the labwc Super+E binding that
/// deploy-stack.sh installs; they resolve the same directory in shell
/// (`$XDG_RUNTIME_DIR/yantrik-scratch` when the runtime dir exists, else `~/.cache/yantrik/tmp`),
/// so change both together. It used to
/// be a fixed name in `/tmp`, where anyone could have left a "fresh" dump for us to read as the
/// person's terminal — and then act on the errors it claimed.
///
/// Read through `read_scratch`, not a plain open: the file tools can leave anything at this name
/// (extracting an archive is enough), and `yantrik-scrollback.txt -> ~/.ssh/id_ed25519` would
/// otherwise hand the key to the model as "the terminal". The age is taken from the descriptor
/// that is read, so it is this file's age and not whatever the name pointed at a moment before.
pub(crate) fn read_scrollback(max_age_secs: u64) -> Option<String> {
    use std::io::Read;
    let mut file = yantrik_ml::private_dir::read_scratch("yantrik-scrollback.txt").ok()?;
    let modified = file.metadata().ok()?.modified().ok()?;
    if modified.elapsed().unwrap_or_default().as_secs() >= max_age_secs {
        return None;
    }
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    Some(text)
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
                "description": "Read recent terminal output; not run commands",
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
        // Yantrik configures Foot with a pipe-scrollback binding that writes the file
        // `read_scrollback` reads, bound to a hotkey or triggered automatically.
        // Only use if file was modified in the last 60 seconds (fresh dump)
        if let Some(content) = read_scrollback(60) {
            let all_lines: Vec<&str> = content.lines().collect();
            let start = all_lines.len().saturating_sub(lines);
            let tail = &all_lines[start..];
            return format!(
                "Terminal scrollback (last {} of {} lines):\n{}",
                tail.len(), all_lines.len(), tail.join("\n")
            );
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
        if let Some(content) = read_scrollback(u64::MAX) {
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
