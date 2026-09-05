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

fn runtime_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        let p = PathBuf::from(dir).join("yantrik");
        if fs::create_dir_all(&p).is_ok() {
            return p;
        }
    }
    // No runtime dir: fall back to a per-USER temp dir. It must not vary per process, or every
    // launch would get its own pid file and the guard would never see the previous instance.
    let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
    let p = std::env::temp_dir().join(format!("yantrik-{user}"));
    let _ = fs::create_dir_all(&p);
    p
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
    let path = runtime_dir().join(format!("{app_name}.pid"));

    if let Ok(existing) = fs::read_to_string(&path) {
        if let Ok(pid) = existing.trim().parse::<u32>() {
            if pid != std::process::id() && pid_alive(pid) {
                tracing::info!(app = app_name, pid, "Already running; leaving it to the existing instance");
                return None;
            }
        }
        // Stale: the recorded pid is gone. Fall through and take over.
    }

    match fs::File::create(&path).and_then(|mut f| write!(f, "{}", std::process::id())) {
        Ok(()) => Some(InstanceGuard { path }),
        Err(e) => {
            // Not being able to write the pid file is not a reason to refuse to run.
            tracing::warn!(app = app_name, error = %e, "Could not write pid file; running unguarded");
            Some(InstanceGuard { path: PathBuf::new() })
        }
    }
}
