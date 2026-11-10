//! The port readers against real Chromiums and the real /proc (#1085). Skips, saying so, where there
//! is no Chromium-family browser. A real packaged Chromium's command line has shown up as NUL
//! separated, as one space-joined string, and as that with a trailing NUL; the readers do not look
//! at it. They ask which profile a process owns (its `SingletonLock`), which a real Chromium writes.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use yantrik_browser_ports::{listener, procs, Place, Profiles};

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

fn start(binary: &Path, home: &Path, profile: Option<&Path>, profiles: &Profiles) -> Browser {
    let port = free_port();
    let mut cmd = Command::new(binary);
    cmd.args(["--headless=new", "--no-first-run", "--no-default-browser-check", "--disable-gpu", "--no-sandbox", "--disable-background-networking", "--disable-component-update"])
        .arg("--remote-debugging-address=127.0.0.1")
        .arg(format!("--remote-debugging-port={port}"))
        .env("HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(p) = profile {
        cmd.arg(format!("--user-data-dir={}", p.display()));
    }
    cmd.arg("about:blank");
    let browser = Browser { child: cmd.spawn().expect("start the browser"), port };
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if listener::identify_in(Path::new("/proc"), port, profiles).is_ok() {
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
    let profiles = Profiles { mind_view: home.join("data/mind-view/chromium"), identities: home.join("data/browsers") };

    let mind = start(&binary, &home, Some(&profiles.mind_view), &profiles);
    let person_default = start(&binary, &home, None, &profiles);
    let person_named = start(&binary, &home, Some(&home.join("person-profile")), &profiles);

    let held = listener::identify_in(Path::new("/proc"), mind.port, &profiles).unwrap();
    assert_eq!(held.place, Place::MindView, "{held:?}");
    assert_eq!(held.uid, listener::own_uid());
    let line = std::fs::read(format!("/proc/{}/cmdline", held.pids[0])).unwrap();
    eprintln!("a real main process reads back as: {:?}", String::from_utf8_lossy(&line));
    for person in [&person_default, &person_named] {
        let l = listener::identify_in(Path::new("/proc"), person.port, &profiles).unwrap();
        assert_eq!(l.place, Place::Person, "{l:?}");
    }

    // Mind View's browser is on a port that is not the person's: it is left running. Moved onto
    // the person's port (it holds that port's socket), it is the one thing the update stops.
    let all = procs::snapshot();
    let mains: Vec<u32> = procs::mind_side_mains(&all, listener::own_uid(), &profiles).iter().map(|(p, _)| p.pid).collect();
    assert!(mains.iter().any(|p| held.pids.contains(p)), "{mains:?} vs {:?}", held.pids);
    for person in [&person_default, &person_named] {
        let theirs = listener::identify_in(Path::new("/proc"), person.port, &profiles).unwrap().pids;
        assert!(procs::mind_view_off_port(&all, listener::own_uid(), &profiles, &theirs).is_empty());
    }
    let off = procs::mind_view_off_port(&all, listener::own_uid(), &profiles, &held.pids);
    assert!(!off.is_empty());
    let family = procs::with_descendants(&all, &off);
    assert!(procs::signal(&family, "-15") > 0);
    let deadline = Instant::now() + Duration::from_secs(20);
    while listener::identify_in(Path::new("/proc"), mind.port, &profiles).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(listener::identify_in(Path::new("/proc"), mind.port, &profiles).is_err(), "the mind's browser should have stopped");
    for person in [&person_default, &person_named] {
        assert!(listener::identify_in(Path::new("/proc"), person.port, &profiles).is_ok(), "the person's was touched");
    }
    let _ = std::fs::remove_dir_all(&home);
}
