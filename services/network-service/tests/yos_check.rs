//! The service as the desktop runs it, held to the protocol by the OS's own checker: `network`
//! served on a private machine, and `yos check network` run against it.
//!
//! This is the test issue #179 is about. The socket used to answer `app.describe` with an empty
//! action list and not answer `app.act` at all, and the walk's `empty` and `unknown` rows name
//! exactly that — they can only pass when the published actions are real ones the dispatch
//! refuses, guards and runs. So the check runs against the served binary, not a mock of it.
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
/// ships (a `sensitive` ceiling, `ask` mode), and the service serving on it. The service reads
/// the real radio state wherever it runs, but the walk never reaches a handler (every probe is
/// refused before one), so this touches nothing outside its own directories.
struct Machine {
    root: PathBuf,
    service: Option<Child>,
}

impl Machine {
    fn new() -> Machine {
        let root = std::env::temp_dir().join(format!("network-service-check-{}", std::process::id()));
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
            .on_it(&mut Command::new(env!("CARGO_BIN_EXE_network-service")))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start network-service");
        self.service = Some(service);
        let socket = self.root.join("run/yantrik/network.sock");
        let deadline = Instant::now() + Duration::from_secs(20);
        while UnixStream::connect(&socket).is_err() {
            assert!(Instant::now() < deadline, "network-service never answered on {}", socket.display());
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

/// Every row the walk is expected to pass against this surface, deterministically, whatever
/// radio the machine has: `describe` is a timing budget and `steady` skips on live data, so
/// neither is named; `missing` answers honestly that no published action requires an argument.
const PASSED: [&str; 13] = [
    "ping", "protocol", "schema", "grades", "params", "secrets", "revision", "method", "empty",
    "unknown", "undeclared", "types", "stale",
];

#[test]
fn yos_check_holds_the_surface_to_the_protocol() {
    let Some(mut check) = yos() else {
        eprintln!("skipped: no yos on this machine (set YOS=/path/to/yos)");
        return;
    };
    let mut machine = Machine::new();
    machine.serve();
    let out = match machine.on_it(check.args(["check", "network", "--json"])).output() {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipped: `yos` was found but not the `python3` to run it");
            return;
        }
        Err(e) => panic!("run yos: {e}"),
    };
    let report: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!("yos check printed no JSON ({e}): {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
    });
    let rows = report["surfaces"][0]["checks"].as_array().expect("check rows");
    let failed: Vec<&Value> = rows.iter().filter(|r| r["status"] == "fail").collect();
    assert!(out.status.success() && report["ok"] == true, "yos check network failed: {failed:#?}");
    for name in PASSED {
        let row = rows.iter().find(|r| r["check"] == name).unwrap_or_else(|| panic!("no `{name}` row in {rows:#?}"));
        assert_eq!(row["status"], "pass", "`{name}`: {}", row["saw"]);
    }
}
