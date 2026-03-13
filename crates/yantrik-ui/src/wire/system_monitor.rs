//! System Monitor wiring — 1-second timer that reads /proc to populate
//! the comprehensive System Monitor screen (screen 23).
//!
//! Reads CPU (per-core), memory, disk, network, and process data directly
//! from /proc. Gracefully degrades if /proc files are unavailable.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, CpuCoreData, DiskData, MonitorProcessData, NetworkInterfaceData};

/// Wire the System Monitor timer and sort callback.
pub fn wire(ui: &App, ctx: &AppContext) {
    // Sort column callback
    let ui_weak = ui.as_weak();
    ui.on_sort_by_column(move |col| {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_mon_sort_column(col);
        }
    });

    // ── AI Explain callback ──
    let bridge = ctx.bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();
    let ui_weak_ai = ui.as_weak();
    let ai_st = ai_state.clone();
    ui.on_mon_ai_explain(move || {
        let Some(ui) = ui_weak_ai.upgrade() else { return };

        // Gather current system snapshot for AI analysis
        let cpu = ui.get_mon_cpu_usage();
        let mem = ui.get_mon_memory_usage();
        let mem_used = ui.get_mon_memory_used_text().to_string();
        let mem_total = ui.get_mon_memory_total_text().to_string();
        let load1 = ui.get_mon_load_avg_1().to_string();
        let load5 = ui.get_mon_load_avg_5().to_string();
        let uptime = ui.get_mon_uptime_text().to_string();

        // Top processes
        let procs = ui.get_mon_processes();
        let mut proc_lines = String::new();
        for i in 0..procs.row_count().min(5) {
            let p = procs.row_data(i).unwrap();
            proc_lines.push_str(&format!(
                "  {} (PID {}) — CPU: {:.1}%, MEM: {:.1}%\n",
                p.name, p.pid, p.cpu_percent, p.mem_percent
            ));
        }

        let context = format!(
            "CPU: {:.1}%, Memory: {:.1}% ({}/{}), Load: {} {} {}, Uptime: {}\nTop processes:\n{}",
            cpu, mem, mem_used, mem_total, load1, load5, uptime, uptime, proc_lines
        );
        let prompt = super::ai_assist::system_analysis_prompt(
            &context,
            "Give a brief health assessment of this system. Is anything unusual?"
        );

        super::ai_assist::ai_request(
            &ui.as_weak(),
            &bridge,
            &ai_st,
            super::ai_assist::AiAssistRequest {
                prompt,
                timeout_secs: 30,
                set_working: Box::new(|ui, v| ui.set_mon_ai_is_working(v)),
                set_response: Box::new(|ui, s| ui.set_mon_ai_response(s.into())),
                get_response: Box::new(|ui| ui.get_mon_ai_response().to_string()),
            },
        );
    });

    let ui_weak_dismiss = ui.as_weak();
    ui.on_mon_ai_dismiss(move || {
        if let Some(ui) = ui_weak_dismiss.upgrade() {
            ui.set_mon_ai_panel_open(false);
        }
    });

    // ── Process search callback ──
    let search_query: std::rc::Rc<RefCell<String>> = std::rc::Rc::new(RefCell::new(String::new()));
    let sq = search_query.clone();
    ui.on_mon_process_search_changed(move |query: SharedString| {
        *sq.borrow_mut() = query.to_string();
    });

    // ── Kill process callback ──
    ui.on_mon_kill_process(move |pid: i32| {
        tracing::info!("System Monitor: sending SIGTERM to PID {}", pid);
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .output();
    });

    // ── Force kill process callback ──
    ui.on_mon_force_kill_process(move |pid: i32| {
        tracing::info!("System Monitor: sending SIGKILL to PID {}", pid);
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .output();
    });

    // 1-second poll timer
    let ui_weak = ui.as_weak();
    let bridge_mon = ctx.bridge.clone();

    // Per-core CPU state: previous idle and total jiffies per core
    let prev_cpu: RefCell<Vec<(u64, u64)>> = RefCell::new(Vec::new());
    // Network state: previous rx/tx bytes per interface
    let prev_net: RefCell<HashMap<String, (u64, u64)>> = RefCell::new(HashMap::new());

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
        let Some(ui) = ui_weak.upgrade() else { return };

        // Only update when System Monitor (screen 23) is active
        if ui.get_current_screen() != 23 {
            return;
        }

        // ── CPU ──
        let (overall_cpu, cores) = read_cpu_usage(&prev_cpu);
        ui.set_mon_cpu_usage(overall_cpu);
        let core_data: Vec<CpuCoreData> = cores
            .iter()
            .enumerate()
            .map(|(i, &usage)| CpuCoreData {
                core_id: i as i32,
                usage,
            })
            .collect();
        ui.set_mon_cpu_cores(ModelRc::new(VecModel::from(core_data)));

        // CPU model + frequency
        let (model, freq) = read_cpu_info();
        ui.set_mon_cpu_model(model.into());
        ui.set_mon_cpu_frequency(freq.into());

        // Load averages
        let (l1, l5, l15) = read_load_avg();
        ui.set_mon_load_avg_1(l1.into());
        ui.set_mon_load_avg_5(l5.into());
        ui.set_mon_load_avg_15(l15.into());

        // ── Memory ──
        let mem = read_meminfo();
        let total = mem.get("MemTotal").copied().unwrap_or(0);
        let _free = mem.get("MemFree").copied().unwrap_or(0);
        let available = mem.get("MemAvailable").copied().unwrap_or(0);
        let buffers = mem.get("Buffers").copied().unwrap_or(0);
        let cached = mem.get("Cached").copied().unwrap_or(0);
        let swap_total = mem.get("SwapTotal").copied().unwrap_or(0);
        let swap_free = mem.get("SwapFree").copied().unwrap_or(0);
        let used = total.saturating_sub(available);
        let swap_used = swap_total.saturating_sub(swap_free);

        let mem_pct = if total > 0 {
            (used as f64 / total as f64 * 100.0) as f32
        } else {
            0.0
        };
        let swap_pct = if swap_total > 0 {
            (swap_used as f64 / swap_total as f64 * 100.0) as f32
        } else {
            0.0
        };

        ui.set_mon_memory_usage(mem_pct);
        ui.set_mon_memory_used_text(format_bytes(used).into());
        ui.set_mon_memory_total_text(format_bytes(total).into());
        ui.set_mon_memory_cached_text(format_bytes(cached).into());
        ui.set_mon_memory_buffers_text(format_bytes(buffers).into());
        ui.set_mon_memory_available_text(format_bytes(available).into());
        ui.set_mon_swap_usage(swap_pct);
        ui.set_mon_swap_used_text(format_bytes(swap_used).into());
        ui.set_mon_swap_total_text(format_bytes(swap_total).into());

        // ── Disk ──
        let disks = read_mounts();
        let disk_data: Vec<DiskData> = disks
            .into_iter()
            .map(|(mount, fs, used_b, total_b)| {
                let pct = if total_b > 0 {
                    (used_b as f64 / total_b as f64 * 100.0) as f32
                } else {
                    0.0
                };
                DiskData {
                    mount_point: mount.into(),
                    filesystem: fs.into(),
                    used_bytes: format_bytes(used_b).into(),
                    total_bytes: format_bytes(total_b).into(),
                    usage_percent: pct,
                }
            })
            .collect();
        ui.set_mon_disks(ModelRc::new(VecModel::from(disk_data)));

        // ── Network ──
        let net_ifaces = read_net_dev(&prev_net);
        let net_data: Vec<NetworkInterfaceData> = net_ifaces
            .into_iter()
            .map(|n| NetworkInterfaceData {
                name: n.name.into(),
                ip_address: n.ip.into(),
                rx_bytes: format_bytes(n.rx_total).into(),
                tx_bytes: format_bytes(n.tx_total).into(),
                rx_speed: format_speed(n.rx_speed).into(),
                tx_speed: format_speed(n.tx_speed).into(),
            })
            .collect();
        ui.set_mon_network_interfaces(ModelRc::new(VecModel::from(net_data)));

        // ── Processes ──
        let sort_col = ui.get_mon_sort_column();
        let total_mem = total; // from meminfo above
        let procs = read_processes(total_mem, sort_col);
        let filter = search_query.borrow().clone();
        let filter_lower = filter.to_lowercase();
        let proc_data: Vec<MonitorProcessData> = procs
            .into_iter()
            .filter(|p| filter_lower.is_empty() || p.name.to_lowercase().contains(&filter_lower))
            .take(15)
            .map(|p| MonitorProcessData {
                pid: p.pid as i32,
                name: p.name.into(),
                cpu_percent: p.cpu_pct,
                mem_percent: p.mem_pct,
                status: p.status.into(),
            })
            .collect();
        ui.set_mon_processes(ModelRc::new(VecModel::from(proc_data)));

        // ── Uptime ──
        ui.set_mon_uptime_text(read_uptime().into());

        // ── AI Workloads ──
        // Model name and tier from settings (already set at startup)
        let model_name = ui.get_settings_llm_api_model().to_string();
        if !model_name.is_empty() {
            ui.set_mon_ai_model_name(model_name.clone().into());
            // Detect tier from model name
            let tier = super::settings::detect_tier_from_name(&model_name);
            ui.set_mon_ai_model_tier(tier.into());
        }
        // Provider health from companion online status
        let online = bridge_mon.is_online();
        if !online {
            ui.set_mon_ai_provider_latency_ms(-1);
        }

        // ── Health Score ──
        let mut score = 100.0f32;
        let mut issues: Vec<&str> = Vec::new();

        // CPU pressure
        if overall_cpu > 90.0 {
            score -= 30.0;
            issues.push("CPU critical");
        } else if overall_cpu > 70.0 {
            score -= 15.0;
            issues.push("CPU high");
        }

        // Memory pressure
        if mem_pct > 90.0 {
            score -= 30.0;
            issues.push("Memory critical");
        } else if mem_pct > 75.0 {
            score -= 15.0;
            issues.push("Memory high");
        }

        // Swap pressure
        if swap_pct > 50.0 {
            score -= 15.0;
            issues.push("Swap pressure");
        }

        // Disk pressure (any disk > 90%)
        let disk_model = ui.get_mon_disks();
        for i in 0..disk_model.row_count() {
            if let Some(d) = disk_model.row_data(i) {
                if d.usage_percent > 90.0 {
                    score -= 15.0;
                    issues.push("Disk critical");
                    break;
                }
            }
        }

        let status = if score >= 80.0 {
            "Healthy"
        } else if score >= 50.0 {
            "Degraded"
        } else {
            "Critical"
        };
        let summary = if issues.is_empty() {
            "All systems nominal".to_string()
        } else {
            issues.join(" \u{00b7} ")
        };

        ui.set_mon_health_status(status.into());
        ui.set_mon_health_score(score.max(0.0));
        ui.set_mon_health_summary(summary.into());
    });

    std::mem::forget(timer);
}

// ═══════════════════════════════════════════════════════════════════════
// /proc readers
// ═══════════════════════════════════════════════════════════════════════

/// Read /proc/stat and compute per-core + overall CPU usage as percentages.
/// Uses delta from previous reading (stored in prev_cpu).
fn read_cpu_usage(prev: &RefCell<Vec<(u64, u64)>>) -> (f32, Vec<f32>) {
    let content = match std::fs::read_to_string("/proc/stat") {
        Ok(c) => c,
        Err(_) => return (0.0, Vec::new()),
    };

    let mut current: Vec<(u64, u64)> = Vec::new(); // (idle, total) per line
    let mut overall_idx = None;

    for line in content.lines() {
        if line.starts_with("cpu") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 5 {
                continue;
            }
            let values: Vec<u64> = parts[1..]
                .iter()
                .filter_map(|s| s.parse::<u64>().ok())
                .collect();
            if values.len() < 4 {
                continue;
            }
            let idle = values[3] + values.get(4).copied().unwrap_or(0); // idle + iowait
            let total: u64 = values.iter().sum();

            if parts[0] == "cpu" {
                overall_idx = Some(current.len());
            }
            current.push((idle, total));
        }
    }

    let mut prev_data = prev.borrow_mut();
    let mut usages: Vec<f32> = Vec::new();

    for (i, &(idle, total)) in current.iter().enumerate() {
        let (prev_idle, prev_total) = if i < prev_data.len() {
            prev_data[i]
        } else {
            (0, 0)
        };

        let delta_total = total.saturating_sub(prev_total);
        let delta_idle = idle.saturating_sub(prev_idle);
        let usage = if delta_total > 0 {
            ((delta_total - delta_idle) as f64 / delta_total as f64 * 100.0) as f32
        } else {
            0.0
        };
        usages.push(usage);
    }

    *prev_data = current;

    let overall = overall_idx
        .and_then(|i| usages.get(i).copied())
        .unwrap_or(0.0);

    // Core usages = everything except the overall "cpu" line
    let core_usages: Vec<f32> = usages
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != overall_idx)
        .map(|(_, &u)| u)
        .collect();

    (overall, core_usages)
}

/// Read CPU model name and frequency from /proc/cpuinfo.
fn read_cpu_info() -> (String, String) {
    let content = match std::fs::read_to_string("/proc/cpuinfo") {
        Ok(c) => c,
        Err(_) => return (String::new(), String::new()),
    };

    let mut model = String::new();
    let mut mhz = String::new();

    for line in content.lines() {
        if model.is_empty() && line.starts_with("model name") {
            if let Some(val) = line.split(':').nth(1) {
                model = val.trim().to_string();
            }
        }
        if mhz.is_empty() && line.starts_with("cpu MHz") {
            if let Some(val) = line.split(':').nth(1) {
                if let Ok(freq) = val.trim().parse::<f64>() {
                    if freq >= 1000.0 {
                        mhz = format!("{:.2} GHz", freq / 1000.0);
                    } else {
                        mhz = format!("{:.0} MHz", freq);
                    }
                }
            }
        }
        if !model.is_empty() && !mhz.is_empty() {
            break;
        }
    }

    (model, mhz)
}

/// Read /proc/loadavg.
fn read_load_avg() -> (String, String, String) {
    match std::fs::read_to_string("/proc/loadavg") {
        Ok(content) => {
            let parts: Vec<&str> = content.split_whitespace().collect();
            (
                parts.first().unwrap_or(&"0.00").to_string(),
                parts.get(1).unwrap_or(&"0.00").to_string(),
                parts.get(2).unwrap_or(&"0.00").to_string(),
            )
        }
        Err(_) => ("0.00".into(), "0.00".into(), "0.00".into()),
    }
}

/// Read /proc/meminfo and return key-value map (values in bytes).
fn read_meminfo() -> HashMap<String, u64> {
    let mut map = HashMap::new();
    let content = match std::fs::read_to_string("/proc/meminfo") {
        Ok(c) => c,
        Err(_) => return map,
    };

    for line in content.lines() {
        if let Some((key, rest)) = line.split_once(':') {
            let key = key.trim().to_string();
            let val_str = rest.trim().trim_end_matches(" kB").trim();
            if let Ok(kb) = val_str.parse::<u64>() {
                map.insert(key, kb * 1024); // Convert kB to bytes
            }
        }
    }

    map
}

/// Read mounted filesystems from /proc/mounts and statvfs each one.
/// Returns (mount_point, filesystem, used_bytes, total_bytes).
fn read_mounts() -> Vec<(String, String, u64, u64)> {
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
        let _device = parts[0];
        let mount = parts[1];
        let fs = parts[2];

        // Filter: only real filesystems
        if !matches!(fs, "ext4" | "ext3" | "ext2" | "xfs" | "btrfs" | "f2fs" | "vfat" | "ntfs" | "zfs" | "tmpfs")
        {
            continue;
        }
        // Skip tmpfs unless it's /tmp or /dev/shm
        if fs == "tmpfs" && !matches!(mount, "/tmp" | "/dev/shm") {
            continue;
        }
        if !seen.insert(mount.to_string()) {
            continue;
        }

        // statvfs via libc
        if let Some((total, avail)) = statvfs_bytes(mount) {
            if total == 0 {
                continue;
            }
            let used = total.saturating_sub(avail);
            results.push((mount.to_string(), fs.to_string(), used, total));
        }
    }

    results
}

/// Call statvfs on a mount point and return (total_bytes, available_bytes).
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

/// Network interface info collected from /proc/net/dev + ip addr.
struct NetIfaceInfo {
    name: String,
    ip: String,
    rx_total: u64,
    tx_total: u64,
    rx_speed: u64, // bytes/sec
    tx_speed: u64,
}

/// Read /proc/net/dev for interface stats. Computes speed from deltas.
fn read_net_dev(prev: &RefCell<HashMap<String, (u64, u64)>>) -> Vec<NetIfaceInfo> {
    let content = match std::fs::read_to_string("/proc/net/dev") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // Read IP addresses (best effort)
    let ips = read_interface_ips();

    let mut prev_data = prev.borrow_mut();
    let mut results = Vec::new();

    for line in content.lines().skip(2) {
        // Skip header lines
        let line = line.trim();
        if let Some((name, rest)) = line.split_once(':') {
            let name = name.trim().to_string();
            // Skip loopback
            if name == "lo" {
                continue;
            }

            let values: Vec<u64> = rest
                .split_whitespace()
                .filter_map(|s| s.parse::<u64>().ok())
                .collect();
            if values.len() < 10 {
                continue;
            }

            let rx_bytes = values[0];
            let tx_bytes = values[8];

            // Skip interfaces with zero traffic
            if rx_bytes == 0 && tx_bytes == 0 {
                continue;
            }

            // Compute speed (delta from previous)
            let (rx_speed, tx_speed) = if let Some(&(prev_rx, prev_tx)) = prev_data.get(&name) {
                (
                    rx_bytes.saturating_sub(prev_rx),
                    tx_bytes.saturating_sub(prev_tx),
                )
            } else {
                (0, 0)
            };
            prev_data.insert(name.clone(), (rx_bytes, tx_bytes));

            let ip = ips.get(&name).cloned().unwrap_or_default();

            results.push(NetIfaceInfo {
                name,
                ip,
                rx_total: rx_bytes,
                tx_total: tx_bytes,
                rx_speed,
                tx_speed,
            });
        }
    }

    results
}

/// Read IP addresses for network interfaces.
/// Uses `ip -4 -o addr show` command (standard on Linux).
/// Returns interface_name -> IP address.
fn read_interface_ips() -> HashMap<String, String> {
    let mut ips = HashMap::new();

    if let Ok(output) = std::process::Command::new("ip")
        .args(["-4", "-o", "addr", "show"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                // Format: "2: eth0    inet 10.0.2.15/24 brd 10.0.2.255 scope global eth0"
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 && parts[2] == "inet" {
                    let iface = parts[1].trim_end_matches(':').to_string();
                    let ip = parts[3].split('/').next().unwrap_or("").to_string();
                    if !ip.starts_with("127.") {
                        ips.insert(iface, ip);
                    }
                }
            }
        }
    }

    ips
}

/// Process info for the process table.
struct ProcInfo {
    pid: u32,
    name: String,
    cpu_pct: f32,
    mem_pct: f32,
    status: String,
}

/// Read /proc/[pid]/stat and /proc/[pid]/statm for top processes.
fn read_processes(total_mem_bytes: u64, sort_column: i32) -> Vec<ProcInfo> {
    let entries = match std::fs::read_dir("/proc") {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 };
    let clock_ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) as f64 };

    // Read system uptime for CPU% calculation
    let uptime_secs = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(String::from))
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(1.0);

    let mut procs = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let pid: u32 = match name_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        let stat_path = format!("/proc/{}/stat", pid);
        let stat_content = match std::fs::read_to_string(&stat_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Parse /proc/[pid]/stat
        // Format: pid (comm) state ... field14=utime field15=stime ...
        // Find the closing ')' to handle names with spaces/parens
        let comm_start = match stat_content.find('(') {
            Some(i) => i + 1,
            None => continue,
        };
        let comm_end = match stat_content.rfind(')') {
            Some(i) => i,
            None => continue,
        };

        let proc_name = stat_content[comm_start..comm_end].to_string();
        let after_comm = &stat_content[comm_end + 2..]; // skip ") "
        let fields: Vec<&str> = after_comm.split_whitespace().collect();
        if fields.len() < 22 {
            continue;
        }

        let state = fields[0].to_string();
        let utime: f64 = fields[11].parse().unwrap_or(0.0);
        let stime: f64 = fields[12].parse().unwrap_or(0.0);
        let starttime: f64 = fields[19].parse().unwrap_or(0.0);

        // CPU usage: (utime + stime) / (uptime - starttime/clock_ticks) * 100 / num_cpus
        let process_uptime = uptime_secs - (starttime / clock_ticks);
        let cpu_pct = if process_uptime > 0.0 {
            ((utime + stime) / clock_ticks / process_uptime * 100.0) as f32
        } else {
            0.0
        };

        // Memory: read from /proc/[pid]/statm
        let mem_pct = if total_mem_bytes > 0 {
            let statm_path = format!("/proc/{}/statm", pid);
            std::fs::read_to_string(&statm_path)
                .ok()
                .and_then(|c| c.split_whitespace().nth(1).map(String::from))
                .and_then(|s| s.parse::<u64>().ok())
                .map(|resident_pages| {
                    (resident_pages * page_size) as f64 / total_mem_bytes as f64 * 100.0
                })
                .unwrap_or(0.0) as f32
        } else {
            0.0
        };

        // Skip kernel threads (zero memory, pid 2 children)
        if mem_pct < 0.01 && cpu_pct < 0.01 {
            continue;
        }

        procs.push(ProcInfo {
            pid,
            name: proc_name,
            cpu_pct,
            mem_pct,
            status: state,
        });
    }

    // Sort by selected column (descending)
    match sort_column {
        1 => procs.sort_by(|a, b| b.mem_pct.partial_cmp(&a.mem_pct).unwrap_or(std::cmp::Ordering::Equal)),
        _ => procs.sort_by(|a, b| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(std::cmp::Ordering::Equal)),
    }

    procs
}

/// Read /proc/uptime and format as human-readable.
fn read_uptime() -> String {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|content| content.split_whitespace().next().map(String::from))
        .and_then(|s| s.parse::<f64>().ok())
        .map(|secs| {
            let total = secs as u64;
            let days = total / 86400;
            let hours = (total % 86400) / 3600;
            let mins = (total % 3600) / 60;
            if days > 0 {
                format!("{}d {}h {}m", days, hours, mins)
            } else if hours > 0 {
                format!("{}h {}m", hours, mins)
            } else {
                format!("{}m", mins)
            }
        })
        .unwrap_or_default()
}

/// Format a byte count as human-readable (KB / MB / GB / TB).
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    const TB: u64 = 1024 * 1024 * 1024 * 1024;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Format bytes/sec as human-readable speed.
fn format_speed(bytes_per_sec: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;

    if bytes_per_sec >= MB {
        format!("{:.1} MB/s", bytes_per_sec as f64 / MB as f64)
    } else if bytes_per_sec >= KB {
        format!("{:.1} KB/s", bytes_per_sec as f64 / KB as f64)
    } else {
        format!("{} B/s", bytes_per_sec)
    }
}
