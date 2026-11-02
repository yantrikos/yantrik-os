//! The port readers against real Chromiums and the real /proc (#1085). Skips, saying so, where there
//! is no Chromium-family browser or no /proc. The unit tests use a fake /proc; this is what the
//! 520 gate showed they would miss: a real Chromium main process rewrites its command line into one
//! space-joined string, and a reader that splits on NUL takes every browser for the default profile.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use yantrik_browser_ports::{listener, procs, Place};

fn chrome() -> Option<PathBuf> {
    ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"].iter().find_map(|n| {
        let out = Command::new("which").arg(n).output().ok()?;
        out.status.success().then(|| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
    })
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

struct Browser {
    child: Child,
    port: u16,
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(binary: &Path, home: &Path, profile: Option<&Path>) -> Browser {
    let port = free_port();
    let mut cmd = Command::new(binary);
    cmd.args(["--headless=new", "--no-first-run", "--no-default-browser-check", "--disable-gpu", "--no-sandbox", "--disable-background-networking", "--disable-component-update"])
        .arg("--remote-debugging-address=127.0.0.1")
        .arg(format!("--remote-debugging-port={port}"))
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(p) = profile {
        cmd.arg(format!("--user-data-dir={}", p.display()));
    }
    cmd.arg("about:blank");
    let child = cmd.spawn().expect("start the browser");
    let browser = Browser { child, port };
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if listener::identify(port).is_ok() {
            return browser;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("the browser did not listen on {port} in 90 s");
}

#[test]
fn a_real_chromium_is_told_by_its_profile_and_the_wrong_ones_are_never_stopped() {
    let Some(binary) = chrome() else {
        eprintln!("skipped: no Chromium-family browser on PATH");
        return;
    };
    let home = std::env::temp_dir().join(format!("yb-ports-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // Everything below reads the environment of this process the way the shell does.
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_DATA_HOME", home.join(".local/share"));
    std::env::set_var("XDG_CONFIG_HOME", home.join(".config"));

    let mind = start(&binary, &home, Some(&yantrik_browser_ports::mind_view_profile()));
    let person_default = start(&binary, &home, None);
    let person_spelled = start(&binary, &home, Some(&home.join(".config/chromium-person")));

    let held = listener::identify(mind.port).unwrap();
    assert_eq!(held.place, Place::MindView, "{held:?}");
    assert_eq!(held.uid, listener::own_uid());
    assert!(held.pids.contains(&mind.child.id()) || !held.pids.is_empty());
    // The reason this test exists: the holder's real command line has no NUL in it.
    let raw = std::fs::read(format!("/proc/{}/cmdline", held.pids[0])).unwrap();
    eprintln!("a real main process reads back as: {}", String::from_utf8_lossy(&raw));
    assert!(listener::mind_side(mind.port).is_ok());

    assert_eq!(listener::identify(person_default.port).unwrap().place, Place::PersonDefault);
    assert_eq!(listener::identify(person_spelled.port).unwrap().place, Place::Other);
    for person in [&person_default, &person_spelled] {
        let why = listener::mind_side(person.port).unwrap_err();
        assert!(why.contains("not a mind's own"), "{why}");
    }

    // The mind's browser is on a port that is not Mind View's: the update stops exactly it.
    let all = procs::snapshot();
    let mains: Vec<u32> = procs::mind_side_mains(&all, listener::own_uid()).iter().map(|p| p.pid).collect();
    assert!(mains.iter().any(|p| held.pids.contains(p)), "{mains:?} vs {:?}", held.pids);
    for person in [&person_default, &person_spelled] {
        assert!(!mains.contains(&person.child.id()) && !mains.contains(&listener::identify(person.port).unwrap().pids[0]));
    }
    assert!(procs::stop_mind_view_off_port() > 0);
    let deadline = Instant::now() + Duration::from_secs(20);
    while listener::identify(mind.port).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(listener::identify(mind.port).is_err(), "the mind's browser should have stopped");
    // Nothing of the person's was touched.
    assert!(listener::identify(person_default.port).is_ok() && listener::identify(person_spelled.port).is_ok());
    let _ = std::fs::remove_dir_all(&home);
}
