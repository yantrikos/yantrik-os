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
    let mut file = yantrik_ml::private_dir::read_scratch(yantrik_ml::private_dir::SCROLLBACK_NAME).ok()?;
    let modified = file.metadata().ok()?.modified().ok()?;
    if modified.elapsed().unwrap_or_default().as_secs() >= max_age_secs {
        return None;
    }
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    Some(without_secret_lines(&text))
}

/// `-J` joins a command the pane wrapped, so a file name split across two rows is still one line
/// to `without_secret_lines`.
pub(crate) const TMUX_CAPTURE_ARGS: [&str; 5] = ["capture-pane", "-p", "-J", "-S", "-"];

/// Start of the note that stands in for the withheld part of a scrollback.
pub(crate) const WITHHELD_NOTE_START: &str = "(";
const WITHHELD_NOTE_TAIL: &str = "line(s) from the first one that names a protected file to the end are not shown)";

/// `text` up to the first line that names a secret (`cat ~/.kube/config`, `vim tokens.json`), then
/// one note saying how many lines it stands for. The output of that command is on the lines after
/// it and nothing tells where it ends, so from that line to the end nothing is shown. The scan
/// covers the whole of `text`, so a command above the window asked for hides its output inside it
/// (#1247).
pub(crate) fn without_secret_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let Some(first) = lines.iter().position(|l| yantrik_ipc_contracts::home_paths::title_names_a_secret(l)) else {
        return lines.join("\n");
    };
    let mut kept: Vec<String> = lines[..first].iter().map(|l| l.to_string()).collect();
    kept.push(format!("{WITHHELD_NOTE_START}{} {WITHHELD_NOTE_TAIL}", lines.len() - first));
    kept.join("\n")
}

/// Whether `line` is the note `without_secret_lines` leaves in place of the withheld lines.
pub(crate) fn is_withheld_note(line: &str) -> bool {
    line.starts_with(WITHHELD_NOTE_START) && line.ends_with(WITHHELD_NOTE_TAIL)
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
            .args(TMUX_CAPTURE_ARGS)
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                let shown = without_secret_lines(text.trim_end());
                if !shown.is_empty() {
                    let all_lines: Vec<&str> = shown.lines().collect();
                    let tail = &all_lines[all_lines.len().saturating_sub(lines)..];
                    return format!("Terminal buffer (tmux, last {} lines):\n{}", tail.len(), tail.join("\n"));
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

    const CAT: &str = "$ ls\n$ cat ~/.kube/config\nserver: FAKE-SECRET-1247\n$ echo after-the-cat";

    #[test]
    fn nothing_after_the_first_line_that_names_a_secret_is_shown() {
        let out = without_secret_lines(CAT);
        assert!(out.starts_with("$ ls\n"), "{out}");
        assert!(!out.contains("FAKE-SECRET") && !out.contains("after-the-cat") && !out.contains("kube"), "{out}");
        assert!(out.lines().last().is_some_and(is_withheld_note) && out.contains("3 line"), "{out}");
    }

    #[test]
    fn tmux_is_asked_to_join_wrapped_lines() {
        assert!(TMUX_CAPTURE_ARGS.contains(&"-J"), "{TMUX_CAPTURE_ARGS:?}");
        // Joined, a command wrapped inside a file name is one line and names the secret.
        let joined = "$ cat backup-folder-with-a-longish-name/tokens.json; echo after-the-cat\nFAKE-SECRET-1247";
        let out = without_secret_lines(joined);
        assert!(!out.contains("FAKE-SECRET") && !out.contains("after-the-cat"), "{out}");
    }

    #[test]
    fn a_scrollback_that_names_no_secret_is_shown_whole() {
        assert_eq!(without_secret_lines("$ ls\nfoo\n$ echo hi"), "$ ls\nfoo\n$ echo hi");
    }

    #[test]
    fn a_command_above_the_window_hides_its_output_inside_it() {
        let mut text = String::from("$ cat tokens.json\nFAKE-SECRET-1247\n");
        for i in 0..300 { text.push_str(&format!("line {i}\n")); }
        let out = without_secret_lines(&text);
        let window: Vec<&str> = out.lines().rev().take(5).collect();
        assert!(window.iter().all(|l| !l.contains("FAKE-SECRET") && !l.contains("line ")), "{window:?}");
        assert!(out.contains("302 line"), "{out}");
    }
}
