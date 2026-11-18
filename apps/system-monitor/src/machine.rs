//! The machine this app reads, and the processes it ends: behind one trait, so the app runs
//! headless under a test against a fake and no test signals a real process.
//!
//! Everything here blocks (a service call, `/proc`, a signal and the wait after it), so it is
//! only ever called from a job. The live machine is the code `main.rs` used to run on the
//! window's thread and on the sampler's: the same service calls, the same local fallback, the
//! same `kill(2)`.

use std::sync::{Arc, Mutex};

use yantrik_ipc_transport::SyncRpcClient;

use crate::outcome::{self, Liveness, Observed, Signal, Source};
use crate::sample::{self, Local, Reading};

/// What the app asks of the machine.
pub trait Machine: Send + Sync {
    /// One round of readings: the snapshot and the busiest `limit` processes by `sort`
    /// (`"cpu"` or `"mem"`), with where they came from.
    fn reading(&self, sort: &'static str, limit: u32) -> Reading;
    /// Whether the process table still holds `pid`.
    fn liveness(&self, pid: u32) -> Liveness;
    /// Send `signal` to `pid`, by the service if it will take it and by this process if not.
    fn deliver(&self, pid: u32, signal: Signal) -> Result<Source, String>;
}

/// End a process and say what was observed to happen to it: one path for the End button, the
/// Force Kill button and the `kill_process` action.
///
/// Before this existed they were three different stories. The action invoked the button's
/// callback and answered `{"killed": pid}` regardless of what happened next; the callback logged
/// a tracing warning when the service refused and then retried locally in silence; the force
/// callback skipped the service and dropped the return value of the kill outright. A kill is a
/// request, not an outcome, so what is reported is read back from the process table.
///
/// `pid` has passed [`outcome::checked_pid`]; `name` is what the list on screen calls it.
pub fn end(machine: &dyn Machine, pid: u32, name: &str, force: bool) -> Result<Observed, String> {
    // Nothing to signal. `kill(2)` would fail with ESRCH anyway, but the old local fallback
    // swallowed that — `sys.process(...)` returning `None` simply did nothing — so ending a pid
    // that was never there reported the same success as ending a real one.
    if machine.liveness(pid) == Liveness::Gone {
        return Err(format!("no process {pid} is running"));
    }
    let signal = if force { Signal::Kill } else { Signal::Term };
    let via = machine.deliver(pid, signal)?;
    let (exited, waited_ms) = outcome::wait_until_gone(
        pid,
        outcome::VERIFY_BUDGET,
        outcome::VERIFY_STEP,
        |p| machine.liveness(p),
    );
    outcome::classify(pid, name, signal, via, exited, waited_ms)
}

/// The machine this process runs on. (Never made under a test; see [`for_paths`].)
#[derive(Default)]
#[cfg_attr(test, allow(dead_code))]
pub struct Live {
    /// The fallback's `sysinfo` state: its CPU figures are a difference between two refreshes,
    /// so it is kept from one reading to the next. One reading at a time holds it.
    local: Mutex<Local>,
}

impl Machine for Live {
    fn reading(&self, sort: &'static str, limit: u32) -> Reading {
        let mut local = self.local.lock().unwrap_or_else(|p| p.into_inner());
        sample::take(sort, limit, &mut local)
    }

    fn liveness(&self, pid: u32) -> Liveness {
        outcome::liveness(pid)
    }

    /// The service runs the same `kill(2)` this does, so the local path is a genuine fallback and
    /// not a lesser one — but which of the two did it is part of the answer, because a machine
    /// whose system-monitor service is dead is a machine whose owner should hear about it.
    fn deliver(&self, pid: u32, signal: Signal) -> Result<Source, String> {
        let mut service_refused = None;

        // `sysmon.kill_process` takes a pid and no signal, and sends SIGTERM (see
        // services/system-monitor-service). A forced kill therefore has no service path to try,
        // and pretending otherwise would send a SIGTERM while answering SIGKILL.
        if signal == Signal::Term {
            match kill_process_via_service(pid) {
                Ok(()) => return Ok(Source::Service),
                Err(e) => {
                    tracing::warn!("Kill process {pid} via service failed: {e}");
                    service_refused = Some(e);
                }
            }
        }

        match signal_locally(pid, signal) {
            Ok(()) => Ok(Source::Local),
            Err(local) => Err(match service_refused {
                Some(service) => format!("{local} (the service did not take it either: {service})"),
                None => local,
            }),
        }
    }
}

#[cfg_attr(test, allow(dead_code))]
fn kill_process_via_service(pid: u32) -> Result<(), String> {
    let client = SyncRpcClient::for_service("system-monitor");
    client
        .call("sysmon.kill_process", serde_json::json!({ "pid": pid }))
        .map_err(|e| e.message)?;
    Ok(())
}

#[cfg(unix)]
#[cfg_attr(test, allow(dead_code))]
fn signal_locally(pid: u32, signal: Signal) -> Result<(), String> {
    let number = match signal {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    if unsafe { libc::kill(pid as libc::pid_t, number) } == 0 {
        Ok(())
    } else {
        Err(outcome::signal_failure(pid, &std::io::Error::last_os_error()))
    }
}

#[cfg(not(unix))]
#[cfg_attr(test, allow(dead_code))]
fn signal_locally(pid: u32, signal: Signal) -> Result<(), String> {
    Err(format!("this build cannot send {} to {pid}", signal.as_str()))
}

/// The machine the app is given at start: the live one, or under a test the fake the test
/// installed for these paths (`fake::install`). A test never reaches the live machine, so no
/// test signals a process it did not start.
pub fn for_paths(paths: &yantrik_app::Paths) -> Arc<dyn Machine> {
    #[cfg(test)]
    {
        fake::installed(paths)
    }
    #[cfg(not(test))]
    {
        let _ = paths;
        Arc::new(Live::default())
    }
}

#[cfg(test)]
pub mod fake {
    //! A machine for tests: it reads what it was told to, and its process table is a set.

    use std::collections::{HashMap, HashSet, VecDeque};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use yantrik_ipc_contracts::system_monitor::{
        CpuCore, CpuInfo, DiskInfo, MemoryInfo, NetworkInterface, ProcessInfo, SystemSnapshot,
    };

    use super::Machine;
    use crate::outcome::{Liveness, Provenance, Signal, Source};
    use crate::sample::Reading;

    const GB: u64 = 1024 * 1024 * 1024;

    static INSTALLED: Mutex<Vec<(PathBuf, Arc<FakeMachine>)>> = Mutex::new(Vec::new());

    /// The fake an app started at `paths` is given.
    pub fn install(paths: &yantrik_app::Paths, machine: Arc<FakeMachine>) {
        INSTALLED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((paths.config_file().to_path_buf(), machine));
    }

    pub(super) fn installed(paths: &yantrik_app::Paths) -> Arc<dyn Machine> {
        let found = INSTALLED
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|(at, _)| at == paths.config_file())
            .map(|(_, m)| m.clone());
        found.unwrap_or_else(|| Arc::new(FakeMachine::new()))
    }

    #[derive(Default)]
    pub struct FakeMachine {
        /// What the next readings say, in order; past them, the last again.
        readings: Mutex<VecDeque<Reading>>,
        last: Mutex<Option<Reading>>,
        asked: Mutex<Vec<(&'static str, u32)>>,
        /// The pids in the table.
        running: Mutex<HashSet<u32>>,
        /// Pids that stay in the table whatever they are sent.
        stubborn: Mutex<HashSet<u32>>,
        /// Pids whose signal cannot be delivered, and why.
        undeliverable: Mutex<HashMap<u32, String>>,
        signalled: Mutex<Vec<(u32, Signal)>>,
        /// A reading panics, as a worker would on a document it cannot read.
        panics: Mutex<bool>,
    }

    impl FakeMachine {
        pub fn new() -> FakeMachine {
            FakeMachine::default()
        }

        pub fn reads(self, reading: Reading) -> FakeMachine {
            self.readings.lock().unwrap().push_back(reading);
            self
        }

        pub fn running(self, pids: impl IntoIterator<Item = u32>) -> FakeMachine {
            self.running.lock().unwrap().extend(pids);
            self
        }

        pub fn stubborn(self, pid: u32) -> FakeMachine {
            self.running.lock().unwrap().insert(pid);
            self.stubborn.lock().unwrap().insert(pid);
            self
        }

        pub fn undeliverable(self, pid: u32, why: &str) -> FakeMachine {
            self.running.lock().unwrap().insert(pid);
            self.undeliverable.lock().unwrap().insert(pid, why.to_string());
            self
        }

        pub fn panicking(self) -> FakeMachine {
            *self.panics.lock().unwrap() = true;
            self
        }

        /// The next readings say this.
        pub fn will_read(&self, reading: Reading) {
            self.readings.lock().unwrap().push_back(reading);
        }

        pub fn asked(&self) -> Vec<(&'static str, u32)> {
            self.asked.lock().unwrap().clone()
        }

        pub fn signalled(&self) -> Vec<(u32, Signal)> {
            self.signalled.lock().unwrap().clone()
        }
    }

    impl Machine for FakeMachine {
        fn reading(&self, sort: &'static str, limit: u32) -> Reading {
            if *self.panics.lock().unwrap() {
                panic!("the service's answer had no cpu block");
            }
            self.asked.lock().unwrap().push((sort, limit));
            let next = self.readings.lock().unwrap().pop_front();
            let mut last = self.last.lock().unwrap();
            if let Some(next) = next {
                *last = Some(next);
            }
            last.clone().unwrap_or_else(|| reading(snapshot(Some(2000)), Vec::new(), Provenance::service()))
        }

        fn liveness(&self, pid: u32) -> Liveness {
            if self.running.lock().unwrap().contains(&pid) {
                Liveness::Running
            } else {
                Liveness::Gone
            }
        }

        fn deliver(&self, pid: u32, signal: Signal) -> Result<Source, String> {
            if let Some(why) = self.undeliverable.lock().unwrap().get(&pid) {
                return Err(why.clone());
            }
            self.signalled.lock().unwrap().push((pid, signal));
            if !self.stubborn.lock().unwrap().contains(&pid) {
                self.running.lock().unwrap().remove(&pid);
            }
            // The service takes a SIGTERM; a SIGKILL has no service call, so it is this process's.
            Ok(if signal == Signal::Term { Source::Service } else { Source::Local })
        }
    }

    pub fn reading(snap: SystemSnapshot, procs: Vec<ProcessInfo>, provenance: Provenance) -> Reading {
        Reading { snap, procs, provenance }
    }

    pub fn process(pid: u32, name: &str, cpu: f64, mem: f64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.into(),
            cpu_percent: cpu,
            mem_percent: mem,
            mem_bytes: 1024,
            state: "S".into(),
            user: "yantrik".into(),
        }
    }

    /// A machine at 12.34% CPU over `window_ms`, half its 16 GB in use, `disk_used_gb` of / used.
    pub fn snapshot_with_disk(window_ms: Option<u64>, disk_used_gb: f64) -> SystemSnapshot {
        let total = (31.3 * GB as f64) as u64;
        let used = (disk_used_gb * GB as f64) as u64;
        SystemSnapshot {
            cpu: CpuInfo {
                overall_percent: 12.34,
                cores: (0..4).map(|id| CpuCore { id, usage_percent: 12.34 }).collect(),
                load_avg_1: 1.15,
                load_avg_5: 1.0,
                load_avg_15: 0.9,
                window_ms,
            },
            memory: MemoryInfo {
                total_bytes: 16 * GB,
                used_bytes: 8 * GB,
                usage_percent: 50.0,
                swap_total_bytes: 4 * GB,
                swap_used_bytes: GB,
                available_bytes: 8 * GB,
                cached_bytes: 2 * GB,
                buffers_bytes: 512 * 1024 * 1024,
            },
            disks: vec![DiskInfo {
                mount_point: "/".into(),
                device: "/dev/sda1".into(),
                filesystem: "ext4".into(),
                total_bytes: total,
                used_bytes: used,
                usage_percent: used as f64 / total as f64 * 100.0,
            }],
            networks: vec![NetworkInterface {
                name: "eth0".into(),
                rx_bytes: 3 * 1024 * 1024,
                tx_bytes: 1024 * 1024,
                rx_rate_bps: 2048,
                tx_rate_bps: 100,
            }],
            uptime_secs: 3 * 3600 + 25 * 60,
        }
    }

    pub fn snapshot(window_ms: Option<u64>) -> SystemSnapshot {
        snapshot_with_disk(window_ms, 10.0)
    }
}
