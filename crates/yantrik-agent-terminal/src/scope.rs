//! A Mind's command runs in a scope of its own, outside the person's login session (#1137).
//!
//! The shell and the person's own apps run in the login's `session-N.scope`. The DevTools and VNC
//! ports on loopback refuse every socket under the person's user manager (`user@UID.service`), and
//! let the session through. A command a Mind asked for is the Mind's, so it starts as
//! `systemd-run --user --scope --slice=yantrik-minds.slice --unit=mind-<job>`: the user manager
//! moves the process into a scope under its slice and `systemd-run` execs the command, so the pid
//! stays the job's session and process-group leader and everything else in this crate (the group
//! kill, the busy check on the terminal) is unchanged. Stopping the scope afterwards reaches what
//! left the group on purpose (`setsid`, a daemon).
//!
//! Without a user manager to ask (no `systemd-run`, no `XDG_RUNTIME_DIR`: a test, a machine
//! without systemd) the command starts directly, and says so in the log.

use std::path::Path;
use std::process::{Command, Stdio};

/// How a command is started: the program and its leading arguments, the variables only that
/// program needs, and the scope's unit name when there is one.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub unit: Option<String>,
}

/// The slice every Mind's job lives in, under the person's user manager.
pub(crate) const SLICE: &str = "yantrik-minds.slice";

/// What `systemd-run` needs or adds, and the command must not see: the way to the user manager,
/// and the unit identity systemd stamps on a process it starts.
const DROPPED: [&str; 3] = ["XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS", "INVOCATION_ID"];

/// The scope's unit name for a job.
pub(crate) fn unit_name(job: &str) -> String {
    let safe: String =
        job.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    format!("mind-{safe}.scope")
}

/// The launch for `bash` in job `job`, given what the host offers. A pure function of its inputs.
pub(crate) fn plan(
    job: &str,
    bash: &str,
    systemd_run: Option<&str>,
    runtime_dir: Option<&str>,
    bus: Option<&str>,
) -> Launch {
    let (Some(run), Some(dir)) = (systemd_run, runtime_dir.filter(|d| !d.is_empty())) else {
        return Launch { program: bash.to_string(), args: Vec::new(), env: Vec::new(), unit: None };
    };
    let unit = unit_name(job);
    let mut env = vec![("XDG_RUNTIME_DIR".to_string(), dir.to_string())];
    if let Some(bus) = bus.filter(|b| !b.is_empty()) {
        env.push(("DBUS_SESSION_BUS_ADDRESS".to_string(), bus.to_string()));
    }
    // `env -u` drops the two variables systemd-run needed before bash starts, so the command does
    // not inherit a way to talk to the user manager that it was not given.
    let args = [
        "--user",
        "--scope",
        "--quiet",
        "--collect",
        &format!("--slice={SLICE}"),
        &format!("--unit={}", unit.trim_end_matches(".scope")),
        "--",
        "/usr/bin/env",
    ]
    .iter()
    .map(|a| a.to_string())
    .chain(DROPPED.iter().flat_map(|v| ["-u".to_string(), v.to_string()]))
    .chain([bash.to_string()])
    .collect();
    Launch { program: run.to_string(), args, env, unit: Some(unit) }
}

/// The launch on this host.
pub(crate) fn launch(job: &str, bash: &str) -> Launch {
    let run = ["/usr/bin/systemd-run", "/bin/systemd-run"].into_iter().find(|p| Path::new(p).exists());
    let dir = std::env::var("XDG_RUNTIME_DIR").ok();
    let bus = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok();
    let launch = plan(job, bash, run, dir.as_deref(), bus.as_deref());
    if launch.unit.is_none() {
        tracing::warn!(job, "no user manager to ask: the command starts in this process's own cgroup");
    }
    launch
}

/// Kill whatever is left in the scope, without waiting for it. For the end of a job that was
/// killed: the group kill has been sent, this reaches what is no longer in the group.
pub(crate) fn stop(unit: &str) {
    let spawned = Command::new("systemctl")
        .args(["--user", "kill", "--kill-whom=all", "--signal=SIGKILL", unit])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        // Reaped off-thread so the call never blocks the job's worker and leaves no zombie.
        Ok(mut child) => {
            let _ = std::thread::Builder::new().name("scope-stop".into()).spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!(unit, error = %e, "could not stop the job's scope"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_goes_to_the_minds_slice_in_a_scope_of_its_own() {
        let l = plan(
            "job-ab12",
            "/bin/bash",
            Some("/usr/bin/systemd-run"),
            Some("/run/user/1000"),
            Some("unix:path=/run/user/1000/bus"),
        );
        assert_eq!(l.program, "/usr/bin/systemd-run");
        assert_eq!(l.unit.as_deref(), Some("mind-job-ab12.scope"));
        assert_eq!(&l.args[..4], ["--user", "--scope", "--quiet", "--collect"]);
        assert!(l.args.contains(&"--slice=yantrik-minds.slice".to_string()));
        assert!(l.args.contains(&"--unit=mind-job-ab12".to_string()));
        assert_eq!(l.args.last().map(String::as_str), Some("/bin/bash"));
        assert!(l.env.contains(&("XDG_RUNTIME_DIR".into(), "/run/user/1000".into())));
        assert!(l.env.iter().any(|(k, _)| k == "DBUS_SESSION_BUS_ADDRESS"));
    }

    #[test]
    fn the_command_is_not_handed_the_way_to_the_user_manager() {
        let l = plan("job-1", "/bin/bash", Some("/usr/bin/systemd-run"), Some("/run/user/1000"), None);
        let joined = l.args.join(" ");
        assert!(
            joined.contains("-- /usr/bin/env -u XDG_RUNTIME_DIR -u DBUS_SESSION_BUS_ADDRESS -u INVOCATION_ID /bin/bash"),
            "{joined}"
        );
        assert!(!l.env.iter().any(|(k, _)| k == "DBUS_SESSION_BUS_ADDRESS"));
    }

    #[test]
    fn without_a_user_manager_the_command_starts_directly() {
        for (run, dir) in [
            (None, Some("/run/user/1000")),
            (Some("/usr/bin/systemd-run"), None),
            (Some("/usr/bin/systemd-run"), Some("")),
        ] {
            let l = plan("job-1", "/bin/bash", run, dir, None);
            assert_eq!(l, Launch { program: "/bin/bash".into(), args: vec![], env: vec![], unit: None });
        }
    }

    #[test]
    fn a_unit_name_carries_nothing_a_unit_name_cannot() {
        assert_eq!(unit_name("job-ab/../12 x"), "mind-job-ab____12_x.scope");
    }
}
