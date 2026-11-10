//! The doors for a program a Mind chooses (#1124, round 9; Fable's decision of 10 Nov).
//!
//! A program whose name, path or executed content a Mind chooses, or that asks another manager to
//! start one (`sh -c`, `swaymsg exec`, `xdg-open`, `gio`, `systemd-run`, installers,
//! interpreters), starts through exactly one of three doors:
//!
//! - [`mind_command`] (here): the outside view, with the workshops root and the person's
//!   `/run/user/<uid>` hidden;
//! - `browser_spawn`: the browser view, the base hidden and the session's sockets kept;
//! - the shell's `open_app`, for an app a Mind opens (`workspace.rs` asks it; it starts nothing).
//!
//! `safe_command` stays the door for allowlisted utilities with `is_protected` paths. Every other
//! `Command::new` in this crate names one fixed program by a string literal that is not on the
//! denylist, or carries `// fixed-program: a|b` naming every literal it can take. `xtask
//! check-architecture` (rule `mind-spawn`, no baseline) fails the build on anything else.

use std::path::{Path, PathBuf};
use std::process::Command;

use yantrik_ipc_contracts::home_paths::workshops;

/// The person's home, as the views take it.
pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// `program args` as a command that runs in the outside view (`workshops::outside_command`), or
/// the reason it must not run: on a machine that holds workshops and has no `bwrap`, nothing is
/// started. The caller adds the working directory, the environment and the pipes.
pub fn mind_command(home: &Path, program: &str, args: &[String]) -> Result<Command, String> {
    let (program, argv) = workshops::outside_command(home, program, args)?;
    let mut command = Command::new(program);
    command.args(argv);
    Ok(command)
}

/// A Mind's script inside the Strict or Normal sandbox: `bwrap <sandbox_args> sh -c <script>`. The
/// shell here is the sandbox's own (every file and the network are what `sandbox_args` leave), and
/// `script` is built by the caller from a ulimit prelude and a quoted interpreter and path.
pub fn sandboxed_script(sandbox_args: &[String], script: &str) -> Command {
    let mut command = Command::new("/usr/bin/bwrap");
    command.env_remove("LD_PRELOAD"); // bwrap is native; LD_PRELOAD can interfere
    command.args(sandbox_args);
    command.arg("sh").arg("-c").arg(script);
    command
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn argv(c: &Command) -> Vec<String> {
        c.get_args().map(OsStr::to_string_lossy).map(|s| s.into_owned()).collect()
    }

    /// `mind_command` is `outside_command`'s answer, not a parallel construction.
    #[test]
    fn mind_command_is_outside_commands_answer() {
        let home = home();
        let args = vec!["-c".to_string(), "x".to_string()];
        match (mind_command(&home, "tool", &args), workshops::outside_command(&home, "tool", &args)) {
            (Ok(c), Ok((program, want))) => {
                assert_eq!(c.get_program().to_string_lossy(), program);
                assert_eq!(argv(&c), want);
            }
            (Err(e), Err(w)) => assert_eq!(e, w),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_sandbox_door_runs_one_script_inside_bwrap() {
        let c = sandboxed_script(&["--unshare-all".to_string()], "exec python3 x");
        assert_eq!(c.get_program(), "/usr/bin/bwrap");
        assert_eq!(argv(&c), ["--unshare-all", "sh", "-c", "exec python3 x"]);
    }
}
