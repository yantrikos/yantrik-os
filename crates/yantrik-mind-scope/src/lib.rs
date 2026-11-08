//! What a Mind asked for runs in a scope of its own, outside the person's login session (#1137).
//!
//! The shell and the person's own apps run in the login's `session-N.scope`. The DevTools and VNC
//! ports on loopback refuse every socket under the person's user manager (`user@UID.service`), and
//! let the session through. A process a Mind asked for (an `agent_run` job, a companion tool's
//! script, an app a Mind opens) is the Mind's, so it starts as
//! `systemd-run --user --scope --slice=yantrik-minds.slice --unit=mind-<id>`: the user manager moves
//! the process into a scope under its slice and `systemd-run` execs the program, so the pid stays
//! the caller's child and everything the caller does with it (wait, group kill) is unchanged.
//! Stopping the scope afterwards reaches what left the group on purpose (`setsid`, a daemon).
//!
//! There is no quiet fallback. On a host that runs systemd, a launch that cannot get a scope is
//! refused with a sentence, because starting it in the session would put the Mind exactly where
//! the rule lets it through. Only a host without systemd starts the program directly.

use std::path::Path;
use std::process::{Command, Stdio};

/// The slice every Mind's process lives in, under the person's user manager.
pub const SLICE: &str = "yantrik-minds.slice";

/// What `systemd-run` needs or adds, and a stripped program must not see: the way to the user
/// manager, and the unit identity systemd stamps on a process it starts.
const DROPPED: [&str; 3] = ["XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "INVOCATION_ID"];

/// What the host offers, for [`plan`]. Gathered by [`Host::here`]; built by hand in a test.
#[derive(Debug, Default, Clone)]
pub struct Host {
    /// Runs systemd as init: a scope is expected, so not getting one is a refusal.
    pub systemd: bool,
    pub systemd_run: Option<String>,
    pub runtime_dir: Option<String>,
    pub bus: Option<String>,
}

impl Host {
    /// This machine. `YANTRIK_MIND_SCOPE=off` is the explicit opt-out for a host (a container, a
    /// test box) that has systemd's files but no user manager; it is read from the shell's own
    /// environment, which a Mind does not write.
    pub fn here() -> Host {
        let off = std::env::var("YANTRIK_MIND_SCOPE").is_ok_and(|v| v == "off");
        Host {
            systemd: !off && Path::new("/run/systemd/system").exists(),
            systemd_run: ["/usr/bin/systemd-run", "/bin/systemd-run"]
                .into_iter()
                .find(|p| Path::new(p).exists())
                .map(String::from),
            runtime_dir: std::env::var("XDG_RUNTIME_DIR").ok(),
            bus: std::env::var("DBUS_SESSION_BUS_ADDRESS").ok(),
        }
    }
}

/// How a program is started: what to run, its leading arguments, the variables only `systemd-run`
/// needs, and the scope's unit name when there is one.
#[derive(Debug, PartialEq, Eq)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub unit: Option<String>,
}

/// The scope's unit name for an id.
pub fn unit_name(id: &str) -> String {
    let safe: String =
        id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    format!("mind-{safe}.scope")
}

/// An id no other launch has: `<prefix>-<nanoseconds>`.
pub fn fresh_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{prefix}-{nanos}")
}

/// The launch for `program` as unit `id`, given what the host offers. A pure function of its
/// inputs. `strip` hides the way to the user manager from the program (a Mind's script); an app
/// needs its runtime directory, so it keeps it.
pub fn plan(id: &str, program: &str, host: &Host, strip: bool) -> Result<Launch, String> {
    let dir = host.runtime_dir.as_deref().filter(|d| !d.is_empty());
    let (Some(run), Some(dir)) = (host.systemd_run.as_deref(), dir) else {
        if host.systemd {
            return Err("It was not started: this machine runs systemd, but there is no user manager to \
                        put it in its own scope (no systemd-run, or XDG_RUNTIME_DIR is not set). Run in \
                        the person's session, it could reach their browser."
                .to_string());
        }
        return Ok(Launch { program: program.to_string(), args: Vec::new(), env: Vec::new(), unit: None });
    };
    let unit = unit_name(id);
    let mut env = vec![("XDG_RUNTIME_DIR".to_string(), dir.to_string())];
    if let Some(bus) = host.bus.as_deref().filter(|b| !b.is_empty()) {
        env.push(("DBUS_SESSION_BUS_ADDRESS".to_string(), bus.to_string()));
    }
    let mut args: Vec<String> = [
        "--user",
        "--scope",
        "--quiet",
        "--collect",
        &format!("--slice={SLICE}"),
        &format!("--unit={}", unit.trim_end_matches(".scope")),
        "--",
    ]
    .iter()
    .map(|a| a.to_string())
    .collect();
    if strip {
        // `env -u` drops what systemd-run needed before the program starts, so it does not
        // inherit a way to talk to the user manager that it was not given.
        args.push("/usr/bin/env".to_string());
        args.extend(DROPPED.iter().flat_map(|v| ["-u".to_string(), v.to_string()]));
    }
    args.push(program.to_string());
    Ok(Launch { program: run.to_string(), args, env, unit: Some(unit) })
}

/// The launch on this host, or the sentence that says why there is none.
pub fn launch(id: &str, program: &str, strip: bool) -> Result<Launch, String> {
    plan(id, program, &Host::here(), strip)
}

/// A [`Command`] for `program` in its own scope, with its unit name when it has one. The caller
/// adds the program's arguments, environment and directory as for any other command.
pub fn command(id: &str, program: &str, strip: bool) -> Result<(Command, Option<String>), String> {
    let launch = launch(id, program, strip)?;
    let mut command = Command::new(&launch.program);
    command.args(&launch.args);
    command.envs(launch.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    Ok((command, launch.unit))
}

/// `command` (program, arguments, environment, directory) started in a scope of its own instead.
/// For a caller that has already built the command the person's way and then learns a Mind asked.
pub fn wrap(command: &Command, id: &str, strip: bool) -> Result<(Command, Option<String>), String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let (mut wrapped, unit) = self::command(id, &program, strip)?;
    wrapped.args(command.get_args());
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => wrapped.env(key, value),
            None => wrapped.env_remove(key),
        };
    }
    if let Some(dir) = command.get_current_dir() {
        wrapped.current_dir(dir);
    }
    Ok((wrapped, unit))
}

/// Kill whatever is left in the scope, without waiting for it. For the end of a job that was
/// killed: the group kill has been sent, this reaches what is no longer in the group.
pub fn stop(unit: &str) {
    let spawned = Command::new("systemctl")
        .args(["--user", "kill", "--kill-whom=all", "--signal=SIGKILL", unit])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        // Reaped off-thread so the call never blocks the caller and leaves no zombie.
        Ok(mut child) => {
            let _ = std::thread::Builder::new().name("scope-stop".into()).spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!(unit, error = %e, "could not stop the scope"),
    }
}

#[cfg(test)]
mod tests;
