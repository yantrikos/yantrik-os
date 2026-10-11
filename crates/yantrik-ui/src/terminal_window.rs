//! A terminal window running one command, for the person to type into.
//!
//! For the setup steps only a program's own prompts can do: signing in to a vendor's account
//! (Settings → Accounts), choosing a harness's model (Settings → Harnesses). The desktop does not
//! answer those prompts and does not read what is typed; it opens the window and lets go.
//!
//! `foot --hold`, so what the command said last stays on screen after it exits — a sign-in that
//! failed says why, instead of a window that vanished. `sh -lc`, because the programs these run
//! live in directories only a login PATH names.

use std::ffi::OsStr;
use std::process::Command;

/// What to do with one environment variable in the window: set it, or remove it.
pub type EnvChange<'a> = (&'a str, Option<&'a OsStr>);

/// Open `title`, running `command`, with the session's display environment and `env` applied.
pub fn open(title: &str, command: &str, env: &[EnvChange<'_>]) -> Result<(), String> {
    let mut cmd = Command::new("foot");
    cmd.args(["--title", title, "--hold", "--", "sh", "-lc", command]);
    for (k, v) in crate::wire::dock::session_env() {
        cmd.env(k, v);
    }
    for (var, value) in env {
        match value {
            Some(v) => cmd.env(var, v),
            None => cmd.env_remove(var),
        };
    }
    let mut child = cmd.spawn().map_err(|e| format!("the terminal could not be opened: {e}"))?;
    // Reaped on its own thread, so a closed window does not linger as a zombie of the shell.
    std::thread::Builder::new()
        .name("terminal-window".into())
        .spawn(move || {
            let _ = child.wait();
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}
