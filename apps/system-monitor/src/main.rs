//! Yantrik System Monitor — standalone app binary.
//!
//! Polls `system-monitor` service via JSON-RPC IPC every 2 seconds.
//! Falls back to local `sysinfo` crate if the service is unavailable.

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};
use yantrik_app_runtime::prelude::*;
use yantrik_ipc_contracts::system_monitor::{
    CpuInfo, DiskInfo, MemoryInfo, NetworkInterface, ProcessInfo, SystemSnapshot,
};
use yantrik_ipc_transport::SyncRpcClient;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-system-monitor");

    let app = SystemMonitorApp::new().unwrap();
    wire(&app);
    app.run().unwrap();
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

fn kill_process_via_service(pid: u32) -> Result<(), String> {
    let client = SyncRpcClient::for_service("system-monitor");
    client
        .call("sysmon.kill_process", serde_json::json!({ "pid": pid }))
        .map_err(|e| e.message)?;
    Ok(())
}

// ── Local sysinfo fallback ───────────────────────────────────────────

fn snapshot_local() -> SystemSnapshot {
    use sysinfo::System;

    let mut sys = System::new_all();
    sys.refresh_all();

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

    let cpu = CpuInfo {
        overall_percent: overall,
        cores,
        load_avg_1: 0.0,
        load_avg_5: 0.0,
        load_avg_15: 0.0,
    };

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
    };

    let disks: Vec<DiskInfo> = sysinfo::Disks::new_with_refreshed_list()
        .iter()
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

// ── Formatting helpers ───────────────────────────────────────────────

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    }
}

fn format_rate(bps: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    if bps >= MB {
        format!("{:.1} MB/s", bps as f64 / MB as f64)
    } else if bps >= KB {
        format!("{:.1} KB/s", bps as f64 / KB as f64)
    } else {
        format!("{} B/s", bps)
    }
}

fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    if days > 0 {
        format!("{}d {}h {}m", days, hours, mins)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    }
}

// ── Apply snapshot to UI ─────────────────────────────────────────────

fn apply_snapshot(ui: &SystemMonitorApp, snap: &SystemSnapshot) {
    // CPU
    ui.set_cpu_usage(snap.cpu.overall_percent as f32);
    ui.set_load_avg_1(format!("{:.2}", snap.cpu.load_avg_1).into());
    ui.set_load_avg_5(format!("{:.2}", snap.cpu.load_avg_5).into());
    ui.set_load_avg_15(format!("{:.2}", snap.cpu.load_avg_15).into());

    let cores: Vec<CpuCoreData> = snap
        .cpu
        .cores
        .iter()
        .map(|c| CpuCoreData {
            core_id: c.id as i32,
            usage: c.usage_percent as f32,
        })
        .collect();
    ui.set_cpu_cores(ModelRc::new(VecModel::from(cores)));

    // Memory
    ui.set_memory_usage(snap.memory.usage_percent as f32);
    ui.set_memory_used_text(format_bytes(snap.memory.used_bytes).into());
    ui.set_memory_total_text(format_bytes(snap.memory.total_bytes).into());
    let swap_pct = if snap.memory.swap_total_bytes > 0 {
        (snap.memory.swap_used_bytes as f64 / snap.memory.swap_total_bytes as f64) * 100.0
    } else {
        0.0
    };
    ui.set_swap_usage(swap_pct as f32);
    ui.set_swap_used_text(format_bytes(snap.memory.swap_used_bytes).into());
    ui.set_swap_total_text(format_bytes(snap.memory.swap_total_bytes).into());

    // Disks
    let disks: Vec<DiskData> = snap
        .disks
        .iter()
        .map(|d| DiskData {
            mount_point: d.mount_point.clone().into(),
            filesystem: d.filesystem.clone().into(),
            used_bytes: format_bytes(d.used_bytes).into(),
            total_bytes: format_bytes(d.total_bytes).into(),
            usage_percent: d.usage_percent as f32,
        })
        .collect();
    ui.set_disks(ModelRc::new(VecModel::from(disks)));

    // Network
    let nets: Vec<NetworkInterfaceData> = snap
        .networks
        .iter()
        .map(|n| NetworkInterfaceData {
            name: n.name.clone().into(),
            ip_address: "".into(),
            rx_bytes: format_bytes(n.rx_bytes).into(),
            tx_bytes: format_bytes(n.tx_bytes).into(),
            rx_speed: format_rate(n.rx_rate_bps).into(),
            tx_speed: format_rate(n.tx_rate_bps).into(),
        })
        .collect();
    ui.set_network_interfaces(ModelRc::new(VecModel::from(nets)));

    // Uptime
    ui.set_uptime_text(format_uptime(snap.uptime_secs).into());

    // Health (simple heuristic)
    let cpu_ok = snap.cpu.overall_percent < 90.0;
    let mem_ok = snap.memory.usage_percent < 90.0;
    if cpu_ok && mem_ok {
        ui.set_health_status("Healthy".into());
        ui.set_health_score(100.0);
        ui.set_health_summary("All systems nominal".into());
    } else if !cpu_ok && !mem_ok {
        ui.set_health_status("Critical".into());
        ui.set_health_score(20.0);
        ui.set_health_summary("High CPU and memory usage".into());
    } else {
        ui.set_health_status("Degraded".into());
        ui.set_health_score(60.0);
        let msg = if !cpu_ok { "High CPU usage" } else { "High memory usage" };
        ui.set_health_summary(msg.into());
    }
}

fn apply_processes(ui: &SystemMonitorApp, procs: &[ProcessInfo]) {
    let items: Vec<MonitorProcessData> = procs
        .iter()
        .map(|p| MonitorProcessData {
            pid: p.pid as i32,
            name: p.name.clone().into(),
            cpu_percent: p.cpu_percent as f32,
            mem_percent: p.mem_percent as f32,
            status: p.state.clone().into(),
        })
        .collect();
    ui.set_processes(ModelRc::new(VecModel::from(items)));
}

// ── Wire all callbacks ───────────────────────────────────────────────

fn wire(app: &SystemMonitorApp) {
    // Initial snapshot
    let snap = snapshot_via_service().unwrap_or_else(|_| snapshot_local());
    apply_snapshot(app, &snap);

    let procs = processes_via_service("cpu", 50).unwrap_or_else(|_| processes_local("cpu", 50));
    apply_processes(app, &procs);

    // Polling timer — every 2 seconds
    let timer = Timer::default();
    let weak = app.as_weak();
    timer.start(TimerMode::Repeated, std::time::Duration::from_secs(2), move || {
        let Some(ui) = weak.upgrade() else { return };
        let snap = snapshot_via_service().unwrap_or_else(|_| snapshot_local());
        apply_snapshot(&ui, &snap);

        let sort = if ui.get_sort_column() == 1 { "mem" } else { "cpu" };
        let filter = ui.get_process_search().to_string();
        let mut procs = processes_via_service(sort, 100).unwrap_or_else(|_| processes_local(sort, 100));
        if !filter.is_empty() {
            let lower = filter.to_lowercase();
            procs.retain(|p| p.name.to_lowercase().contains(&lower));
        }
        apply_processes(&ui, &procs);
    });
    // Keep timer alive
    let _keep = std::rc::Rc::new(timer);

    // Sort column changed
    {
        let weak = app.as_weak();
        app.on_sort_by_column(move |col| {
            if let Some(ui) = weak.upgrade() {
                ui.set_sort_column(col);
            }
        });
    }

    // Process search
    {
        app.on_process_search_changed(move |_query| {
            // Filtering happens in the timer tick
        });
    }

    // Kill process
    {
        app.on_kill_process(move |pid| {
            if pid <= 0 { return; }
            if let Err(e) = kill_process_via_service(pid as u32) {
                tracing::warn!("Kill process {} via service failed: {}", pid, e);
                // Local fallback: attempt sysinfo kill
                let sys = sysinfo::System::new_all();
                if let Some(proc) = sys.process(sysinfo::Pid::from_u32(pid as u32)) {
                    proc.kill();
                }
            }
        });
    }

    // Force kill
    {
        app.on_force_kill_process(move |pid| {
            if pid <= 0 { return; }
            let sys = sysinfo::System::new_all();
            if let Some(proc) = sys.process(sysinfo::Pid::from_u32(pid as u32)) {
                proc.kill();
            }
        });
    }

    // AI stubs
    app.on_ai_explain_pressed(|| { tracing::info!("AI explain requested (standalone mode)"); });
    app.on_ai_dismiss(|| {});
    app.on_back_pressed(|| { tracing::info!("Back pressed (standalone mode — no-op)"); });
}
