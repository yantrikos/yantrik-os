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

use std::os::fd::RawFd;
use std::path::{Path, PathBuf};
use std::os::unix::process::CommandExt;
use std::process::Command;

use yantrik_ipc_contracts::home_paths::workshops;

use crate::coder_script_view::ScriptView;

/// The person's home, as the views take it.
pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// The home of the account this process runs as, for the views: from the account entry, never from
/// `$HOME` alone, and an `Err` rather than a guess when the two disagree or there is none.
pub fn account_home() -> Result<PathBuf, String> {
    yantrik_ipc_contracts::home_paths::outside_view::account_home()
}

/// The directory a command starts in when the caller sets none: this process's own.
pub fn here() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| home())
}

/// `program args` as a command that runs in the outside view (`workshops::outside_command`), or
/// the reason it must not run: on a machine that holds workshops and has no `bwrap`, nothing is
/// started. The caller adds the working directory, the environment and the pipes.
///
/// The environment is the allow list every door uses (`view_env`): `--clearenv`, then `PATH`, `HOME`,
/// `LANG`/`LC_*`, `TERM` and `USER` of this process, then `extra`, the variables the caller's own
/// code (or the person's server config) names on purpose. Nothing else of the shell's passes.
pub fn mind_command(home: &Path, cwd: &Path, program: &str, args: &[String], extra: &[(String, String)]) -> Result<Command, String> {
    let env = yantrik_ipc_contracts::home_paths::view_env::process_env(home, extra);
    let (program, argv, held) = workshops::outside_command_with(home, cwd, &[], &env, program, args)?;
    let mut command = Command::new(program);
    command.args(argv).env_clear();
    // The view's own descriptors stay open across the exec and every other one is closed, as at the
    // other doors (`Held::seal`).
    // SAFETY: `seal` calls only async-signal-safe functions and allocates nothing.
    unsafe { command.pre_exec(move || held.seal(3)) };
    Ok(command)
}

/// Start a [`mind_command`] that outlives the call that starts it (a background task, an MCP
/// server). `--die-with-parent` ends the program when the *thread* that forked it exits, so it
/// is forked on the spawner thread, which lives as long as the process (round 13, M1). A command
/// that is waited for on the calling thread (`output()`) needs no such care.
pub fn spawn_long_lived(command: Command) -> std::io::Result<std::process::Child> {
    yantrik_ipc_contracts::home_paths::spawner::spawn(command)
}

/// Controls how isolated the script execution environment is. There is no mode with no sandbox: a script
/// that runs outside one reads every file the person can, the protected ones included, and the companion's
/// tools raise no card for it (review R11-H2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SandboxMode {
    /// Full isolation: no network, read-only filesystem, PID namespace.
    /// Default for code_execute and script_run.
    Strict,
    /// Filesystem isolation but network allowed.
    /// For web portals, API calls, data fetching scripts.
    Network,
}

/// What a request for no sandbox is answered with, in the rule's own words.
const NO_SANDBOX: &str = "Access denied: a script outside the sandbox reaches protected locations, and nothing asks the person first. Use 'strict' or 'network'.";

impl SandboxMode {
    /// The mode a caller asked for. `none` and `unsafe` are refused, not taken as `strict`.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "network" | "web" => Ok(Self::Network),
            "none" | "unsafe" => Err(NO_SANDBOX.to_string()),
            _ => Ok(Self::Strict),
        }
    }
}

/// Build bwrap arguments for sandboxed execution.
fn build_bwrap_args(
    sandbox: SandboxMode,
    workspace: &Path,
    output: &Path,
    output_fd: RawFd,
    reads: &[(PathBuf, PathBuf)],
    env: &[(String, String)],
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

    // The scripts folder is read-only: a script cannot write code (or anything) that a later script
    // imports or runs. Only the output folder, laid over it, is writable.
    args.extend(["--ro-bind".to_string(), workspace.to_string_lossy().into_owned(), workspace.to_string_lossy().into_owned()]);
    // The output folder is bound from an open descriptor, never by its path: a link planted at the path
    // is not followed (`coder_script_view` opened it without following one).
    args.extend(["--bind-fd".to_string(), output_fd.to_string(), output.to_string_lossy().into_owned()]);

    // Read-only places laid over the workspace (the dependencies folders): a script imports from them
    // and cannot write them. Every one is bound, always: a source that is missing makes the run fail
    // (the caller makes them first), it never leaves a writable folder of the workspace in its place.
    for (source, dest) in reads {
        args.extend(["--ro-bind".to_string(), source.to_string_lossy().into_owned(), dest.to_string_lossy().into_owned()]);
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
    }

    // Die when parent dies (prevent orphan processes)
    args.push("--die-with-parent".into());

    // No capabilities, so the read-only binds cannot be unmounted or remounted from inside, and a new
    // session, so the script has no controlling terminal to push input into.
    args.extend(["--cap-drop".to_string(), "ALL".to_string(), "--new-session".to_string()]);

    // The environment is the one allow list (`view_env`): `--clearenv` first, then each name.
    args.extend(yantrik_ipc_contracts::home_paths::view_env::words(env));

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
pub fn sandboxed_script(
    sandbox: SandboxMode,
    workspace: &Path,
    output: &Path,
    view: ScriptView,
    script: &str,
) -> Command {
    let ScriptView { reads, extra, held, output_fd } = view;
    // The shell's environment never reaches the script (API keys, the agent socket, the session bus):
    // `bwrap` itself gets an empty one, and the script's is the allow list with `HOME` a stand-in
    // (the sandbox's own `/tmp`, so dotfiles and caches never land in the real home), then the
    // door's own named variables in `extra`. `extra` also holds the Mind's own `script_run` env (after
    // the drop list of `coder_script_view::mind_env`, so no loader name and no `PATH`); it acts only
    // inside this sandbox, on the Mind's own script, and the door's names come after it and win.
    let env = yantrik_ipc_contracts::home_paths::view_env::process_env(Path::new("/tmp"), &extra);
    let mut command = Command::new("/usr/bin/bwrap");
    command.env_clear();
    command.args(build_bwrap_args(sandbox, workspace, output, output_fd, &reads, &env));
    command.arg("sh").arg("-c").arg(script);
    // The output folder's descriptor stays open across the exec and every other one is closed.
    // SAFETY: `seal` calls only async-signal-safe functions and allocates nothing.
    unsafe { command.pre_exec(move || held.seal(3)) };
    command
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn no_view() -> ScriptView {
        ScriptView { reads: vec![], extra: vec![], held: Default::default(), output_fd: 7 }
    }

    fn argv(c: &Command) -> Vec<String> {
        c.get_args().map(OsStr::to_string_lossy).map(|s| s.into_owned()).collect()
    }

    /// `mind_command` is `outside_command`'s answer, not a parallel construction.
    #[test]
    fn mind_command_is_outside_commands_answer() {
        let home = home();
        let args = vec!["-c".to_string(), "x".to_string()];
        let cwd = home.clone();
        match (mind_command(&home, &cwd, "tool", &args, &[]), workshops::outside_command(&home, &cwd, &[], "tool", &args)) {
            (Ok(c), Ok((program, want, _))) => {
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
        let strict = sandboxed_script(SandboxMode::Strict, ws, out, no_view(), "exec python3 x");
        assert_eq!(strict.get_program(), "/usr/bin/bwrap");
        let args = argv(&strict);
        assert_eq!(args[args.len() - 3..], ["sh", "-c", "exec python3 x"]);
        let env = yantrik_ipc_contracts::home_paths::view_env::process_env(Path::new("/tmp"), &[]);
        assert_eq!(args[..args.len() - 3], build_bwrap_args(SandboxMode::Strict, ws, out, 7, &[], &env));
        assert!(args.contains(&"--unshare-net".to_string()) && !args.contains(&"--share-net".to_string()));
        let net = argv(&sandboxed_script(SandboxMode::Network, ws, out, no_view(), "x"));
        assert!(net.contains(&"--share-net".to_string()));
    }

    /// Nothing in the sandbox's args binds the root, the workshops base or `/run/user`.
    #[test]
    fn the_sandbox_binds_no_root_and_no_workshops() {
        for mode in [SandboxMode::Strict, SandboxMode::Network] {
            let args = build_bwrap_args(mode, Path::new("/tmp/ws"), Path::new("/tmp/out"), 7, &[], &[]);
            let binds: Vec<&String> = args.windows(3).filter(|w| w[0].contains("bind")).map(|w| &w[1]).collect();
            assert!(!binds.iter().any(|b| *b == "/" || b.starts_with("/var") || b.starts_with("/run")), "{binds:?}");
        }
    }

    /// Round 10 B: `bwrap` itself starts with an empty environment and the script's is `--clearenv`
    /// plus the allow list: a key, the agent socket and the session bus address are not in it.
    #[test]
    fn the_script_sandbox_clears_the_environment() {
        std::env::set_var("YANTRIK_TEST_KEY_R10", "hunter2");
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1/bus");
        let c = sandboxed_script(SandboxMode::Strict, Path::new("/tmp/ws"), Path::new("/tmp/out"), ScriptView { extra: vec![("YANTRIK_OUTPUT_DIR".into(), "/o".into())], ..no_view() }, "x");
        std::env::remove_var("YANTRIK_TEST_KEY_R10");
        std::env::remove_var("DBUS_SESSION_BUS_ADDRESS");
        let a = argv(&c);
        assert!(c.get_envs().next().is_none(), "bwrap's own environment is empty (env_clear)");
        assert_eq!(a.iter().filter(|w| *w == "--clearenv").count(), 1);
        let set: Vec<&String> = a.windows(3).filter(|w| w[0] == "--setenv").map(|w| &w[1]).collect();
        assert!(set.iter().any(|k| *k == "YANTRIK_OUTPUT_DIR") && set.iter().any(|k| *k == "PATH"), "{set:?}");
        assert!(!a.iter().any(|w| w.contains("hunter2") || w.contains("DBUS") || w.contains("YANTRIK_TEST_KEY")), "{a:?}");
        let home: Vec<&String> = a.windows(3).filter(|w| w[0] == "--setenv" && w[1] == "HOME").map(|w| &w[2]).collect();
        assert_eq!(home, ["/tmp"]);
        assert!(a.iter().position(|w| w == "--clearenv").unwrap() < a.iter().position(|w| w == "--setenv").unwrap());
    }

    /// Rounds 10 and 11: the workspace is read-only, the output folder is the one writable place over it,
    /// and the dependencies folders are laid over it read-only after both, a missing source included.
    #[test]
    fn the_dependencies_are_bound_read_only_over_the_workspace() {
        let top = std::env::temp_dir().join(format!("yantrik-r10-reads-{}", std::process::id()));
        let (ws, out) = (top.join("ws"), top.join("ws/output"));
        std::fs::create_dir_all(top.join("deps/python")).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let t = |p: &Path| p.to_str().unwrap().to_string();
        let reads = vec![(top.join("deps/python"), ws.join(".deps")), (top.join("deps/missing"), ws.join("node_modules"))];
        let a = build_bwrap_args(SandboxMode::Strict, &ws, &out, 9, &reads, &[]);
        let at = |w: &[String]| a.windows(w.len()).position(|x| x == w);
        let ws_ro = at(&["--ro-bind".into(), t(&ws), t(&ws)]).expect("the workspace is bound read-only");
        assert!(at(&["--bind".into(), t(&ws), t(&ws)]).is_none(), "no writable bind of the workspace");
        let out_rw = at(&["--bind-fd".into(), "9".into(), t(&out)]).expect("the output folder is bound from its descriptor");
        assert!(at(&["--bind".into(), t(&out), t(&out)]).is_none(), "never by its path");
        let ro = at(&["--ro-bind".into(), t(&top.join("deps/python")), t(&ws.join(".deps"))]).expect("ro-bind of the python deps");
        assert!(ro > out_rw && out_rw > ws_ro, "deps after output after workspace");
        assert!(at(&["--ro-bind".into(), t(&top.join("deps/missing")), t(&ws.join("node_modules"))]).is_some(), "a bind is never skipped");
        assert!(a.windows(2).any(|w| w == ["--cap-drop", "ALL"]) && a.contains(&"--new-session".to_string()));
        let _ = std::fs::remove_dir_all(top);
    }
}
