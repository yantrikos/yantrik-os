//! One window per app.
//!
//! The launcher, the taskbar pins and the companion can all ask for "notes" within the same
//! minute, and without this each request would spawn another process with another window.
//! The guard is a pid file under the runtime dir: the first process writes its pid, later ones
//! find a live pid there, ask that process to show its window (`present`), and exit.
//!
//! Exiting alone was not enough. The shell raises a window only if its compositor lists one, and
//! a copy a mind opened is drawn in Mind View, where the person's compositor never sees it: the
//! dock answered "accepted", the second copy left, and nothing appeared. The first copy owns the
//! window and is the only process that can show it, so it is asked, and what it answered is said.
//!
//! A stale file (the previous instance crashed) is detected by checking that the pid is still
//! alive, so a crash never locks the user out of an app until reboot.
//!
//! The pid file alone was check-then-create: two launches in one instant both found no live pid,
//! and the second unlinked the first's fresh file before making its own, so both ran. The slot is
//! now an exclusive `flock` on `<app>.lock` beside it ([`crate::lock`]), atomic and released on any
//! death. The pid file is still written and still read, so a copy from a build before the lock,
//! which knows only the pid file, is seen, and sees this one, across an upgrade.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// Held for the life of the process; removes the pid file on drop, and the lock goes with it.
pub struct InstanceGuard {
    path: PathBuf,
    _lock: Option<crate::lock::FileLock>,
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// The session's socket directory, where the pid files sit beside the sockets.
///
/// It must not vary per process, or every launch would get its own pid file and the guard would
/// never see the previous instance. This used to fall back to `/tmp/yantrik-$USER` when there was
/// no runtime dir, made with a plain `create_dir_all` and never checked: any account could make
/// that directory first and own every pid file in it. `socket_dir` is the directory the sockets
/// already trust, and it refuses a candidate that is a link or someone else's.
#[cfg(unix)]
fn runtime_dir() -> PathBuf {
    yantrik_ipc_transport::server::socket_dir()
}

/// Windows dev builds: the profile's temp dir is per user already.
#[cfg(not(unix))]
fn runtime_dir() -> PathBuf {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    let p = std::env::temp_dir().join(format!("yantrik-{user}"));
    let _ = fs::create_dir_all(&p);
    p
}

/// Whether `dir` is a real directory owned by us that no one else can write: what `socket_dir`
/// makes of every candidate it accepts.
#[cfg(unix)]
fn private_to_us(dir: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid cannot fail and touches no memory.
    let me = unsafe { libc::geteuid() };
    fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir() && m.uid() == me && m.mode() & 0o022 == 0)
}

#[cfg(not(unix))]
fn private_to_us(_dir: &std::path::Path) -> bool {
    true
}

fn pid_alive(pid: u32) -> bool {
    // On Linux a live process has a /proc entry — but so does a ZOMBIE, and the shell that
    // launched us may not have reaped our predecessor yet. A zombie holds no window and no
    // socket; treating it as "running" locked the user out of the app until the shell exited.
    // /proc/<pid>/stat's state field is 'Z' for a zombie: `pid (comm) State ...`, and comm
    // may itself contain spaces or parens, so parse from the LAST ')'.
    if !cfg!(target_os = "linux") {
        return true;
    }
    let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some(close) = stat.rfind(')') else {
        return false;
    };
    let state = stat[close + 1..].trim_start().chars().next().unwrap_or('Z');
    !matches!(state, 'Z' | 'X')
}

/// Claim the single-instance slot for `app_name`.
///
/// Returns `None` when another live instance holds it, once that instance has been asked to show
/// its window. The caller exits, successfully: the launch was handed over, not refused.
pub fn claim(app_name: &str) -> Option<InstanceGuard> {
    claim_in(&runtime_dir(), app_name, present_running)
}

/// What a launch that found the slot held made of the copy holding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// The running copy took the launch (or at least answered); this process exits.
    HandedOver,
    /// It did not answer: it may have died with its pid file behind it. The slot is checked
    /// again, once, and this process may become the owner.
    OwnerGone,
}

/// [`claim`], handing the launch to the running copy through `hand_over` instead of asking it
/// only to present. A copy that does not answer is given one re-check of the slot (the design's
/// §2.2 step 4): a dead owner's stale pid file is taken over, a live one that is stuck is left.
pub fn claim_or_hand_over(
    app_name: &str,
    mut hand_over: impl FnMut(&str) -> Held,
) -> Option<InstanceGuard> {
    let dir = runtime_dir();
    for attempt in 0..2 {
        let mut gone = false;
        let claimed = claim_in(&dir, app_name, |name| {
            gone = hand_over(name) == Held::OwnerGone;
        });
        match claimed {
            Some(guard) => return Some(guard),
            None if gone && attempt == 0 => {
                tracing::warn!(app = app_name, "the running copy did not answer; checking the slot again");
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            None => return None,
        }
    }
    None
}

/// Ask the running copy of `app_name` to show its window, and say what came of it.
///
/// For a launch that found the app already running by a guard of its own, as Notes does; `claim`
/// calls it for every other app.
pub fn present_running(app_name: &str) {
    let answered = crate::present::ask(app_name);
    match answered.sentence(app_name) {
        None => tracing::info!(
            app = app_name,
            "Already running; asked it to show its window, and it did"
        ),
        Some(said) => {
            tracing::warn!(app = app_name, answer = ?answered, "{said}");
            // Also on stderr, for a person who started it from a terminal and reads no log.
            eprintln!("{said}");
        }
    }
}

/// [`claim`] in `dir`, with `held` run when another live instance has the slot. Split out so a
/// test can hold the slot in a directory of its own and see who was asked.
fn claim_in(
    dir: &std::path::Path,
    app_name: &str,
    held: impl FnOnce(&str),
) -> Option<InstanceGuard> {
    let dir = dir.to_path_buf();
    if !private_to_us(&dir) {
        // `socket_dir` hands back its last candidate unchecked when it could prepare none, and a
        // directory that may be someone else's is no place to keep a file whose contents decide
        // whether we run. Running unguarded risks a second window; trusting it risks worse.
        tracing::warn!(app = app_name, dir = %dir.display(), "Pid directory is not private to us; running unguarded");
        return Some(InstanceGuard {
            path: PathBuf::new(),
            _lock: None,
        });
    }
    // The slot itself: atomic, so two launches in one instant cannot both take it.
    let lock = match crate::lock::hold(&dir.join(format!("{app_name}.lock"))) {
        Ok(lock) => Some(lock),
        Err(crate::lock::LockError::Held) => {
            tracing::info!(app = app_name, "Already running; handing this launch to it");
            held(app_name);
            return None;
        }
        Err(crate::lock::LockError::Io(e)) => {
            // The pid file below still guards; only the instant-race protection is lost.
            tracing::warn!(app = app_name, error = %e, "Could not take the instance lock; the pid file alone guards");
            None
        }
    };
    let path = dir.join(format!("{app_name}.pid"));

    if let Ok(existing) = fs::read_to_string(&path) {
        if let Ok(pid) = existing.trim().parse::<u32>() {
            if pid != std::process::id() && pid_alive(pid) {
                tracing::info!(
                    app = app_name,
                    pid,
                    "Already running; handing this launch to it"
                );
                held(app_name);
                return None;
            }
        }
        // Stale: the recorded pid is gone. Fall through and take over.
    }

    // Unlink, then create only if absent: a plain create follows a link left at the name, and a
    // `notes.pid` pointing at ~/.ssh/authorized_keys would have been emptied by the next launch.
    // `create_new` (O_EXCL) never follows one. Under the lock no copy of this build can race the
    // gap; the pid file is kept for builds that know only it.
    let _ = fs::remove_file(&path);
    let created = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path);
    match created.and_then(|mut f| write!(f, "{}", std::process::id())) {
        Ok(()) => Some(InstanceGuard { path, _lock: lock }),
        Err(e) => {
            // Not being able to write the pid file is not a reason to refuse to run.
            if lock.is_some() {
                tracing::warn!(app = app_name, error = %e, "Could not write pid file; the lock alone guards");
            } else {
                tracing::warn!(app = app_name, error = %e, "Could not write pid file or take the lock; running unguarded");
            }
            Some(InstanceGuard {
                path: PathBuf::new(),
                _lock: lock,
            })
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    /// Set in the environment of the copy [`a_second_launch_asks_the_first_to_present_and_exits_0`]
    /// starts, which runs [`second_launch`] as a launch of its own.
    const CHILD: &str = "YANTRIK_TEST_SECOND_LAUNCH";

    /// A runtime directory of this test's own, as private as `socket_dir` makes the real one.
    fn private_dir(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("yantrik-instance-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("yantrik")).unwrap();
        fs::set_permissions(d.join("yantrik"), fs::Permissions::from_mode(0o700)).unwrap();
        d
    }

    /// A live process that is not this one, to hold the slot: `claim` takes its own pid in the
    /// file as no holder at all.
    fn holder() -> std::process::Child {
        std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep")
    }

    #[test]
    fn a_held_slot_hands_the_launch_over_and_leaves_the_holder_in_place() {
        let dir = private_dir("held").join("yantrik");
        let mut first = holder();
        fs::write(dir.join("calendar.pid"), first.id().to_string()).unwrap();

        let mut asked = Vec::new();
        let guard = claim_in(&dir, "calendar", |app| asked.push(app.to_string()));

        assert!(
            guard.is_none(),
            "a second copy was let run beside the first"
        );
        assert_eq!(
            asked,
            ["calendar"],
            "the running copy was not asked to show its window"
        );
        assert_eq!(
            fs::read_to_string(dir.join("calendar.pid")).unwrap(),
            first.id().to_string()
        );
        let _ = first.kill();
        let _ = first.wait();
    }

    #[test]
    fn two_launches_in_one_instant_do_not_both_run() {
        // The first launch holds the slot; its pid file names this very process, which the pid
        // check alone takes as no holder. Only the lock stops the second.
        let dir = private_dir("instant").join("yantrik");
        let first = claim_in(&dir, "notes", |_| panic!("the first launch found the slot held"));
        assert!(first.is_some());
        let mut asked = false;
        let second = claim_in(&dir, "notes", |_| asked = true);
        assert!(second.is_none(), "a second copy ran beside the first");
        assert!(asked, "the running copy was asked to show its window");
        drop(first);
        assert!(
            crate::lock::soon(|| claim_in(&dir, "notes", |_| {}).is_some()),
            "released with the guard"
        );
    }

    #[test]
    fn a_free_slot_is_taken_and_nobody_is_asked() {
        let dir = private_dir("free").join("yantrik");
        let mut asked = false;
        let guard = claim_in(&dir, "calendar", |_| asked = true);
        assert!(guard.is_some() && !asked);
        assert_eq!(
            fs::read_to_string(dir.join("calendar.pid")).unwrap(),
            std::process::id().to_string()
        );
    }

    /// The launch itself, as a process: `claim` as an app's `main` calls it, and the exit status
    /// that follows. Does nothing unless started by the test below.
    #[test]
    fn second_launch() {
        if std::env::var_os(CHILD).is_none() {
            return;
        }
        match claim("probe") {
            None => std::process::exit(0),
            // A second copy running: the one outcome this guard exists to prevent.
            Some(_) => std::process::exit(3),
        }
    }

    /// The bug, end to end at the runtime's edge: the first copy holds the slot and serves its
    /// surface; a second launch, a real process, asks it to present and exits 0 without ever
    /// running as a second copy.
    #[test]
    fn a_second_launch_asks_the_first_to_present_and_exits_0() {
        let runtime = private_dir("launch");
        let dir = runtime.join("yantrik");
        // This process is the first copy: alive, and not the launch.
        fs::write(dir.join("probe.pid"), std::process::id().to_string()).unwrap();

        // Its surface, reduced to the one call a second launch makes. The launch connects once to
        // see that the socket is up before it asks, so a connection may close without a line.
        let listener = UnixListener::bind(dir.join("app-probe.sock")).unwrap();
        let (asked, requests) = std::sync::mpsc::channel::<serde_json::Value>();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                let reply = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "result": { "accepted": true, "result": { "window": true, "presented": true,
                                "display": "wayland-test", "elsewhere": false } },
                });
                let _ = (&stream).write_all(format!("{reply}\n").as_bytes());
                let _ = asked.send(request);
            }
        });

        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "instance::tests::second_launch",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("WAYLAND_DISPLAY", "wayland-test")
            .env_remove("YANTRIK_MIND_RUN")
            .status()
            .expect("start the second launch");

        assert_eq!(
            status.code(),
            Some(0),
            "the second launch did not exit 0 (3 is a second copy running)"
        );
        let request = requests
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the first copy was never asked");
        assert_eq!(request["method"], "app.act");
        assert_eq!(request["params"]["action"], crate::present::ACTION);
        assert_eq!(request["params"]["args"]["display"], "wayland-test");
        assert_eq!(
            fs::read_to_string(dir.join("probe.pid")).unwrap(),
            std::process::id().to_string(),
            "the slot changed hands"
        );
        let _ = fs::remove_dir_all(&runtime);
    }
}
