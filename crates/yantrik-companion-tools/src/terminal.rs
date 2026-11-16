//! Terminal tools — read scrollback buffer from the active terminal.
//!
//! Strategies (tried in order):
//! 1. Foot scrollback pipe file (`yantrik-scrollback.txt` in the private scratch dir)
//! 2. tmux capture-pane (if running inside tmux)

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
    Some(without_secret_lines(&text))
}

/// `text` with each line that names a secret (`cat ~/.kube/config`, `vim tokens.json`) replaced by a
/// note: the terminal's lines say which files the person works with (#1247).
pub(crate) fn without_secret_lines(text: &str) -> String {
    text.lines()
        .map(|line| if yantrik_ipc_contracts::home_paths::title_names_a_secret(line) { "(a line that names a protected file is not shown)" } else { line })
        .collect::<Vec<_>>()
        .join("\n")
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

        // The shell's history files are a protected place (they hold pasted tokens and the names of
        // secret files): they are not read as a last resort any more (#1247).
        "No terminal buffer available. Open a Foot terminal and try again.".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_line_that_names_a_secret_is_not_shown() {
        let out = without_secret_lines("$ ls\n$ cat ~/.kube/config\nserver: x\n$ vim tokens.json\n$ echo hi");
        assert!(out.contains("$ ls") && out.contains("$ echo hi") && out.contains("server: x"), "{out}");
        assert_eq!(out.matches("not shown").count(), 2, "{out}");
        assert!(!out.contains("kube") && !out.contains("tokens"), "{out}");
    }
}
