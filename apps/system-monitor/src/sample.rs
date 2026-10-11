//! Taking readings, off the window's thread.
//!
//! A reading used to be taken on the UI thread, inside the poll timer: the service's
//! `sysmon.processes` measures CPU over a half-second window, so every two seconds the window
//! froze for more than half a second — and `describe`, which the control surface runs on the UI
//! thread, waited behind it. A reading is now taken on a thread of its own and handed to the
//! window when it is ready, so the window and `describe` only ever read the last one.

use std::cell::Cell;
use std::sync::mpsc;
use std::time::Instant;

use yantrik_ipc_contracts::machine_status::is_real_filesystem;
use yantrik_ipc_contracts::system_monitor::{
    CpuInfo, DiskInfo, MemoryInfo, NetworkInterface, ProcessInfo, SystemSnapshot,
};
use yantrik_ipc_transport::SyncRpcClient;

use crate::outcome::{self, Provenance};

/// One round of readings, and where they came from.
pub struct Reading {
    pub snap: SystemSnapshot,
    pub procs: Vec<ProcessInfo>,
    pub provenance: Provenance,
}

/// Take one reading. Blocks for the service's sample window; never call it on the UI thread.
///
/// The fallback used to be written `snapshot_via_service().unwrap_or_else(|_| snapshot_local())`,
/// which threw away both the reason and the fact that it had happened. It is worth keeping — a
/// monitor that goes blank because a service died is worse than one reading its own `sysinfo` —
/// but only as long as it is visible, so the value and the provenance arrive together.
pub fn take(sort: &str, limit: u32, local: &mut Local) -> Reading {
    let (snap, from_snapshot) = outcome::reading(snapshot_via_service(), || snapshot_local(local));
    let (procs, from_processes) =
        outcome::reading(processes_via_service(sort, limit), || processes_local(sort, limit));
    Reading { snap, procs, provenance: outcome::worse(from_snapshot, from_processes) }
}

/// The thread that takes readings, and the window's end of it.
pub struct Sampler {
    asks: mpsc::Sender<(&'static str, u32)>,
    readings: mpsc::Receiver<Reading>,
    /// One reading at a time. A service slower than the poll would otherwise queue asks
    /// faster than they are answered, and the window would draw an ever older machine.
    in_flight: Cell<bool>,
}

impl Sampler {
    /// Start the thread. `wake` is called on it after each reading, to bring the window to
    /// [`Sampler::latest`]; the thread may not touch the window itself.
    pub fn start(wake: impl Fn() + Send + 'static) -> Sampler {
        let (asks, work) = mpsc::channel::<(&'static str, u32)>();
        let (done, readings) = mpsc::channel();
        std::thread::spawn(move || {
            let mut local = Local::default();
            while let Ok((sort, limit)) = work.recv() {
                if done.send(take(sort, limit, &mut local)).is_err() {
                    break;
                }
                wake();
            }
        });
        Sampler { asks, readings, in_flight: Cell::new(false) }
    }

    /// Ask for a reading, unless one is already being taken.
    pub fn ask(&self, sort: &'static str, limit: u32) {
        if !self.in_flight.replace(true) && self.asks.send((sort, limit)).is_err() {
            self.in_flight.set(false);
        }
    }

    /// The newest reading that has arrived since the last call, if any.
    pub fn latest(&self) -> Option<Reading> {
        let mut last = None;
        while let Ok(reading) = self.readings.try_recv() {
            last = Some(reading);
        }
        if last.is_some() {
            self.in_flight.set(false);
        }
        last
    }
}

// ── Service wrappers ─────────────────────────────────────────────────

fn snapshot_via_service() -> Result<SystemSnapshot, String> {
    let client = SyncRpcClient::for_service("system-monitor");
    let result = client
        .call("sysmon.snapshot", serde_json::json!({}))
        .map_err(|e| e.message)?;
    serde_json::from_value(result).map_err(|e| e.to_string())
}

fn processes_via_service(sort_by: &str, limit: u32) -> Result<Vec<ProcessInfo>, String> {
    let client = SyncRpcClient::for_service("system-monitor");
    let result = client
        .call(
            "sysmon.processes",
            serde_json::json!({ "sort_by": sort_by, "limit": limit }),
        )
        .map_err(|e| e.message)?;
    serde_json::from_value(result).map_err(|e| e.to_string())
}

// ── Local sysinfo fallback ───────────────────────────────────────────

/// The fallback's `sysinfo` state, kept between readings on the sampler's thread.
///
/// `sysinfo`'s CPU figures are, like /proc/stat's, a difference between two refreshes, and the
/// fallback built a fresh `System` for every reading — so it never had two, and its CPU was
/// whatever one refresh said. It now keeps one, and reports no figure until it has an interval.
pub struct Local {
    sys: sysinfo::System,
    cpu_refreshed: Option<Instant>,
}

impl Default for Local {
    fn default() -> Self {
        Local { sys: sysinfo::System::new(), cpu_refreshed: None }
    }
}

fn snapshot_local(local: &mut Local) -> SystemSnapshot {
    use sysinfo::System;

    let sys = &mut local.sys;
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    // The poll is two seconds apart, well past sysinfo's minimum between CPU refreshes.
    let now = Instant::now();
    let window = local.cpu_refreshed.replace(now).map(|at| now.duration_since(at));

    let cores: Vec<_> = sys
        .cpus()
        .iter()
        .enumerate()
        .map(|(i, cpu)| yantrik_ipc_contracts::system_monitor::CpuCore {
            id: i as u32,
            usage_percent: cpu.cpu_usage() as f64,
        })
        .collect();

    let overall = if cores.is_empty() {
        0.0
    } else {
        cores.iter().map(|c| c.usage_percent).sum::<f64>() / cores.len() as f64
    };

    // Was hardcoded to zero, so the card always read "Load: 0.00 0.00 0.00".
    let load = sysinfo::System::load_average();

    let cpu = CpuInfo {
        overall_percent: overall,
        cores,
        load_avg_1: load.one,
        load_avg_5: load.five,
        load_avg_15: load.fifteen,
        window_ms: window.map(|w| w.as_millis() as u64),
    };

    let (cached, buffers) = cached_and_buffers();

    let memory = MemoryInfo {
        total_bytes: sys.total_memory(),
        used_bytes: sys.used_memory(),
        usage_percent: if sys.total_memory() > 0 {
            (sys.used_memory() as f64 / sys.total_memory() as f64) * 100.0
        } else {
            0.0
        },
        swap_total_bytes: sys.total_swap(),
        swap_used_bytes: sys.used_swap(),
        available_bytes: sys.available_memory(),
        cached_bytes: cached,
        buffers_bytes: buffers,
    };

    // The same filesystems the service lists. `sysinfo` also reports squashfs and ISO images,
    // which are full by construction, and would draw a live session's disk card at 100%.
    let disks: Vec<DiskInfo> = sysinfo::Disks::new_with_refreshed_list()
        .iter()
        .filter(|d| is_real_filesystem(&d.file_system().to_string_lossy()))
        .map(|d| DiskInfo {
            mount_point: d.mount_point().to_string_lossy().to_string(),
            device: d.name().to_string_lossy().to_string(),
            filesystem: d.file_system().to_string_lossy().to_string(),
            total_bytes: d.total_space(),
            used_bytes: d.total_space() - d.available_space(),
            usage_percent: if d.total_space() > 0 {
                ((d.total_space() - d.available_space()) as f64 / d.total_space() as f64) * 100.0
            } else {
                0.0
            },
        })
        .collect();

    let networks: Vec<NetworkInterface> = sysinfo::Networks::new_with_refreshed_list()
        .iter()
        .map(|(name, data)| NetworkInterface {
            name: name.clone(),
            rx_bytes: data.total_received(),
            tx_bytes: data.total_transmitted(),
            rx_rate_bps: data.received(),
            tx_rate_bps: data.transmitted(),
        })
        .collect();

    SystemSnapshot {
        cpu,
        memory,
        disks,
        networks,
        uptime_secs: System::uptime(),
    }
}

fn processes_local(sort_by: &str, limit: u32) -> Vec<ProcessInfo> {
    use sysinfo::System;

    let mut sys = System::new_all();
    sys.refresh_all();

    let mut procs: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .map(|p| ProcessInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().to_string(),
            cpu_percent: p.cpu_usage() as f64,
            mem_percent: if sys.total_memory() > 0 {
                (p.memory() as f64 / sys.total_memory() as f64) * 100.0
            } else {
                0.0
            },
            mem_bytes: p.memory(),
            state: format!("{:?}", p.status()),
            user: String::new(),
        })
        .collect();

    match sort_by {
        "mem" => procs.sort_by(|a, b| b.mem_percent.partial_cmp(&a.mem_percent).unwrap_or(std::cmp::Ordering::Equal)),
        _ => procs.sort_by(|a, b| b.cpu_percent.partial_cmp(&a.cpu_percent).unwrap_or(std::cmp::Ordering::Equal)),
    }

    procs.truncate(limit as usize);
    procs
}

/// Page cache and buffer sizes are not exposed by `sysinfo`; on Linux they come
/// straight out of /proc/meminfo, whose values are in kB.
#[cfg(target_os = "linux")]
fn cached_and_buffers() -> (u64, u64) {
    let (mut cached, mut buffers) = (0u64, 0u64);
    if let Ok(text) = std::fs::read_to_string("/proc/meminfo") {
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            let key = parts.next().unwrap_or("");
            let kb: u64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            match key {
                "Cached:" => cached = kb * 1024,
                "Buffers:" => buffers = kb * 1024,
                _ => {}
            }
        }
    }
    (cached, buffers)
}

#[cfg(not(target_os = "linux"))]
fn cached_and_buffers() -> (u64, u64) {
    (0, 0)
}
