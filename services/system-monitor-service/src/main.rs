//! System Monitor service — reads /proc for CPU, memory, disk, network, processes.
//!
//! This service is Linux-only (reads /proc, uses libc). On non-Unix platforms
//! it compiles but returns stub data for development purposes.
//!
//! Methods:
//!   sysmon.snapshot    {}                        → SystemSnapshot
//!   sysmon.processes   { sort_by?, limit? }      → Vec<ProcessInfo>
//!   sysmon.kill_process { pid }                  → ()

use yantrik_ipc_contracts::system_monitor::*;
#[cfg(test)]
use yantrik_service_sdk::gate::{self, Authority};
use yantrik_service_sdk::prelude::*;
use yantrik_service_sdk::{Action, Param, PeerCred, Surface, View};

/// The id this surface publishes, and the app a grant for one of its actions is bound to.
const APP: &str = "system-monitor";

fn main() {
    ServiceBuilder::new("system-monitor")
        .handler(SysMonHandler::new())
        .run();
}

struct SysMonHandler {
    /// `app.describe` and `app.act`, dispatched as an app window's are.
    surface: Surface,
}

impl SysMonHandler {
    fn new() -> SysMonHandler {
        SysMonHandler { surface: sysmon_surface() }
    }
}

impl ServiceHandler for SysMonHandler {
    fn service_id(&self) -> &str {
        "system-monitor"
    }

    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        self.handle_from(method, params, None)
    }

    fn handle_from(
        &self,
        method: &str,
        params: serde_json::Value,
        peer: Option<PeerCred>,
    ) -> Result<serde_json::Value, ServiceError> {
        // The agent-facing surface: one call gives live eyesight of the machine — the same
        // numbers the System app draws — without opening a window or reading a screenshot. The
        // ceiling and the mode are read per call, as an app window's dispatch reads them, because
        // a person can change either while this runs.
        if let Some(answer) = self.surface.answer(method, &params, peer) {
            return answer;
        }
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
                // The same reading `app.act` uses, so `0` or a pid past `i32::MAX` never reaches
                // `kill(2)` as a whole process group from this door either.
                let pid = pid_of(&params).map_err(|message| ServiceError { code: -32602, message })?;
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
// Control surface (app.describe / app.act)
// ══════════════════════════════════════════════════════════════════════

/// A byte count a person can read at a glance: "12.4 GB", "512 MB".
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// The machine as data: the snapshot the System app draws, plus the busiest processes, in the
/// one shape every Yantrik surface reports. Reading it costs a caller one call and one line
/// before it decides whether to look closer.
fn describe_view() -> Result<View, ServiceError> {
    let snap = build_snapshot()?;
    // Top few by CPU: the question "what is this machine doing" is almost always "what is using
    // it", and a full process table is the transcript an agent was told to avoid.
    let top = read_processes("cpu", 5).unwrap_or_default();

    let mem_used = human_bytes(snap.memory.used_bytes);
    let mem_total = human_bytes(snap.memory.total_bytes);
    let busiest = top
        .first()
        .map(|p| format!(", busiest {} ({:.0}%)", p.name, p.cpu_percent))
        .unwrap_or_default();
    let summary = format!(
        "System — CPU {:.0}%, memory {} / {} ({:.0}%), load {:.2}, up {}{}",
        snap.cpu.overall_percent,
        mem_used,
        mem_total,
        snap.memory.usage_percent,
        snap.cpu.load_avg_1,
        human_uptime(snap.uptime_secs),
        busiest,
    );

    let disks: Vec<serde_json::Value> = snap
        .disks
        .iter()
        .map(|d| {
            serde_json::json!({
                "mount": d.mount_point,
                "used": human_bytes(d.used_bytes),
                "total": human_bytes(d.total_bytes),
                "percent": (d.usage_percent).round() as i64,
            })
        })
        .collect();
    let processes: Vec<serde_json::Value> = top
        .iter()
        .map(|p| {
            serde_json::json!({
                "pid": p.pid,
                "name": p.name,
                "cpu_percent": (p.cpu_percent * 10.0).round() / 10.0,
                "mem_percent": (p.mem_percent * 10.0).round() / 10.0,
                "user": p.user,
            })
        })
        .collect();

    Ok(View::new(summary)
        .with("cpu_percent", (snap.cpu.overall_percent).round() as i64)
        .with("cores", snap.cpu.cores.len() as i64)
        .with("load", serde_json::json!([snap.cpu.load_avg_1, snap.cpu.load_avg_5, snap.cpu.load_avg_15]))
        .with("memory_used", mem_used)
        .with("memory_total", mem_total)
        .with("memory_percent", (snap.memory.usage_percent).round() as i64)
        .with("swap_used", human_bytes(snap.memory.swap_used_bytes))
        .with("swap_total", human_bytes(snap.memory.swap_total_bytes))
        .with("uptime_secs", snap.uptime_secs as i64)
        .with("disks", serde_json::Value::Array(disks))
        .with("top_processes", serde_json::Value::Array(processes)))
}

/// An uptime a person reads: "3d 4h", "12m".
fn human_uptime(secs: u64) -> String {
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3_600;
    let m = (secs % 3_600) / 60;
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

/// What this service can be asked to do. Ending a process is the one mutating thing it offers,
/// and it destroys work a person cannot get back, so it is graded `dangerous`. This table is
/// what `describe` publishes and what `act` enforces (`published_grade`): the machine's ceiling
/// and the person's mode decide whether a grade runs, and this states the grade.
fn sysmon_actions() -> Vec<Action> {
    vec![
        // `top_processes` is the busiest twenty by CPU. Asked to end `yantrik-notes`, which is
        // idle, a mind found it nowhere on this surface, reached for `terminal.run pgrep` to get
        // a pid, and met a card for a read — twice, before giving up. Finding a process is a
        // read; it is graded as one, and it answers with exactly what `kill_process` takes.
        Action::new("find_process", "Find running processes by name, for a pid")
            .risk("safe")
            .arg(Param::text("name").describe(
                "Part of the program's name or command line, matched without regard to case",
            )),
        Action::new("kill_process", "End a running process by PID")
            .risk("dangerous")
            // An integer, not a number: a pid is whole, and `3.5` used to reach the handler and
            // come back as "needs argument `pid`" for an argument that was plainly there.
            .arg(Param::integer("pid").describe(
                "The process id to end, as shown in top_processes or found by find_process",
            )),
    ]
}

/// The name a process goes by: the program it ran, not the kernel's fifteen-character `comm`.
///
/// `comm` is truncated, so the busy list opened with "yantrik-termina" and "yantrik-calenda"
/// (#35) — names no shell and no memory holds, and what a caller of `kill_process` was told to
/// end. The command line carries the invocation in full; kernel threads have none and keep
/// their `comm`, which is what a person looking for them would type anyway.
fn display_name(cmdline: &str, comm: &str) -> String {
    let argv0 = cmdline.split('\0').next().unwrap_or("");
    let base = argv0.rsplit('/').next().unwrap_or("");
    if base.is_empty() {
        comm.to_string()
    } else {
        base.to_string()
    }
}

/// Every process whose name or command line contains `needle`, newest last.
///
/// A process is named from its command line, not from `/proc/<pid>/comm`, which is truncated to
/// fifteen characters (`yantrik-system-monitor` is `yantrik-system-` there); the truncated
/// `comm` is matched as well, so a needle read off an old list still hits. Kernel threads have
/// no command line and are skipped: nothing on this surface can do anything about them.
fn find_processes(needle: &str) -> Vec<serde_json::Value> {
    let needle = needle.trim().to_lowercase();
    let mut found = Vec::new();
    if needle.is_empty() {
        return found;
    }
    let Ok(entries) = std::fs::read_dir("/proc") else { return found };
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let cmdline = std::fs::read_to_string(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        if cmdline.is_empty() {
            continue;
        }
        let command = cmdline.trim_end_matches('\0').replace('\0', " ");
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        let comm = comm.trim().to_string();
        if !comm.to_lowercase().contains(&needle) && !command.to_lowercase().contains(&needle) {
            continue;
        }
        let uid = std::fs::read_to_string(format!("/proc/{pid}/status"))
            .ok()
            .and_then(|st| {
                st.lines()
                    .find(|l| l.starts_with("Uid:"))
                    .and_then(|l| l.split_whitespace().nth(1).map(str::to_string))
            })
            .unwrap_or_default();
        let shown: String = command.chars().take(160).collect();
        found.push(serde_json::json!({ "pid": pid, "name": display_name(&cmdline, &comm), "command": shown, "uid": uid }));
    }
    found.sort_by_key(|p| p["pid"].as_u64().unwrap_or(0));
    found
}

/// The grade this surface publishes for `action`, from the same table `describe` hands out — so
/// the grade a caller is shown and the grade that is enforced cannot come apart. `None` for an
/// action this service does not have, which has no grade and gets no default.
#[cfg(test)]
fn published_grade(action: &str) -> Option<&'static str> {
    sysmon_actions().into_iter().find(|a| a.name == action).map(|a| a.permission)
}

/// The system monitor's surface, answering on the service's own socket.
///
/// Every call meets the rule an app window's dispatch enforces — the machine's ceiling, then any
/// grant the call carries, then the person's mode — and the argument checks, before a handler
/// runs. This service used to dispatch straight away, so `kill_process` — graded `dangerous` —
/// ended a process on any call to this socket, with no ceiling, no mode and no grant: `yos act
/// system-monitor kill_process` with the window closed, or one raw JSON-RPC line (#153). Now it is
/// refused above the ceiling (the shipped `sensitive` is below it), and under a raised ceiling in
/// `ask` mode it is refused with `GRANT:` until a person has pressed Allow for this exact pid.
fn sysmon_surface() -> Surface {
    let mut surface = Surface::new(APP).socket_name("system-monitor").describe(|| {
        // A machine whose /proc cannot be read still answers describe, and says why, rather than
        // failing the call — and an act whose effect landed is not reported as a failure because
        // the view after it could not be drawn.
        describe_view().unwrap_or_else(|e| {
            View::new(format!("System — could not read this machine's state ({})", e.message))
                .with("error", e.message)
        })
    });
    for spec in sysmon_actions() {
        surface = match spec.name.as_str() {
            "find_process" => surface.action(spec, find_process_action),
            "kill_process" => surface.action(spec, kill_process_action),
            // Published and graded, but no handler here: a mistake in this file.
            // `every_published_action_has_a_handler` keeps it from shipping.
            _ => surface,
        };
    }
    surface
}

/// `find_process {name}`: every process whose name or command line holds `name`.
fn find_process_action(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let name = args["name"].as_str().unwrap_or("").trim();
    if name.is_empty() {
        return Err("`find_process` needs argument `name`".to_string());
    }
    let found = find_processes(name);
    Ok(serde_json::json!({ "query": name, "count": found.len(), "processes": found }))
}

/// `kill_process {pid}`: SIGTERM to one process. The view after it is read by the dispatch, so
/// the caller sees the machine after the kill without a second round trip.
fn kill_process_action(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let pid = pid_of(args)?;
    kill_process(pid).map_err(|e| e.message)?;
    Ok(serde_json::json!({ "killed": pid }))
}

/// The one process `kill_process` may signal, or the refusal.
///
/// `kill(2)` does not read its pid as "a process" at the edges: `0` signals every process in the
/// caller's process group — this service and everything started with it — and a negative pid
/// signals a whole group by its id. A pid above `i32::MAX` became one of those on the way to
/// `libc::kill`, and `0` went straight through. Only `1..=i32::MAX` names one process.
fn pid_of(args: &serde_json::Value) -> Result<u32, String> {
    args["pid"]
        .as_u64()
        .and_then(|p| i32::try_from(p).ok())
        .filter(|p| *p > 0)
        .map(|p| p as u32)
        .ok_or_else(|| "`kill_process` argument `pid` must be a process id: a whole number above zero".to_string())
}

/// `app.act` under a pinned authority, as the socket's dispatch runs it with `Authority::now()`.
/// The tests' door.
#[cfg(test)]
fn act(params: &serde_json::Value, authority: Authority) -> Result<serde_json::Value, ServiceError> {
    sysmon_surface().act(params, None, authority)
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
            available_bytes: available,
            cached_bytes: meminfo.get("Cached").copied().unwrap_or(0),
            buffers_bytes: meminfo.get("Buffers").copied().unwrap_or(0),
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

    /// The window `read_processes` measures CPU over. A process's share of a core is only a
    /// fact about an interval, and the interval this used to divide by was the process's whole
    /// lifetime — the `ps pcpu` number, the average of everything a process has ever done,
    /// which grows more wrong the longer the machine runs (#35: the inside report read
    /// `yantrik-ui 92.7%` where `ps` and this code agreed with each other and neither agreed
    /// with the machine). Half a second per call is the order `top` samples at, and the app
    /// polls every two, so the window never delays a reading that is wanted sooner.
    const CPU_SAMPLE_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);

    /// The `comm`, the state character and the CPU used in total so far (utime + stime, in
    /// ticks) from a `/proc/<pid>/stat` line. `comm` sits in parentheses and can hold spaces
    /// and closing parens, so the numbered fields are counted after the *last* `)` rather
    /// than by splitting the whole line on whitespace.
    pub fn parse_stat(content: &str) -> Option<(&str, &str, f64)> {
        let comm_start = content.find('(')?;
        let comm_end = content.rfind(')')?;
        if comm_end <= comm_start {
            return None;
        }
        // A line that ends at `comm` has nothing left to read, and slicing for it would panic.
        let fields: Vec<&str> = content.get(comm_end + 2..)?.split_whitespace().collect();
        // state is the line's 3rd field and utime and stime its 14th and 15th, so the 1st,
        // 12th and 13th counted from here.
        if fields.len() < 13 {
            return None;
        }
        let utime: f64 = fields[11].parse().unwrap_or(0.0);
        let stime: f64 = fields[12].parse().unwrap_or(0.0);
        Some((&content[comm_start + 1..comm_end], fields[0], utime + stime))
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

        // The whole table as the sample window opens: what each process is, and the CPU it had
        // used by the time this pass read its line. (Reads of a busy table take milliseconds,
        // and the ticks of each line are that much older than the window's start — as in
        // `top`, the skew is ignored.)
        let mut opened = Vec::new();
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
            let Some((comm, state_char, cpu_ticks)) = parse_stat(&stat_content) else {
                continue;
            };

            let name = display_name(
                &std::fs::read_to_string(format!("/proc/{pid}/cmdline")).unwrap_or_default(),
                comm,
            );

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

            opened.push((cpu_ticks, ProcessInfo {
                pid,
                name,
                cpu_percent: 0.0,
                mem_percent,
                mem_bytes,
                state,
                user,
            }));
        }

        let window_start = std::time::Instant::now();
        std::thread::sleep(CPU_SAMPLE_WINDOW);

        // The window closes on each line as it is read again: the ticks used inside it, over
        // that much time, is the share of one core the process is taking right now. A process
        // that died mid-window has no second reading and drops out — the caller re-reads the
        // table seconds later anyway.
        let mut procs = Vec::new();
        for (cpu_ticks_at_start, mut info) in opened {
            let Ok(content) = std::fs::read_to_string(format!("/proc/{}/stat", info.pid)) else {
                continue;
            };
            let Some((_, _, cpu_ticks_now)) = parse_stat(&content) else {
                continue;
            };
            // A pid recycled inside the window is the one thing that can put the counter
            // backwards; nothing else can, and nothing is served by a negative reading.
            let ticks_used = (cpu_ticks_now - cpu_ticks_at_start).max(0.0);
            let seconds = window_start.elapsed().as_secs_f64().max(0.001);
            info.cpu_percent = if clock_ticks > 0.0 {
                ticks_used / clock_ticks / seconds * 100.0
            } else {
                0.0
            };

            if info.mem_percent < 0.01 && info.cpu_percent < 0.01 {
                continue;
            }

            procs.push(info);
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
                available_bytes: 0,
                cached_bytes: 0,
                buffers_bytes: 0,
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

/// The door #153 found open, knocked on the way a caller knocks: `app.act` through this service's
/// handler, with the ceiling and the mode in the files the shell writes. Written against nothing
/// but `SysMonHandler::handle`, so it says the same thing about a handler that never checked —
/// which ended the process.
#[cfg(all(test, unix))]
mod through_the_handler {
    use super::*;
    use std::process::{Child, Command};
    use std::sync::Mutex;
    use std::time::Duration;

    /// `HOME` is process-wide, and these tests point it at settings of their own.
    static HOME: Mutex<()> = Mutex::new(());

    /// A home whose settings and mode file say `ceiling` and `mode`, in the shape the shell
    /// writes them.
    fn home_with(ceiling: &str, mode: &str) -> std::path::PathBuf {
        let home = std::env::temp_dir().join(format!("sysmon-153-{}-{ceiling}-{mode}", std::process::id()));
        let config = home.join(".config/yantrik");
        std::fs::create_dir_all(&config).expect("a config dir of our own");
        std::fs::write(config.join("settings.yaml"), format!("tool_permission: {ceiling}\n")).unwrap();
        std::fs::write(config.join("mind-mode.json"), format!("{{\"mode\":\"{mode}\",\"session_rules\":[]}}")).unwrap();
        home
    }

    /// A process of our own to end. Nothing here ever kills what it did not make.
    fn sleeper() -> Child {
        Command::new("sleep").arg("120").spawn().expect("a sleep of our own")
    }

    /// Still running, after the time a SIGTERM that had been sent would take to land.
    fn still_running(child: &mut Child) -> bool {
        std::thread::sleep(Duration::from_millis(300));
        child.try_wait().expect("wait on our own child").is_none()
    }

    fn reap(mut child: Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    fn kill_on_the_socket(pid: u32) -> Result<serde_json::Value, ServiceError> {
        SysMonHandler::new().handle(
            "app.act",
            serde_json::json!({ "action": "kill_process", "args": { "pid": pid } }),
        )
    }

    /// A person who raised the ceiling to `dangerous` let a mind *ask* to end a process; in
    /// `ask` mode that is a card and a person's Allow, not a raw socket line.
    #[test]
    fn kill_process_in_ask_mode_without_a_grant_ends_nothing() {
        let _home = HOME.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("HOME", home_with("dangerous", "ask"));
        let mut child = sleeper();

        let answer = kill_on_the_socket(child.id());
        let alive = still_running(&mut child);
        reap(child);

        let err = answer.expect_err("a dangerous act ran in ask mode with no grant");
        assert!(
            err.message.starts_with("GRANT: system-monitor.kill_process is graded `dangerous`")
                && err.message.contains("ask mode"),
            "{}",
            err.message
        );
        assert!(alive, "the refusal was reported, and the process was ended anyway");
    }

    /// Under the shipped ceiling — `sensitive`, VM 520's setting — no mode reaches `dangerous`.
    #[test]
    fn kill_process_above_the_ceiling_ends_nothing_even_in_bypass() {
        let _home = HOME.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("HOME", home_with("sensitive", "bypass"));
        let mut child = sleeper();

        let answer = kill_on_the_socket(child.id());
        let alive = still_running(&mut child);
        reap(child);

        let err = answer.expect_err("a dangerous act ran above a sensitive ceiling");
        assert!(err.message.starts_with("CEILING:"), "{}", err.message);
        assert!(alive, "the refusal was reported, and the process was ended anyway");
    }
}

/// The same rule with the ceiling, the mode and the shell's grant store pinned per case.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Child, Command};
    use std::time::Duration;

    fn at(ceiling: &str, mode: &str) -> Authority {
        Authority { ceiling: ceiling.into(), mode: gate::Mode::named(mode), granted: false }
    }

    fn kill(pid: u32, grant: Option<&str>) -> serde_json::Value {
        let mut params = serde_json::json!({ "action": "kill_process", "args": { "pid": pid } });
        if let Some(grant) = grant {
            params["grant"] = grant.into();
        }
        params
    }

    fn sleeper() -> Child {
        Command::new("sleep").arg("120").spawn().expect("a sleep of our own")
    }

    fn still_running(child: &mut Child) -> bool {
        std::thread::sleep(Duration::from_millis(300));
        child.try_wait().expect("wait on our own child").is_none()
    }

    fn reap(mut child: Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    /// A stand-in for the shell's store: `ok-kill-<pid>` is a person's Allow for exactly
    /// `system-monitor.kill_process {"pid": <pid>}`, good once; anything else is refused in the
    /// shell's words. Installed once, because the spender is process-wide, as the shell's is.
    fn spend_through_a_stand_in_shell() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let spent = std::sync::Mutex::new(std::collections::HashSet::<String>::new());
            gate::spend_grants_with(move |id, app, action, args| {
                let Some(pid) = id.strip_prefix("ok-kill-").and_then(|p| p.parse::<u64>().ok()) else {
                    return Err(format!("no approval request `{id}`."));
                };
                // The whole arguments, as the shell binds them: `{"pid": <pid>}` and nothing else.
                if app != APP || action != "kill_process" || *args != serde_json::json!({ "pid": pid }) {
                    return Err(format!("`{id}` was approved for another call, and this call carries {args}."));
                }
                let mut spent = spent.lock().unwrap_or_else(|e| e.into_inner());
                if !spent.insert(id.to_string()) {
                    return Err(format!("`{id}` was already used."));
                }
                Ok(())
            });
        });
    }

    /// The ceiling is the machine's wall: not bypass, not a grant, not both. And a grant it
    /// refused is still whole afterwards (#154): with the ceiling raised it ends the process it
    /// was given for, once.
    #[test]
    fn above_the_ceiling_kill_process_is_refused_whatever_the_grant_or_mode() {
        spend_through_a_stand_in_shell();
        let mut child = sleeper();
        let pid = child.id();
        let grant = format!("ok-kill-{pid}");

        for (mode, carried) in [("bypass", None), ("ask", Some(grant.as_str())), ("bypass", Some(grant.as_str()))] {
            let err = act(&kill(pid, carried), at("sensitive", mode)).expect_err("above the ceiling");
            assert!(
                err.message.starts_with("CEILING: system-monitor.kill_process is graded `dangerous`"),
                "{mode}, grant={carried:?}: {}",
                err.message
            );
            assert_eq!(err.code, -32602, "a refusal, not a transport failure");
        }
        if !still_running(&mut child) {
            panic!("a process was ended above the ceiling");
        }

        let answer = act(&kill(pid, Some(&grant)), at("dangerous", "ask")).expect("the grant was never spent");
        assert_eq!(answer["result"]["killed"], serde_json::json!(pid));
        assert_eq!(child.wait().expect("reaped").signal(), Some(libc::SIGTERM));
        let err = act(&kill(pid, Some(&grant)), at("dangerous", "ask")).unwrap_err();
        assert!(err.message.starts_with("GRANT:") && err.message.contains("already used"), "{}", err.message);
    }

    /// Under a raised ceiling, `dangerous` is above what every mode but bypass runs unasked, so
    /// it is refused with `GRANT:` — the refusal `yos act` turns into a card — until a person's
    /// Allow for this pid rides on the call.
    #[test]
    fn in_ask_mode_kill_process_needs_a_grant_and_runs_with_one() {
        spend_through_a_stand_in_shell();
        let mut child = sleeper();
        let pid = child.id();

        let err = act(&kill(pid, None), at("dangerous", "ask")).unwrap_err();
        assert!(err.message.starts_with("GRANT: system-monitor.kill_process is graded `dangerous`"), "{}", err.message);
        assert!(err.message.contains("ask mode") && err.message.contains("request_approval"), "{}", err.message);
        let err = act(&kill(pid, None), at("dangerous", "auto")).unwrap_err();
        assert!(err.message.starts_with("GRANT:") && err.message.contains("auto mode"), "{}", err.message);
        let err = act(&kill(pid, None), at("dangerous", "plan")).unwrap_err();
        assert!(err.message.contains("plan mode") && err.message.contains("raises no card"), "{}", err.message);
        // A person's Allow for another process is not one for this.
        let err = act(&kill(pid, Some(&format!("ok-kill-{}", pid + 1))), at("dangerous", "ask")).unwrap_err();
        assert!(err.message.starts_with("GRANT:") && err.message.contains("does not authorise"), "{}", err.message);
        if !still_running(&mut child) {
            panic!("a process was ended without the person's Allow");
        }

        act(&kill(pid, Some(&format!("ok-kill-{pid}"))), at("dangerous", "ask"))
            .expect("a person's Allow for this pid ends it");
        assert_eq!(child.wait().expect("reaped").signal(), Some(libc::SIGTERM));
    }

    /// An agent token travels beside `args`, never among them. One a caller put among them is
    /// taken out before the grant is spent, so the person's Allow for `{"pid": <pid>}` is the
    /// grant for this call — and the token is never part of what a grant is bound to.
    #[test]
    fn an_agent_token_among_the_arguments_is_not_what_a_grant_is_bound_to() {
        spend_through_a_stand_in_shell();
        let child = sleeper();
        let pid = child.id();
        let params = serde_json::json!({
            "action": "kill_process",
            "args": { "pid": pid, "agent_token": "smuggled" },
            "agent_token": "tok-7f3a",
            "grant": format!("ok-kill-{pid}"),
        });
        let answer = act(&params, at("dangerous", "ask"));
        let mut child = child;
        if answer.is_err() {
            let _ = child.kill();
        }
        let status = child.wait().expect("reaped");
        let answer = answer.unwrap_or_else(|e| panic!("the token was bound into the grant: {}", e.message));
        assert_eq!(answer["result"]["killed"], serde_json::json!(pid));
        assert_eq!(status.signal(), Some(libc::SIGTERM));
        assert!(!answer.to_string().contains("smuggled"), "{answer}");
    }

    /// Finding a process is a read, graded `safe`, and asks nobody in any mode.
    #[test]
    fn a_read_runs_in_every_mode() {
        let mut child = sleeper();
        for mode in ["plan", "ask", "auto", "bypass"] {
            let answer = act(
                &serde_json::json!({ "action": "find_process", "args": { "name": "sleep" } }),
                at("sensitive", mode),
            )
            .unwrap_or_else(|e| panic!("{mode}: {}", e.message));
            assert_eq!(answer["accepted"], serde_json::json!(true), "{mode}");
        }
        assert!(still_running(&mut child));
        reap(child);
    }

    /// `describe` takes no authority, and the grades it publishes are the grades `act` enforces.
    #[test]
    fn describe_needs_nothing_and_publishes_the_grades_act_enforces() {
        let described = SysMonHandler::new().handle("app.describe", serde_json::json!({})).expect("describe");
        let actions = described["actions"].as_array().expect("actions");
        assert_eq!(actions.len(), sysmon_actions().len());
        for a in actions {
            let name = a["name"].as_str().unwrap();
            assert_eq!(a["permission"].as_str(), published_grade(name), "{name}");
        }
        assert_eq!(published_grade("kill_process"), Some("dangerous"));
        assert_eq!(published_grade("find_process"), Some("safe"));
    }

    /// Every action `describe` offers reaches a handler past the gate, and an action it does not
    /// offer is answered as that before any grant is looked at.
    #[test]
    fn every_published_action_has_a_handler() {
        for spec in sysmon_actions() {
            // No arguments, so no handler can do anything; each has to say what it needs.
            let err = act(&serde_json::json!({ "action": spec.name, "args": {} }), at("dangerous", "bypass"))
                .expect_err("no arguments were given");
            assert!(!err.message.starts_with("unknown action"), "{}: {}", spec.name, err.message);
        }
        let err = act(
            &serde_json::json!({ "action": "reboot", "args": {}, "grant": "ok-kill-1" }),
            at("dangerous", "ask"),
        )
        .unwrap_err();
        assert_eq!(err.code, -32602);
        assert_eq!(err.message, "unknown action `reboot`; this app offers: find_process, kill_process");
    }

    /// A pid is an integer, and anything else is refused by the dispatch before a signal could be
    /// sent — above the gate, so the refusal is for the argument and not for the grade.
    #[test]
    fn kill_process_takes_a_whole_pid_and_refuses_anything_else_before_it_runs() {
        let mut child = sleeper();
        let pid = child.id();
        for (given, why) in [
            (serde_json::json!(format!("{pid}x")), "a string arrived"),
            (serde_json::json!(format!(" {pid}")), "a string arrived"),
            (serde_json::json!(pid as f64 + 0.5), "a number with a fraction arrived"),
            (serde_json::json!(true), "a boolean arrived"),
        ] {
            let err = act(
                &serde_json::json!({ "action": "kill_process", "args": { "pid": given } }),
                at("dangerous", "bypass"),
            )
            .unwrap_err();
            assert_eq!(err.code, -32602);
            assert!(
                err.message.starts_with("`kill_process` argument `pid` must be an integer, and ")
                    && err.message.contains(why),
                "{}",
                err.message
            );
        }
        let err = act(
            &serde_json::json!({ "action": "kill_process", "args": { "pid": -1 } }),
            at("dangerous", "bypass"),
        )
        .unwrap_err();
        assert_eq!(err.message, "`kill_process` argument `pid` must be a process id: a whole number above zero");
        assert!(still_running(&mut child), "a refusal ended the process anyway");

        // The pid as the text it is written in converts without loss: the handler reads the
        // integer, and our own sleeper is ended.
        let answer = act(
            &serde_json::json!({ "action": "kill_process", "args": { "pid": pid.to_string() } }),
            at("dangerous", "bypass"),
        )
        .expect("a pid written as text is the pid");
        assert_eq!(answer["result"]["killed"], serde_json::json!(pid));
        assert_eq!(child.wait().expect("reaped").signal(), Some(libc::SIGTERM));
    }

    /// The pids `kill(2)` reads as a whole process group are never handed to it. Checked on the
    /// reading alone, so a regression here fails this test instead of signalling the test's own
    /// process group.
    #[test]
    fn a_pid_that_names_a_process_group_is_refused() {
        let refusal = "`kill_process` argument `pid` must be a process id: a whole number above zero";
        for pid in [serde_json::json!(0), serde_json::json!(-1), serde_json::json!(1u64 << 31), serde_json::json!(u64::MAX)] {
            assert_eq!(pid_of(&serde_json::json!({ "pid": pid })), Err(refusal.to_string()), "{pid}");
        }
        assert_eq!(pid_of(&serde_json::json!({ "pid": 1 })), Ok(1));
        assert_eq!(pid_of(&serde_json::json!({ "pid": i32::MAX })), Ok(i32::MAX as u32));
    }

    /// An act decided on a view the machine has left is refused before the signal, as on a window.
    #[test]
    fn kill_process_on_a_stale_revision_ends_nothing() {
        let mut child = sleeper();
        let err = act(
            &serde_json::json!({
                "action": "kill_process",
                "args": { "pid": child.id() },
                "expect_revision": "0000000000000000",
            }),
            at("dangerous", "bypass"),
        )
        .unwrap_err();
        assert_eq!(err.code, -32602);
        assert!(err.message.starts_with("STALE: this app is at revision "), "{}", err.message);
        assert!(still_running(&mut child), "the refusal was reported, and the process was ended anyway");
        reap(child);
    }

    /// Each act has its own name, on the service's own socket, not one fixed id for every call.
    #[test]
    fn every_act_gets_its_own_action_id() {
        let find = || {
            act(&serde_json::json!({ "action": "find_process", "args": { "name": "sleep" } }), at("sensitive", "ask"))
                .expect("a read runs")["action_id"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let (first, second) = (find(), find());
        assert_ne!(first, second);
        assert!(first.starts_with("system-monitor#"), "{first}");
    }

    /// The surface declares nothing the dispatch cannot check.
    #[test]
    fn the_surface_is_declared_soundly() {
        assert!(sysmon_surface().registry().problems().is_empty());
    }
}

/// The process list says what a process is using CPU *now*, and calls it by its whole name.
///
/// The inside report caught both halves in one line: `yantrik-ui` listed at 92.7% while top,
/// the same moment, had 5.0% — the old reading divided a process's total CPU by its own age,
/// the `ps pcpu` lifetime average, which grows more wrong the longer the machine runs — and
/// names like `"yantrik-termina"`, the kernel's fifteen-character `comm` where a mind reaches
/// for `kill_process`. Both faults are tested against real children of the test's own, so no
/// mock decides what a process is doing.
#[cfg(all(test, unix))]
mod process_readings {
    use super::*;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    /// A child of ours that is ended when the test ends, panic included. A busy child left
    /// running is worse than a leaked file handle: it holds the test binary's stderr pipe
    /// open, so a *failing* test hangs the runner's output instead of reporting the failure.
    /// Both pipes are nulled for the same reason — a child that has none cannot block one.
    struct Ended(Option<Child>);

    impl Ended {
        fn spawn(command: &mut Command) -> Ended {
            Ended(Some(
                command
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("a child of our own"),
            ))
        }

        fn pid(&self) -> u32 {
            self.0.as_ref().expect("our child, until it is ended").id()
        }
    }

    impl Drop for Ended {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    /// The whole table, re-read until it agrees about our child. Each read takes its own
    /// half-second sample, so this loop is the wait and the deadline is slack, not a guess
    /// about when the child gets there.
    fn wait_until_listed(pid: u32, agrees: impl Fn(&ProcessInfo) -> bool) -> ProcessInfo {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(list) = read_processes("cpu", 100_000) {
                if let Some(found) = list.into_iter().find(|p| p.pid == pid && agrees(p)) {
                    return found;
                }
            }
            assert!(Instant::now() < deadline, "pid {pid} never showed up as asked");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// A copy of `yes` under a name longer than the kernel's `comm` can hold — and whose
    /// first fifteen characters are not its whole, so a truncated name cannot pass by luck.
    fn long_named_spinner(tag: &str) -> (Ended, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sysmon-35-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a directory of our own");
        let program = dir.join("yos-monitor-current-cpu-check");
        // A link, not a copy. A copy is a file this process holds open for writing, and a sibling
        // test forking in that moment carries the write descriptor into its child until the child
        // execs; running the copy inside that window fails with ETXTBSY, "Text file busy" (four CI
        // runs on main did). A link is never open for writing, and exec through it still gives the
        // child the long name as argv[0] and the truncated one as its `comm`.
        let _ = std::fs::remove_file(&program);
        std::os::unix::fs::symlink("/usr/bin/yes", &program)
            .expect("a `yes` that answers to a long name");
        let mut spinner = Command::new(&program);
        (Ended::spawn(&mut spinner), program)
    }

    /// A process that keeps a spin going is named by its program, whole. The list used to
    /// carry `comm` instead, so a caller of `kill_process` was told to end "yos-monitor-cur"
    /// — a name no program answers to.
    #[test]
    fn a_process_is_named_by_its_whole_program() {
        let (child, program) = long_named_spinner("name");
        let found = wait_until_listed(child.pid(), |_| true);
        let _ = std::fs::remove_dir_all(program.parent().unwrap());

        assert_eq!(found.name, "yos-monitor-current-cpu-check");
        // The spinner really was spinning while the list looked at it: an assertion about a
        // process that had already exited would pass on nothing.
        assert!(found.cpu_percent > 20.0, "spinner read {}%", found.cpu_percent);
    }

    /// Three seconds of work and then sleep reads as *sleep*. The old number divided the
    /// three seconds by the child's age — barely older than the three seconds, so tens of
    /// percent at the best moment to look, when the machine's own answer was zero.
    #[test]
    fn cpu_percent_reads_the_sample_window_not_the_lifetime() {
        // The list's own filter (which drops what uses no CPU and under 0.01% of memory)
        // would hide a sleeping child entirely, so it holds half a percent of this machine's
        // RAM — at least 100 MB — allocated *before* it starts working. That way the state
        // the test waits for, asleep with the allocation held, can only be seen after the
        // work phase is over: the test never catches a running state by luck and never
        // sleeps a fixed guess. (A bash string substitution would not do it — bash frees the
        // pages before the test can read them; `bytearray` is real, resident memory.)
        let hold = 100_000_000u64.max(
            std::fs::read_to_string("/proc/meminfo")
                .expect("this machine has a /proc/meminfo")
                .lines()
                .find_map(|line| line.strip_prefix("MemTotal:"))
                .and_then(|rest| rest.trim().trim_end_matches(" kB").trim().parse::<u64>().ok())
                .map(|kb| kb * 1024 / 200)
                .unwrap_or(100_000_000),
        );
        let child = Ended::spawn(
            Command::new("python3")
                .arg("-c")
                .arg(format!(
                    "import time\n\
                     held = bytearray({hold})\n\
                     step = 1 << 21\n\
                     for i in range(0, {hold}, step): held[i] = 1\n\
                     end = time.time() + 3\n\
                     while time.time() < end: pass\n\
                     time.sleep(120)\n"
                )),
        );
        // Sleeping with a half percent of RAM resident can only be the far end of the
        // script: the allocation precedes the work, and the work precedes the sleep.
        let found = wait_until_listed(child.pid(), |p| {
            p.state == "Sleeping" && p.mem_percent >= 0.4
        });

        assert!(
            found.cpu_percent < 10.0,
            "{} is asleep but reads {}% — that is its lifetime average, not its use",
            found.name,
            found.cpu_percent
        );
    }

    /// `comm` sits in parentheses and can hold spaces and closing parens, so the numbered
    /// fields are counted after the *last* `)`. These lines keep that rule on record.
    #[test]
    fn a_stat_line_is_read_from_after_its_comm() {
        let (comm, state, ticks) = platform::parse_stat(
            "42 (my (odd) name) R 1 42 42 0 -1 0 100 50 0 0 300 100 0 0 20 0 0 0",
        )
        .expect("a stat line as /proc writes it");
        assert_eq!(comm, "my (odd) name");
        assert_eq!(state, "R");
        assert_eq!(ticks, 400.0);

        assert!(platform::parse_stat("42 (gone").is_none(), "no closing paren for comm");
        assert!(platform::parse_stat("42 (x) S 1 42").is_none(), "ends before the CPU fields");
    }

    /// The name comes from the command line; `comm` is only what a kernel thread, which has
    /// no command line, is called.
    #[test]
    fn a_name_is_the_program_and_not_the_paths_around_it() {
        assert_eq!(
            display_name("/usr/bin/yantrik-terminal\0--wait\0", "yantrik-termina"),
            "yantrik-terminal"
        );
        assert_eq!(
            display_name("./run_beyond_comm_fifteen\0", "run_beyond_comm"),
            "run_beyond_comm_fifteen"
        );
        assert_eq!(display_name("", "kworker/0:1H-kblockd"), "kworker/0:1H-kblockd");
        assert_eq!(display_name("\0\0", "sh"), "sh");
    }
}
