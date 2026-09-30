//! The service as the desktop runs it, held to the protocol by the OS's own checker: `calendar`
//! served on a private machine, and `yos check calendar` run against it.
//!
//! This is the test issue #179 is about. The socket used to answer `app.describe` with an empty
//! action list and not answer `app.act` at all, and the walk's `empty` and `unknown` rows name
//! exactly that — they can only pass when the published actions are real ones the dispatch
//! refuses, guards and runs. So the check runs against the served binary, not a mock of it.
//! The second test then asks the CLI, over that same socket, to do the one thing the issue says
//! a caller could not: put an event on the calendar.
//!
//! `yos` is looked for where it lives: `$YOS` if set, `deploy/yantrik-os/yos` inside the
//! yantrik-os repository, `/opt/yantrik/bin/yos` on a Yantrik machine, then `PATH`. Where there
//! is none the test says so and passes without checking — on a Yantrik machine, and in the
//! repository's CI, there always is one.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use std::os::unix::net::UnixStream;

/// A private machine: `HOME` and `XDG_RUNTIME_DIR` of the test's own, with the settings this OS
/// ships (a `sensitive` ceiling, `ask` mode), and the service serving on it. The service's
/// calendar is under the scratch `$HOME` too, so nothing here reads or writes the developer's
/// events, and a test can assert on an empty calendar because it made the machine.
struct Machine {
    root: PathBuf,
    service: Option<Child>,
}

impl Machine {
    /// A directory of its own per machine: the two tests in this file run as threads of one
    /// process, and one machine's `Drop` removing the directory the other is serving on is a
    /// race, not a finding — so the name carries a counter as well as the pid.
    fn new() -> Machine {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "calendar-service-check-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let config = root.join("home/.config/yantrik");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(root.join("run")).unwrap();
        std::fs::write(config.join("settings.yaml"), "tool_permission: sensitive\n").unwrap();
        std::fs::write(config.join("mind-mode.json"), r#"{"mode":"ask","session_rules":[]}"#).unwrap();
        Machine { root, service: None }
    }

    fn on_it<'a>(&self, command: &'a mut Command) -> &'a mut Command {
        command.env("HOME", self.root.join("home")).env("XDG_RUNTIME_DIR", self.root.join("run"))
    }

    fn serve(&mut self) {
        let service = self
            .on_it(&mut Command::new(env!("CARGO_BIN_EXE_calendar-service")))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start calendar-service");
        self.service = Some(service);
        let socket = self.root.join("run/yantrik/calendar.sock");
        let deadline = Instant::now() + Duration::from_secs(20);
        while UnixStream::connect(&socket).is_err() {
            assert!(Instant::now() < deadline, "calendar-service never answered on {}", socket.display());
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        if let Some(mut service) = self.service.take() {
            let _ = service.kill();
            let _ = service.wait();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// How to run `yos` here, or `None`.
fn yos() -> Option<Command> {
    let in_repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/yantrik-os/yos");
    let candidates = [std::env::var_os("YOS").map(PathBuf::from), Some(in_repo), Some("/opt/yantrik/bin/yos".into())];
    for path in candidates.into_iter().flatten() {
        if path.is_file() {
            let mut command = Command::new("python3");
            command.arg(path);
            return Some(command);
        }
    }
    Command::new("yos").arg("--help").output().ok().map(|_| Command::new("yos"))
}

/// Run one `yos` command on this machine, or `None` where python3 to run it with is not here.
fn run(machine: &Machine, mut command: Command, args: &[&str]) -> Option<std::process::Output> {
    match machine.on_it(command.args(args)).output() {
        Ok(out) => Some(out),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipped: `yos` was found but not the `python3` to run it");
            None
        }
        Err(e) => panic!("run yos {args:?}: {e}"),
    }
}

/// Every row the walk is expected to pass against this surface, deterministically: `describe`
/// is a timing budget and `steady` skips on live data, so neither is named. `missing` is —
/// unlike network, this surface honestly does require arguments, and the dispatch must name
/// them before anything is stored.
const PASSED: [&str; 14] = [
    "ping", "protocol", "schema", "grades", "params", "secrets", "revision", "method", "empty",
    "unknown", "missing", "undeclared", "types", "stale",
];

#[test]
fn yos_check_holds_the_surface_to_the_protocol() {
    let Some(check) = yos() else {
        eprintln!("skipped: no yos on this machine (set YOS=/path/to/yos)");
        return;
    };
    let mut machine = Machine::new();
    machine.serve();
    let Some(out) = run(&machine, check, &["check", "calendar", "--json"]) else { return };
    let report: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!("yos check printed no JSON ({e}): {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
    });
    let rows = report["surfaces"][0]["checks"].as_array().expect("check rows");
    let failed: Vec<&Value> = rows.iter().filter(|r| r["status"] == "fail").collect();
    assert!(out.status.success() && report["ok"] == true, "yos check calendar failed: {failed:#?}");
    for name in PASSED {
        let row = rows.iter().find(|r| r["check"] == name).unwrap_or_else(|| panic!("no `{name}` row in {rows:#?}"));
        assert_eq!(row["status"], "pass", "`{name}`: {}", row["saw"]);
    }
}

/// The whole point of #179, from outside: a caller on the CLI adds an event through `app.act`,
/// and the next `app.describe` reports it. Before, `app.act` did not answer at all.
///
/// The dates are days out, not minutes: the event must sit inside the default week the service
/// lists without being asked — so no case depends on how long the machine takes to get there.
#[test]
fn the_cli_can_put_an_event_on_the_calendar() {
    let Some(act) = yos() else {
        eprintln!("skipped: no yos on this machine (set YOS=/path/to/yos)");
        return;
    };
    let mut machine = Machine::new();
    machine.serve();
    let now = chrono::Local::now().naive_local();
    let day = |n: i64| (now + chrono::Duration::days(n)).format("%Y-%m-%dT%H:%M:%S").to_string();
    let (start, end) = (format!("start={}", day(2)), format!("end={}", day(3)));
    let Some(out) = run(
        &machine,
        act,
        &["act", "calendar", "add_event", "title=Team Standup", &start, &end],
    ) else {
        return;
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "yos act failed: {stdout}{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("accepted: True, settled: True"), "the act did not settle: {stdout}");
    assert!(stdout.contains("1 events stored"), "no event counted after it: {stdout}");

    let describe = yos().expect("yos was found above");
    let Some(out) = run(&machine, describe, &["describe", "calendar"]) else { return };
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "yos describe failed: {stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("Team Standup"), "the calendar does not report the event: {stdout}");
}
