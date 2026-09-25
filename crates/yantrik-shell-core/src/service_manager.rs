//! Service Lifecycle Manager — starts, stops, and monitors service processes.
//!
//! The shell uses this to manage standalone service binaries (weather-service,
//! system-monitor-service, etc.) as child processes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};

/// Status of a managed service.
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    Stopped,
    Starting,
    Running,
    Failed(String),
}

/// Info about a registered service.
#[derive(Debug, Clone)]
pub struct ServiceEntry {
    pub id: String,
    pub binary: PathBuf,
    pub autostart: bool,
    pub status: ServiceStatus,
}

/// Manages service process lifecycles.
#[derive(Clone)]
pub struct ServiceManager {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    services: HashMap<String, ServiceEntry>,
    processes: HashMap<String, Child>,
    services_dir: PathBuf,
}

impl ServiceManager {
    /// Create a new service manager.
    /// `services_dir` is the directory containing service binaries.
    pub fn new(services_dir: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                services: HashMap::new(),
                processes: HashMap::new(),
                services_dir,
            })),
        }
    }

    /// Register a service for management.
    pub fn register(&self, id: &str, binary_name: &str, autostart: bool) {
        let mut inner = self.inner.lock().unwrap();
        let binary = inner.services_dir.join(binary_name);
        inner.services.insert(
            id.to_string(),
            ServiceEntry {
                id: id.to_string(),
                binary,
                autostart,
                status: ServiceStatus::Stopped,
            },
        );
        tracing::info!(service = id, "Service registered");
    }

    /// Start all services marked as autostart.
    pub fn start_autostart(&self) {
        let ids: Vec<String> = {
            let inner = self.inner.lock().unwrap();
            inner
                .services
                .values()
                .filter(|s| s.autostart)
                .map(|s| s.id.clone())
                .collect()
        };

        for id in ids {
            if let Err(e) = self.start(&id) {
                tracing::error!(service = %id, error = %e, "Failed to autostart service");
            }
        }
    }

    /// Start a service by ID.
    pub fn start(&self, id: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        // A service that exited since anyone last asked still reads `Running` until it is reaped,
        // and short-circuiting on that left it down: a caller asked for it to be up and was told
        // it was. Reaping here is what `status` does, so `start` no longer depends on every
        // caller remembering to ask `status` first (#58).
        inner.reap(id);

        let entry = inner
            .services
            .get(id)
            .ok_or_else(|| format!("Unknown service: {id}"))?;

        if entry.status == ServiceStatus::Running {
            return Ok(());
        }

        if !entry.binary.exists() {
            let msg = format!("Binary not found: {}", entry.binary.display());
            inner.services.get_mut(id).unwrap().status =
                ServiceStatus::Failed(msg.clone());
            return Err(msg);
        }

        let binary = entry.binary.clone();
        inner.services.get_mut(id).unwrap().status = ServiceStatus::Starting;
        tracing::info!(service = id, binary = %binary.display(), "Starting service");

        let mut command = Command::new(&binary);
        tie_lifetime_to_ours(&mut command);

        match command.spawn() {
            Ok(child) => {
                inner.services.get_mut(id).unwrap().status = ServiceStatus::Running;
                inner.processes.insert(id.to_string(), child);
                tracing::info!(service = id, "Service started");
                Ok(())
            }
            Err(e) => {
                let msg = format!("Failed to start {id}: {e}");
                inner.services.get_mut(id).unwrap().status =
                    ServiceStatus::Failed(msg.clone());
                Err(msg)
            }
        }
    }

    /// Stop a service by ID.
    pub fn stop(&self, id: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();

        if let Some(mut child) = inner.processes.remove(id) {
            tracing::info!(service = id, "Stopping service");
            let _ = child.kill();
            let _ = child.wait();
        }

        if let Some(entry) = inner.services.get_mut(id) {
            entry.status = ServiceStatus::Stopped;
        }
        Ok(())
    }

    /// Get the status of a service.
    pub fn status(&self, id: &str) -> Option<ServiceStatus> {
        let mut inner = self.inner.lock().unwrap();
        inner.reap(id);
        inner.services.get(id).map(|s| s.status.clone())
    }

    /// List all registered services.
    pub fn list(&self) -> Vec<ServiceEntry> {
        let inner = self.inner.lock().unwrap();
        inner.services.values().cloned().collect()
    }

    /// Stop all running services.
    pub fn stop_all(&self) {
        let ids: Vec<String> = {
            let inner = self.inner.lock().unwrap();
            inner.processes.keys().cloned().collect()
        };
        for id in ids {
            let _ = self.stop(&id);
        }
    }

    /// Scan a directory for service manifests (yantrik.toml) and register them.
    pub fn scan_and_register(&self, manifests_dir: &Path) {
        let entries = match std::fs::read_dir(manifests_dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(path = %manifests_dir.display(), error = %e, "Cannot scan service dir");
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("yantrik.toml");
            if !manifest_path.exists() {
                continue;
            }
            // Simple TOML parsing for service registration
            if let Ok(content) = std::fs::read_to_string(&manifest_path) {
                let id = extract_toml_value(&content, "id").unwrap_or_default();
                let binary = extract_toml_value(&content, "binary").unwrap_or_default();
                let autostart = extract_toml_value(&content, "autostart")
                    .map(|v| v == "true")
                    .unwrap_or(false);

                if !id.is_empty() && !binary.is_empty() {
                    self.register(&id, &binary, autostart);
                }
            }
        }
    }
}

impl Inner {
    /// If `id`'s process has exited, collect it and record how it ended.
    fn reap(&mut self, id: &str) {
        let Some(child) = self.processes.get_mut(id) else { return };
        match child.try_wait() {
            Ok(Some(status)) => {
                self.processes.remove(id);
                if let Some(entry) = self.services.get_mut(id) {
                    entry.status = if status.success() {
                        ServiceStatus::Stopped
                    } else {
                        ServiceStatus::Failed(format!("Exited with code: {:?}", status.code()))
                    };
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(service = id, error = %e, "Error checking service status");
            }
        }
    }
}

/// Simple TOML value extractor (avoids pulling in toml crate for shell-core).
fn extract_toml_value(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(key) && trimmed.contains('=') {
            let val = trimmed.split('=').nth(1)?.trim();
            let val = val.trim_matches('"').trim_matches('\'');
            return Some(val.to_string());
        }
    }
    None
}

/// Stop what is still running when the LAST handle goes, not when any of them does.
///
/// This was `impl Drop for ServiceManager`, and ServiceManager is `Clone`: every clone shares one
/// `Inner`, so dropping any handle stopped every service for all of them. It went unnoticed while
/// each clone happened to be moved into something that lives as long as the shell — a timer
/// closure, the boot wiring. Then `control::publish` was given a handle for `start_service`,
/// cloned it into the action and let its own copy go when the function returned, and every
/// autostart service on the machine was killed two hundred milliseconds after it started, on
/// every shell start. Nothing reported it: Weather fell back to fetching directly, System Monitor
/// fell back to reading the machine itself, and both went on passing their checks. The one app
/// with no fallback, Network Manager, is how it was found — by a probe, on a real machine.
///
/// On `Inner` the rule is the one that was always meant: the services stop when nothing can reach
/// the manager any more, which is when the shell is going away.
impl Drop for Inner {
    fn drop(&mut self) {
        for (id, mut child) in self.processes.drain() {
            tracing::info!(service = %id, "Stopping service");
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Make the kernel kill this child if the shell dies.
///
/// `stop_all` handles a clean shutdown, and a clean shutdown is not the case that matters. A
/// crash, an OOM kill, a `pkill` — any of those leave the services running with no parent, and
/// nothing ever collects them. Measuring memory on this machine turned up forty-six orphaned
/// `network-service` processes, the oldest forty-one hours old, one per shell start over two days
/// of testing. They were reparented to init and would have stayed until reboot.
///
/// `PR_SET_PDEATHSIG` asks the kernel to send a signal to this process when its *parent thread*
/// exits, which closes that hole without any bookkeeping on our side.
///
/// Two things worth knowing about it. It is inherited across `fork` but cleared on `execve` of a
/// setuid binary, which none of these are. And it fires on the death of the parent *thread*, not
/// the parent process — so it is set in `pre_exec`, after the fork, when the child is still
/// attached to the thread that spawned it.
#[cfg(unix)]
fn tie_lifetime_to_ours(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // SAFETY: `pre_exec` runs in the forked child between fork and exec, where only
    // async-signal-safe calls are allowed. `prctl` with these arguments is one: it sets a flag on
    // the calling process and allocates nothing.
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The race this closes: if the parent died between the fork and the prctl above, the
            // signal has already been sent and will never come again. Checking afterwards is the
            // only way to notice, and exiting is the right answer — the shell we were started for
            // is gone.
            if libc::getppid() == 1 {
                std::process::exit(0);
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn tie_lifetime_to_ours(_command: &mut Command) {}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    /// One test at a time, from writing its script to the end.
    ///
    /// These tests write an executable and then run it, and the harness runs them on parallel
    /// threads. `fork` copies every open descriptor into the child, and O_CLOEXEC closes them
    /// only at `exec` — so while one test's script is still open for writing, another test's
    /// freshly forked child holds a second copy of that write descriptor for a few microseconds.
    /// If the first test reaches its own `exec` inside that window the kernel refuses with
    /// ETXTBSY, "Text file busy". It passed three CI runs and failed the fourth, on a loaded
    /// runner, with a message that says nothing about tests. Nothing in the shell writes the
    /// binaries it starts, so this is the tests' problem and is fixed in the tests: they take
    /// turns. A poisoned lock is still a lock — one failed test must not fail the others.
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn taking_turns() -> std::sync::MutexGuard<'static, ()> {
        ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A directory holding one "service": a script that records its pid and then waits.
    fn fixture(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("svcmgr-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("pid");
        let script = dir.join("svc");
        std::fs::write(
            &script,
            format!("#!/bin/sh\necho $$ > {}\nexec sleep 30\n", pidfile.display()),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, pidfile)
    }

    fn pid_from(pidfile: &Path) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(s) = std::fs::read_to_string(pidfile) {
                if let Ok(pid) = s.trim().parse() {
                    return pid;
                }
            }
            assert!(Instant::now() < deadline, "the service never wrote its pid");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn alive(pid: u32) -> bool {
        // A zombie still has a /proc entry; a reaped process does not.
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|s| !s.contains(") Z "))
            .unwrap_or(false)
    }

    #[test]
    fn dropping_one_handle_does_not_stop_the_services_the_others_still_hold() {
        let _turn = taking_turns();
        // The bug: ServiceManager is Clone, Drop was on the handle, and a function that was
        // handed a clone and returned took every autostart service on the machine down with it.
        let (dir, pidfile) = fixture("clone-drop");
        let mgr = ServiceManager::new(dir.clone());
        mgr.register("svc", "svc", true);
        mgr.start("svc").unwrap();
        let pid = pid_from(&pidfile);

        {
            let handed_to_a_function = mgr.clone();
            drop(handed_to_a_function);
        }
        std::thread::sleep(Duration::from_millis(150));
        assert!(alive(pid), "a dropped clone stopped a service the manager still owns");
        assert_eq!(mgr.status("svc"), Some(ServiceStatus::Running));

        let _ = std::fs::remove_dir_all(&dir);
        drop(mgr);
    }

    #[test]
    fn starting_a_service_that_exited_starts_it_again() {
        let _turn = taking_turns();
        // `start` short-circuited on a `Running` that was only true until the process exited,
        // and nothing had reaped it yet: asked to bring a dead service up, it did nothing and
        // said Ok. Both callers asked `status` first to get around it (#58).
        let (dir, pidfile) = fixture("restart");
        let mgr = ServiceManager::new(dir.clone());
        mgr.register("svc", "svc", false);
        mgr.start("svc").unwrap();
        let first = pid_from(&pidfile);

        let _ = std::fs::remove_file(&pidfile);
        unsafe { libc::kill(first as i32, libc::SIGKILL) };
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(first) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive(first), "the fixture service did not die");

        // No `status` call in between: that is the case the old `start` got wrong.
        mgr.start("svc").unwrap();
        let second = pid_from(&pidfile);
        assert_ne!(first, second, "start left the dead service down and reported success");
        assert!(alive(second));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_last_handle_going_away_stops_what_is_running() {
        let _turn = taking_turns();
        let (dir, pidfile) = fixture("last-drop");
        let mgr = ServiceManager::new(dir.clone());
        mgr.register("svc", "svc", true);
        mgr.start("svc").unwrap();
        let pid = pid_from(&pidfile);
        let other = mgr.clone();

        drop(mgr);
        assert!(alive(pid), "one handle is still held");
        drop(other);

        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive(pid), "the service outlived every handle to its manager");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
