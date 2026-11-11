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

/// Controls how isolated the script execution environment is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SandboxMode {
    /// Full isolation: no network, read-only filesystem, PID namespace.
    /// Default for code_execute and script_run.
    Strict,
    /// Filesystem isolation but network allowed.
    /// For web portals, API calls, data fetching scripts.
    Network,
    /// No sandbox. Only used if bwrap unavailable.
    None,
}

impl SandboxMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "network" | "web" => Self::Network,
            "none" | "unsafe" => Self::None,
            _ => Self::Strict,
        }
    }
}

/// Build bwrap arguments for sandboxed execution.
fn build_bwrap_args(
    sandbox: SandboxMode,
    workspace: &Path,
    output: &Path,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();

    // Read-only system mounts
    for dir in &["/usr", "/lib", "/bin", "/etc"] {
        if Path::new(dir).exists() {
            args.push("--ro-bind".into());
            args.push(dir.to_string());
            args.push(dir.to_string());
        }
    }
    // /sbin for system tools some scripts may need
    if Path::new("/sbin").exists() {
        args.push("--ro-bind".into());
        args.push("/sbin".into());
        args.push("/sbin".into());
    }
    // lib64 symlink (needed on some systems)
    if Path::new("/usr/lib64").exists() {
        args.push("--symlink".into());
        args.push("usr/lib64".into());
        args.push("/lib64".into());
    }

    // Writable workspace (scripts + output)
    args.push("--bind".into());
    args.push(workspace.to_string_lossy().to_string());
    args.push(workspace.to_string_lossy().to_string());
    if output != workspace && output.starts_with(workspace) {
        // output is inside workspace, already covered
    } else if output.exists() {
        args.push("--bind".into());
        args.push(output.to_string_lossy().to_string());
        args.push(output.to_string_lossy().to_string());
    }

    // Writable /tmp for temp files
    args.push("--tmpfs".into());
    args.push("/tmp".into());

    // /dev and /proc (needed for basic operation)
    args.push("--dev".into());
    args.push("/dev".into());
    args.push("--proc".into());
    args.push("/proc".into());

    // PID namespace (always isolate PIDs)
    args.push("--unshare-pid".into());

    // Network isolation
    match sandbox {
        SandboxMode::Strict => {
            args.push("--unshare-net".into());
        }
        SandboxMode::Network => {
            args.push("--share-net".into());
        }
        SandboxMode::None => unreachable!("bwrap not used in None mode"),
    }

    // Die when parent dies (prevent orphan processes)
    args.push("--die-with-parent".into());

    // Resource limits via inner ulimit wrapper
    // (bwrap doesn't set ulimits directly — we wrap the command)

    args.push("--".into());
    args
}

/// A Mind's script inside the Strict or Network sandbox: `bwrap <the sandbox's args> sh -c
/// <script>`. The caller names only the [`SandboxMode`], the writable `workspace` and `output`
/// dirs and the script (a ulimit prelude and a quoted interpreter and path); the bwrap arguments
/// are built here, so no caller can widen what the shell sees. It is the sandbox's own shell
/// (`/usr /lib /bin /etc /sbin` read-only, a tmpfs `/tmp`, a fresh `/proc`, no workshops root).
pub fn sandboxed_script(sandbox: SandboxMode, workspace: &Path, output: &Path, script: &str) -> Command {
    let mut command = Command::new("/usr/bin/bwrap");
    command.env_remove("LD_PRELOAD"); // bwrap is native; LD_PRELOAD can interfere
    command.args(build_bwrap_args(sandbox, workspace, output));
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
    fn the_sandbox_door_runs_one_script_inside_bwrap_with_its_own_args() {
        let (ws, out) = (Path::new("/tmp/ws"), Path::new("/tmp/out"));
        let strict = sandboxed_script(SandboxMode::Strict, ws, out, "exec python3 x");
        assert_eq!(strict.get_program(), "/usr/bin/bwrap");
        let args = argv(&strict);
        assert_eq!(args[args.len() - 3..], ["sh", "-c", "exec python3 x"]);
        assert_eq!(args[..args.len() - 3], build_bwrap_args(SandboxMode::Strict, ws, out));
        assert!(args.contains(&"--unshare-net".to_string()) && !args.contains(&"--share-net".to_string()));
        let net = argv(&sandboxed_script(SandboxMode::Network, ws, out, "x"));
        assert!(net.contains(&"--share-net".to_string()));
    }

    /// Nothing in the sandbox's args binds the root, the workshops base or `/run/user`.
    #[test]
    fn the_sandbox_binds_no_root_and_no_workshops() {
        for mode in [SandboxMode::Strict, SandboxMode::Network] {
            let args = build_bwrap_args(mode, Path::new("/tmp/ws"), Path::new("/tmp/out"));
            let binds: Vec<&String> = args.windows(3).filter(|w| w[0].contains("bind")).map(|w| &w[1]).collect();
            assert!(!binds.iter().any(|b| *b == "/" || b.starts_with("/var") || b.starts_with("/run")), "{binds:?}");
        }
    }
}
