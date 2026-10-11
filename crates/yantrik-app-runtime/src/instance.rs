//! One window per app.
//!
//! The launcher, the taskbar pins and the companion can all ask for "notes" within the same
//! minute, and without this each request would spawn another process with another window.
//! The guard is a pid file under the runtime dir: the first process writes its pid, later ones
//! find a live pid there and exit at once, and the shell — which already lists open windows —
//! focuses the one that exists.
//!
//! A stale file (the previous instance crashed) is detected by checking that the pid is still
//! alive, so a crash never locks the user out of an app until reboot.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// Held for the life of the process; removes the pid file on drop.
pub struct InstanceGuard {
    path: PathBuf,
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
    let Some(close) = stat.rfind(')') else { return false };
    let state = stat[close + 1..].trim_start().chars().next().unwrap_or('Z');
    !matches!(state, 'Z' | 'X')
}

/// Claim the single-instance slot for `app_name`.
///
/// Returns `None` when another live instance holds it — the caller should exit quietly.
pub fn claim(app_name: &str) -> Option<InstanceGuard> {
    let dir = runtime_dir();
    if !private_to_us(&dir) {
        // `socket_dir` hands back its last candidate unchecked when it could prepare none, and a
        // directory that may be someone else's is no place to keep a file whose contents decide
        // whether we run. Running unguarded risks a second window; trusting it risks worse.
        tracing::warn!(app = app_name, dir = %dir.display(), "Pid directory is not private to us; running unguarded");
        return Some(InstanceGuard { path: PathBuf::new() });
    }
    let path = dir.join(format!("{app_name}.pid"));

    if let Ok(existing) = fs::read_to_string(&path) {
        if let Ok(pid) = existing.trim().parse::<u32>() {
            if pid != std::process::id() && pid_alive(pid) {
                tracing::info!(app = app_name, pid, "Already running; leaving it to the existing instance");
                return None;
            }
        }
        // Stale: the recorded pid is gone. Fall through and take over.
    }

    // Unlink, then create only if absent: a plain create follows a link left at the name, and a
    // `notes.pid` pointing at ~/.ssh/authorized_keys would have been emptied by the next launch.
    // `create_new` (O_EXCL) never follows one. If another launch wins the gap, we run unguarded.
    let _ = fs::remove_file(&path);
    let created = fs::OpenOptions::new().write(true).create_new(true).open(&path);
    match created.and_then(|mut f| write!(f, "{}", std::process::id())) {
        Ok(()) => Some(InstanceGuard { path }),
        Err(e) => {
            // Not being able to write the pid file is not a reason to refuse to run.
            tracing::warn!(app = app_name, error = %e, "Could not write pid file; running unguarded");
            Some(InstanceGuard { path: PathBuf::new() })
        }
    }
}
