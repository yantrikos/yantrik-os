//! System Monitor service — reads /proc for CPU, memory, disk, network, processes.
//!
//! This service is Linux-only (reads /proc, uses libc). On non-Unix platforms
//! it compiles but returns stub data for development purposes.
//!
//! Methods:
//!   sysmon.snapshot    {}                        → SystemSnapshot
//!   sysmon.processes   { sort_by?, limit? }      → Vec<ProcessInfo>
//!   sysmon.kill_process { pid }                  → ()

use std::sync::Arc;

use yantrik_ipc_contracts::email::ServiceError;
use yantrik_ipc_contracts::system_monitor::*;
use yantrik_ipc_transport::server::{RpcServer, ServiceHandler};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("system_monitor_service=info".parse().unwrap()),
        )
        .init();

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    rt.block_on(async {
        let handler = Arc::new(SysMonHandler);
        let addr = RpcServer::default_address("system-monitor");
        let server = RpcServer::new(&addr);
        tracing::info!("Starting system-monitor service");
        if let Err(e) = server.serve(handler).await {
            tracing::error!(error = %e, "System-monitor service failed");
        }
    });
}

struct SysMonHandler;

impl ServiceHandler for SysMonHandler {
    fn service_id(&self) -> &str {
        "system-monitor"
    }

    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        match method {
            "sysmon.snapshot" => {
                let snap = build_snapshot()?;
                Ok(serde_json::to_value(snap).unwrap())
            }
            "sysmon.processes" => {
                let sort_by = params["sort_by"].as_str().unwrap_or("cpu");
                let limit = params["limit"].as_u64().unwrap_or(20) as u32;
                let procs = read_processes(sort_by, limit)?;
                Ok(serde_json::to_value(procs).unwrap())
            }
            "sysmon.kill_process" => {
                let pid = params["pid"].as_u64().ok_or_else(|| ServiceError {
                    code: -32602,
                    message: "Missing 'pid' parameter".to_string(),
                })? as u32;
                kill_process(pid)?;
                Ok(serde_json::json!(null))
            }
            _ => Err(ServiceError {
                code: -1,
                message: format!("Unknown method: {method}"),
            }),
        }
    }
}

// ══════════════════════════════════════════════════════════════════════
// Linux implementation (reads /proc, uses libc)
// ══════════════════════════════════════════════════════════════════════

#[cfg(unix)]
mod platform {
    use super::*;
    use std::collections::HashMap;

    pub fn build_snapshot() -> Result<SystemSnapshot, ServiceError> {
        let (overall, cores) = read_cpu_usage();
        let (l1, l5, l15) = read_load_avg();

        let cpu = CpuInfo {
            overall_percent: overall,
            cores: cores
                .into_iter()
                .enumerate()
                .map(|(i, usage)| CpuCore {
                    id: i as u32,
                    usage_percent: usage,
                })
                .collect(),
            load_avg_1: l1,
            load_avg_5: l5,
            load_avg_15: l15,
        };

        let meminfo = read_meminfo();
        let total = meminfo.get("MemTotal").copied().unwrap_or(0);
        let available = meminfo.get("MemAvailable").copied().unwrap_or(0);
        let used = total.saturating_sub(available);
        let swap_total = meminfo.get("SwapTotal").copied().unwrap_or(0);
        let swap_free = meminfo.get("SwapFree").copied().unwrap_or(0);
        let swap_used = swap_total.saturating_sub(swap_free);

        let memory = MemoryInfo {
            total_bytes: total,
            used_bytes: used,
            usage_percent: if total > 0 {
                used as f64 / total as f64 * 100.0
            } else {
                0.0
            },
            swap_total_bytes: swap_total,
            swap_used_bytes: swap_used,
        };

        let disks = read_mounts()
            .into_iter()
            .map(|(mount, dev, fs, used_b, total_b)| DiskInfo {
                mount_point: mount,
                device: dev,
                filesystem: fs,
                total_bytes: total_b,
                used_bytes: used_b,
                usage_percent: if total_b > 0 {
                    used_b as f64 / total_b as f64 * 100.0
                } else {
                    0.0
                },
            })
            .collect();

        let networks = read_net_dev()
            .into_iter()
            .map(|(name, rx, tx)| NetworkInterface {
                name,
                rx_bytes: rx,
                tx_bytes: tx,
                rx_rate_bps: 0,
                tx_rate_bps: 0,
            })
            .collect();

        let uptime_secs = read_uptime_secs();

        Ok(SystemSnapshot {
            cpu,
            memory,
            disks,
            networks,
            uptime_secs,
        })
    }

    fn read_cpu_usage() -> (f64, Vec<f64>) {
        let content = match std::fs::read_to_string("/proc/stat") {
            Ok(c) => c,
            Err(_) => return (0.0, Vec::new()),
        };

        let mut overall = 0.0;
        let mut cores = Vec::new();

        for line in content.lines() {
            if !line.starts_with("cpu") {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 5 {
                continue;
            }
            let values: Vec<u64> = parts[1..]
                .iter()
                .filter_map(|s| s.parse().ok())
                .collect();
            if values.len() < 4 {
                continue;
            }
            let idle = values[3] + values.get(4).copied().unwrap_or(0);
            let total: u64 = values.iter().sum();
            let usage = if total > 0 {
                (total - idle) as f64 / total as f64 * 100.0
            } else {
                0.0
            };

            if parts[0] == "cpu" {
                overall = usage;
            } else {
                cores.push(usage);
            }
        }

        (overall, cores)
    }

    fn read_load_avg() -> (f64, f64, f64) {
        match std::fs::read_to_string("/proc/loadavg") {
            Ok(content) => {
                let parts: Vec<&str> = content.split_whitespace().collect();
                let l1 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let l5 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let l15 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                (l1, l5, l15)
            }
            Err(_) => (0.0, 0.0, 0.0),
        }
    }

    fn read_meminfo() -> HashMap<String, u64> {
        let mut map = HashMap::new();
        let content = match std::fs::read_to_string("/proc/meminfo") {
            Ok(c) => c,
            Err(_) => return map,
        };

        for line in content.lines() {
            if let Some((key, rest)) = line.split_once(':') {
                let val_str = rest.trim().trim_end_matches(" kB").trim();
                if let Ok(kb) = val_str.parse::<u64>() {
                    map.insert(key.trim().to_string(), kb * 1024);
                }
            }
        }
        map
    }

    fn read_mounts() -> Vec<(String, String, String, u64, u64)> {
        let content = match std::fs::read_to_string("/proc/mounts") {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                continue;
            }
            let device = parts[0];
            let mount = parts[1];
            let fs = parts[2];

            if !matches!(
                fs,
                "ext4" | "ext3" | "ext2" | "xfs" | "btrfs" | "f2fs" | "vfat" | "ntfs" | "zfs"
            ) {
                continue;
            }
            if !seen.insert(mount.to_string()) {
                continue;
            }

            if let Some((total, avail)) = statvfs_bytes(mount) {
                if total == 0 {
                    continue;
                }
                let used = total.saturating_sub(avail);
                results.push((
                    mount.to_string(),
                    device.to_string(),
                    fs.to_string(),
                    used,
                    total,
                ));
            }
        }
        results
    }

    fn statvfs_bytes(path: &str) -> Option<(u64, u64)> {
        use std::ffi::CString;
        let c_path = CString::new(path).ok()?;

        unsafe {
            let mut stat: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
                let block_size = stat.f_frsize as u64;
                let total = stat.f_blocks as u64 * block_size;
                let avail = stat.f_bavail as u64 * block_size;
                Some((total, avail))
            } else {
                None
            }
        }
    }

    fn read_net_dev() -> Vec<(String, u64, u64)> {
        let content = match std::fs::read_to_string("/proc/net/dev") {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };

        let mut results = Vec::new();
        for line in content.lines().skip(2) {
            let line = line.trim();
            if let Some((name, rest)) = line.split_once(':') {
                let name = name.trim();
                if name == "lo" {
                    continue;
                }
                let values: Vec<u64> = rest
                    .split_whitespace()
                    .filter_map(|s| s.parse().ok())
                    .collect();
                if values.len() < 10 {
                    continue;
                }
                let rx = values[0];
                let tx = values[8];
                if rx == 0 && tx == 0 {
                    continue;
                }
                results.push((name.to_string(), rx, tx));
            }
        }
        results
    }

    fn read_uptime_secs() -> u64 {
        std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| s.split_whitespace().next().map(String::from))
            .and_then(|s| s.parse::<f64>().ok())
            .map(|f| f as u64)
            .unwrap_or(0)
    }

    pub fn read_processes(sort_by: &str, limit: u32) -> Result<Vec<ProcessInfo>, ServiceError> {
        let entries = std::fs::read_dir("/proc").map_err(|e| ServiceError {
            code: -32000,
            message: format!("Cannot read /proc: {e}"),
        })?;

        let meminfo = read_meminfo();
        let total_mem = meminfo.get("MemTotal").copied().unwrap_or(1);
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 };
        let clock_ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) as f64 };

        let uptime_secs = std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| s.split_whitespace().next().map(String::from))
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(1.0);

        let mut procs = Vec::new();

        for entry in entries.flatten() {
            let name_os = entry.file_name();
            let name_str = name_os.to_string_lossy();
            let pid: u32 = match name_str.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            let stat_content = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let comm_start = match stat_content.find('(') {
                Some(i) => i + 1,
                None => continue,
            };
            let comm_end = match stat_content.rfind(')') {
                Some(i) => i,
                None => continue,
            };

            let proc_name = stat_content[comm_start..comm_end].to_string();
            let after_comm = &stat_content[comm_end + 2..];
            let fields: Vec<&str> = after_comm.split_whitespace().collect();
            if fields.len() < 22 {
                continue;
            }

            let state_char = fields[0];
            let utime: f64 = fields[11].parse().unwrap_or(0.0);
            let stime: f64 = fields[12].parse().unwrap_or(0.0);
            let starttime: f64 = fields[19].parse().unwrap_or(0.0);

            let process_uptime = uptime_secs - (starttime / clock_ticks);
            let cpu_percent = if process_uptime > 0.0 {
                (utime + stime) / clock_ticks / process_uptime * 100.0
            } else {
                0.0
            };

            let mem_bytes = std::fs::read_to_string(format!("/proc/{pid}/statm"))
                .ok()
                .and_then(|c| c.split_whitespace().nth(1).map(String::from))
                .and_then(|s| s.parse::<u64>().ok())
                .map(|pages| pages * page_size)
                .unwrap_or(0);

            let mem_percent = if total_mem > 0 {
                mem_bytes as f64 / total_mem as f64 * 100.0
            } else {
                0.0
            };

            if mem_percent < 0.01 && cpu_percent < 0.01 {
                continue;
            }

            let user = std::fs::read_to_string(format!("/proc/{pid}/loginuid"))
                .ok()
                .and_then(|s| {
                    let uid: u32 = s.trim().parse().ok()?;
                    if uid == 4294967295 {
                        None
                    } else {
                        Some(uid.to_string())
                    }
                })
                .unwrap_or_else(|| "system".to_string());

            let state = match state_char {
                "R" => "Running",
                "S" => "Sleeping",
                "D" => "Disk sleep",
                "Z" => "Zombie",
                "T" => "Stopped",
                _ => state_char,
            }
            .to_string();

            procs.push(ProcessInfo {
                pid,
                name: proc_name,
                cpu_percent,
                mem_percent,
                mem_bytes,
                state,
                user,
            });
        }

        match sort_by {
            "mem" | "memory" => {
                procs.sort_by(|a, b| {
                    b.mem_percent
                        .partial_cmp(&a.mem_percent)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            _ => {
                procs.sort_by(|a, b| {
                    b.cpu_percent
                        .partial_cmp(&a.cpu_percent)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
        }

        procs.truncate(limit as usize);
        Ok(procs)
    }

    pub fn kill_process(pid: u32) -> Result<(), ServiceError> {
        tracing::info!("Killing process PID {pid}");
        let result = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        if result != 0 {
            Err(ServiceError {
                code: -32000,
                message: format!(
                    "Failed to kill PID {pid}: {}",
                    std::io::Error::last_os_error()
                ),
            })
        } else {
            Ok(())
        }
    }
}

// ══════════════════════════════════════════════════════════════════════
// Windows stub (for compilation only — service runs on Linux)
// ══════════════════════════════════════════════════════════════════════

#[cfg(not(unix))]
mod platform {
    use super::*;

    pub fn build_snapshot() -> Result<SystemSnapshot, ServiceError> {
        Ok(SystemSnapshot {
            cpu: CpuInfo {
                overall_percent: 0.0,
                cores: Vec::new(),
                load_avg_1: 0.0,
                load_avg_5: 0.0,
                load_avg_15: 0.0,
            },
            memory: MemoryInfo {
                total_bytes: 0,
                used_bytes: 0,
                usage_percent: 0.0,
                swap_total_bytes: 0,
                swap_used_bytes: 0,
            },
            disks: Vec::new(),
            networks: Vec::new(),
            uptime_secs: 0,
        })
    }

    pub fn read_processes(_sort_by: &str, _limit: u32) -> Result<Vec<ProcessInfo>, ServiceError> {
        Ok(Vec::new())
    }

    pub fn kill_process(_pid: u32) -> Result<(), ServiceError> {
        Err(ServiceError {
            code: -32000,
            message: "kill_process not supported on this platform".to_string(),
        })
    }
}

fn build_snapshot() -> Result<SystemSnapshot, ServiceError> {
    platform::build_snapshot()
}

fn read_processes(sort_by: &str, limit: u32) -> Result<Vec<ProcessInfo>, ServiceError> {
    platform::read_processes(sort_by, limit)
}

fn kill_process(pid: u32) -> Result<(), ServiceError> {
    platform::kill_process(pid)
}
