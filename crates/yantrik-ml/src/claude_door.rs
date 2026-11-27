//! The one door for an unattended `claude -p` (#1435). Unix only.
//!
//! `claude` started unattended is a program a Mind steers: its prompt is the whole of what it does.
//! Every such start carries the same three flags ([`SEALED_PRINT_FLAGS`]), runs in the outside view
//! with the allow-listed environment (the same construction as `yantrik-companion-tools`'s
//! `spawn::mind_command`, built here because `yantrik-ml` is below that crate), and never has
//! untrusted text in argv: the conversation goes on stdin.
//!
//! Layering: the list lives here, in the lower crate, and `yantrik-companion-tools` re-exports it
//! (`claude::SEALED_PRINT_FLAGS`), so there is one copy and nothing to keep equal.
//! `xtask check-architecture` (rule `mind-spawn`) holds this crate too; this file is a door.

use std::io::Write;
use std::process::{Command, Stdio};

use yantrik_ipc_contracts::home_paths::{outside_view, view_env, workshops};

/// The settings the sealed start passes: reading any file is denied, so an `@` mention sends nothing.
pub const SEALED_SETTINGS: &str = r#"{"permissions":{"deny":["Read"]}}"#;

/// The flags every unattended `claude -p` carries: no built-in tools, none of the person's own MCP
/// servers, and no `Read`, so an `@path` mention in a prompt is not expanded by the CLI (R12-H1).
pub const SEALED_PRINT_FLAGS: [&str; 5] = ["--tools", "", "--strict-mcp-config", "--settings", SEALED_SETTINGS];

/// A fixed system prompt for a start whose real system prompt travels on stdin: with any
/// `--system-prompt` the CLI's own coding-agent prompt is replaced, not extended.
pub const STDIN_SYSTEM_PROMPT: &str = "You answer as the assistant described in the SYSTEM section at the start of the input, following it exactly. The rest of the input is the conversation so far.";

/// The argv of a sealed print start: `-p`, the sealed flags, then `extra` (fixed, trusted words only).
pub fn sealed_words(extra: &[String]) -> Vec<String> {
    let mut words = vec!["-p".to_string()];
    words.extend(SEALED_PRINT_FLAGS.map(String::from));
    words.extend(extra.iter().cloned());
    words
}

/// `claude <words>` in the outside view, or the reason it must not run. `CLAUDECODE` is not in the
/// environment (the allow list drops it; it is not set to empty), and
/// `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` is.
pub fn claude_command(words: &[String]) -> Result<Command, String> {
    use std::os::unix::process::CommandExt;
    let home = outside_view::account_home()?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| home.clone());
    let extra = [("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_string(), "1".to_string())];
    let env = view_env::process_env(&home, &extra);
    let (program, argv, held) = workshops::outside_command_with(&home, &cwd, &[], &env, "claude", words)?;
    let mut command = Command::new(program);
    command.args(argv).env_clear();
    // SAFETY: `seal` calls only async-signal-safe functions and allocates nothing.
    unsafe { command.pre_exec(move || held.seal(3)) };
    Ok(command)
}

/// Run a prepared command with `input` on its stdin, and return its stdout.
pub fn run_with_stdin(mut command: Command, input: &str) -> Result<String, String> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run claude CLI. Is it installed? ({e})"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let bytes = input.as_bytes().to_vec();
    // Written on its own thread: a long conversation must not block on a full pipe while the
    // child's output pipe fills.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&bytes);
    });
    let output = child.wait_with_output().map_err(|e| format!("claude CLI: {e}"))?;
    let _ = writer.join();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Claude CLI error (exit {}): {}", output.status, stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// What goes on stdin: the system prompt under its own heading, then the conversation.
pub fn stdin_input(system: Option<&str>, conversation: &str) -> String {
    match system {
        Some(s) if !s.is_empty() => format!("SYSTEM:\n{s}\n\nCONVERSATION:\n{conversation}"),
        _ => conversation.to_string(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_sealed_flags_are_the_three_flags() {
        let w = sealed_words(&[]);
        assert_eq!(w[0], "-p");
        assert_eq!(w[1..3], ["--tools", ""]);
        assert!(w.contains(&"--strict-mcp-config".to_string()));
        let at = w.iter().position(|x| x == "--settings").unwrap();
        assert_eq!(w[at + 1], r#"{"permissions":{"deny":["Read"]}}"#);
    }

    #[test]
    fn stdin_holds_system_and_conversation() {
        assert_eq!(stdin_input(Some("be kind"), "hi"), "SYSTEM:\nbe kind\n\nCONVERSATION:\nhi");
        assert_eq!(stdin_input(None, "hi"), "hi");
    }
}
