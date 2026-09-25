//! Yantrik System Monitor — standalone app binary.
//!
//! Polls `system-monitor` service via JSON-RPC IPC every 2 seconds.
//! Falls back to the local `sysinfo` crate if the service is unavailable, and says so — on
//! screen and in `describe` — because a reading taken by the fallback is not the same reading.

mod outcome;

use std::cell::RefCell;
use std::rc::Rc;

use outcome::{Liveness, Observed, Provenance, Signal, Source};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use yantrik_app_runtime::prelude::*;
use yantrik_ipc_contracts::system_monitor::{
    CpuInfo, DiskInfo, MemoryInfo, NetworkInterface, ProcessInfo, SystemSnapshot,
};
use yantrik_ipc_transport::SyncRpcClient;

slint::include_modules!();

/// What the window currently cannot do, and what it currently cannot measure.
///
/// Both belong on the same one-line strip and neither may overwrite the other, so they are held
/// apart here and composed on the way to the screen. Shared by the poll timer, the kill path and
/// `describe`, all three of which run on the UI thread — the control surface's closures are
/// dispatched there on purpose (see `control.rs`, "Threading"), so an `Rc` is the right handle.
#[derive(Default)]
struct Status {
    /// Where the numbers on screen came from, as of the most recent poll.
    reading: Option<Provenance>,
    /// Why the last thing someone asked for could not be done. Cleared by the next one that can.
    failure: Option<String>,
}

/// Put the two kinds of bad news on screen, and nothing when there is none.
fn show_status(ui: &SystemMonitorApp, status: &Rc<RefCell<Status>>) {
    let state = status.borrow();
    let degraded = state.reading.as_ref().and_then(|p| p.notice());
    ui.set_notice(outcome::compose_notice(state.failure.as_deref(), degraded.as_deref()).into());
}

/// Fill the agent rail from the numbers already on screen.
///
/// This app's whole content is measurements, so its context is the readings themselves. None
/// of it is asked of a model: the machine knows what it is doing. The one thing a model adds
/// is what the numbers MEAN, and that is the only suggestion.
fn refresh_agent_rail(ui: &SystemMonitorApp) {
    let context = vec![
        AgentContextItem {
            id: "cpu".into(),
            label: format!("CPU {:.0}%", ui.get_cpu_usage()).into(),
            detail: "processor".into(),
            source: "file".into(),
        },
        AgentContextItem {
            id: "mem".into(),
            label: format!(
                "Memory {} of {}",
                ui.get_memory_used_text(),
                ui.get_memory_total_text()
            )
            .into(),
            detail: format!("{:.0}% in use", ui.get_memory_usage()).into(),
            source: "file".into(),
        },
    ];
    ui.set_agent_context(ModelRc::new(VecModel::from(context)));

    let reach = companion::reach();
    let mut next: Vec<AgentSuggestion> = Vec::new();
    if reach == companion::Reach::Ready {
        next.push(AgentSuggestion {
            id: "explain".into(),
            label: "Is anything wrong?".into(),
            detail: "reads the current numbers".into(),
            icon: "spark".into(),
            running: ui.get_proposal_working(),
            proposes: false,
        });
    }
    ui.set_agent_suggestions(ModelRc::new(VecModel::from(next)));
    ui.set_agent_unavailable(match reach.hint() {
        Some(hint) => hint.into(),
        None => SharedString::new(),
    });
}

fn main() {
    init_tracing("yantrik-system-monitor");

    // One window per app: a second launch defers to the running one (the shell focuses it).
    let Some(_instance) = instance::claim("system-monitor") else { return };

    let app = SystemMonitorApp::new().unwrap();
    // The window's title bar is the app's own (#256).
    yantrik_app_runtime::window_chrome!(app);

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    // Held for the life of the window: a dropped Slint timer stops. This one was bound inside
    // `wire` and dropped at the end of it, so the window took its readings once at startup and
    // then showed them, unchanging, for as long as it was open.
    let _refresh_timer = wire(&app);
    // ── The agent layer ──
    {
        let weak = app.as_weak();
        app.on_agent_suggestion_activated(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            if id != "explain" {
                return;
            }
            // The same question the header's AI Explain asks, from the same readings.
            let prompt = machine_question(&ui);
            ui.set_proposal_working(true);
            ui.set_proposal(AgentProposal {
                title: "The machine right now".into(),
                source: "from the current readings".into(),
                ..Default::default()
            });
            let back = ui.as_weak();
            std::thread::spawn(move || {
                let outcome = companion::ask(&prompt);
                let _ = back.upgrade_in_event_loop(move |ui| {
                    ui.set_proposal_working(false);
                    match outcome {
                        Ok(text) => ui.set_proposal(AgentProposal {
                            title: "The machine right now".into(),
                            body: text.into(),
                            source: "from the current readings".into(),
                            verb: "Close".into(),
                            ..Default::default()
                        }),
                        Err(e) => ui.set_proposal(AgentProposal {
                            title: "The companion did not answer".into(),
                            body: format!("{e}").into(),
                            verb: "Close".into(),
                            ..Default::default()
                        }),
                    }
                });
            });
        });
    }
    {
        let weak = app.as_weak();
        app.on_proposal_dismissed(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_proposal(AgentProposal::default());
            }
        });
    }
    app.on_proposal_applied(|| {});
    app.on_agent_context_activated(|_| {});
    refresh_agent_rail(&app);

    run_until_closed(&app, "yantrik-system-monitor");
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

    // Was hardcoded to zero, so the card always read "Load: 0.00 0.00 0.00".
    let load = sysinfo::System::load_average();

    let cpu = CpuInfo {
        overall_percent: overall,
        cores,
        load_avg_1: load.one,
        load_avg_5: load.five,
        load_avg_15: load.fifteen,
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
    ui.set_memory_available_text(format_bytes(snap.memory.available_bytes).into());
    ui.set_memory_cached_text(format_bytes(snap.memory.cached_bytes).into());
    ui.set_memory_buffers_text(format_bytes(snap.memory.buffers_bytes).into());
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
    // The list was just rebuilt under the selection, and the row the selection points at
    // may not have survived it. Dropping a stale pid here is what hides End and Force Kill
    // with the row they act on (#220).
    ui.set_selected_process_pid(selection_after_refresh(
        ui.get_selected_process_pid(),
        procs,
    ));
}

/// What the selection must be after the process list is refreshed.
///
/// End and Force Kill act on `selected-process-pid`, and the list under them is rebuilt
/// every poll: the selected process can end — here, or anywhere else on the machine — or a
/// filter can stop showing its row. The pid used to stay selected through all of it, so the
/// buttons remained on screen aimed at a process that was gone (#220). The selection now
/// lives exactly as long as the shown list does.
fn selection_after_refresh(selected: i32, procs: &[ProcessInfo]) -> i32 {
    if selected >= 0 && procs.iter().any(|p| p.pid as i32 == selected) {
        selected
    } else {
        -1
    }
}

// ── Ending a process ─────────────────────────────────────────────────
//
// One path, three doors: the End button, the Force Kill button, and the `kill_process` action.
//
// Before this existed they were three different stories. The action invoked the button's
// callback and answered `{"killed": pid}` regardless of what happened next; the callback logged
// a tracing warning when the service refused and then retried locally in silence; the force
// callback skipped the service and dropped the return value of the kill outright. So a caller
// was told a process had ended whether the service, the fallback, or neither had managed it,
// and the person at the window was told nothing at all either way.

/// End a process and report what was observed to happen to it.
///
/// The result is returned to whoever asked and, either way, shown on screen — so the agent and
/// the person cannot come away with different accounts of the same kill. This is Download
/// Manager's `settle()` arrangement, applied to the one action here that cannot be undone.
fn end_process(
    ui: &SystemMonitorApp,
    status: &Rc<RefCell<Status>>,
    pid: i32,
    force: bool,
) -> Result<Observed, String> {
    let outcome = kill_and_verify(ui, pid, force);
    match &outcome {
        Ok(observed) => {
            tracing::info!("{}", observed.sentence());
            status.borrow_mut().failure = None;
        }
        Err(reason) => {
            tracing::warn!("Kill failed: {reason}");
            status.borrow_mut().failure = Some(reason.clone());
        }
    }
    show_status(ui, status);
    outcome
}

/// Signal a process, then look at the process table to find out whether it worked.
fn kill_and_verify(ui: &SystemMonitorApp, pid: i32, force: bool) -> Result<Observed, String> {
    let pid = outcome::checked_pid(pid)?;
    let name = process_name(ui, pid);

    // Nothing to signal. `kill(2)` would fail with ESRCH anyway, but the old local fallback
    // swallowed that — `sys.process(...)` returning `None` simply did nothing — so ending a pid
    // that was never there reported the same success as ending a real one.
    if outcome::liveness(pid) == Liveness::Gone {
        return Err(format!("no process {pid} is running"));
    }

    let signal = if force { Signal::Kill } else { Signal::Term };
    let via = deliver(pid, signal)?;
    let (exited, waited_ms) = outcome::wait_until_gone(
        pid,
        outcome::VERIFY_BUDGET,
        outcome::VERIFY_STEP,
        outcome::liveness,
    );
    outcome::classify(pid, &name, signal, via, exited, waited_ms)
}

/// Send the signal by the service if it will take it, and by this process if it will not.
///
/// The service runs the same `kill(2)` this does, so the local path is a genuine fallback and
/// not a lesser one — but which of the two did it is part of the answer, because a machine whose
/// system-monitor service is dead is a machine whose owner should hear about it.
fn deliver(pid: u32, signal: Signal) -> Result<Source, String> {
    let mut service_refused = None;

    // `sysmon.kill_process` takes a pid and no signal, and sends SIGTERM (see
    // services/system-monitor-service). A forced kill therefore has no service path to try, and
    // pretending otherwise would send a SIGTERM while answering SIGKILL.
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

#[cfg(unix)]
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
fn signal_locally(pid: u32, signal: Signal) -> Result<(), String> {
    Err(format!("this build cannot send {} to {pid}", signal.as_str()))
}

/// What the process list is calling this pid, so the answer says what was ended and not only
/// which number. The list is the top hundred, so a pid outside it has no name here.
fn process_name(ui: &SystemMonitorApp, pid: u32) -> String {
    let procs = ui.get_processes();
    (0..procs.row_count())
        .filter_map(|i| procs.row_data(i))
        .find(|p| p.pid == pid as i32)
        .map(|p| p.name.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

// ── Wire all callbacks ───────────────────────────────────────────────

// ── The control surface ──────────────────────────────────────────────
//
// This window already holds, in numbers, everything the companion previously had to shell out
// for: `top`, `df`, `free`, `uptime`. It refreshes every two seconds anyway, so reading it costs
// nothing and is more current than a subprocess would be.
//
// `kill_process` is published as `dangerous`. Every other action here is a view change; this one
// ends someone's work, and the caller's ceiling should have to allow it explicitly.

fn publish_control(app: &SystemMonitorApp, status: Rc<RefCell<Status>>) {
    use yantrik_app_runtime::control::{Action, App, Param, View};

    let describe = {
        let weak = app.as_weak();
        let status = status.clone();
        move || {
            let Some(ui) = weak.upgrade() else {
                return View::new("System Monitor — closing");
            };

            let cpu = ui.get_cpu_usage();
            let mem = ui.get_memory_usage();
            let health = ui.get_health_status().to_string();

            let procs = ui.get_processes();
            // The busiest handful. The window shows hundreds; a caller asking "what is eating the
            // machine" wants the top of the list, not a transcript of it.
            let top: Vec<serde_json::Value> = (0..procs.row_count().min(10))
                .filter_map(|i| procs.row_data(i))
                .map(|p| {
                    serde_json::json!({
                        "pid": p.pid,
                        "name": p.name.to_string(),
                        "cpu_percent": (p.cpu_percent * 10.0).round() / 10.0,
                        "mem_percent": (p.mem_percent * 10.0).round() / 10.0,
                        "status": p.status.to_string(),
                    })
                })
                .collect();

            let disk_model = ui.get_disks();
            let disks: Vec<serde_json::Value> = (0..disk_model.row_count())
                .filter_map(|i| disk_model.row_data(i))
                .map(|d| {
                    serde_json::json!({
                        "mount": d.mount_point.to_string(),
                        "used": d.used_bytes.to_string(),
                        "total": d.total_bytes.to_string(),
                        "percent": (d.usage_percent * 10.0).round() / 10.0,
                    })
                })
                .collect();

            let net_model = ui.get_network_interfaces();
            let interfaces: Vec<serde_json::Value> = (0..net_model.row_count())
                .filter_map(|i| net_model.row_data(i))
                .map(|n| {
                    serde_json::json!({
                        "name": n.name.to_string(),
                        // Null, not "". Neither the service's snapshot nor `sysinfo` carries an
                        // address — `NetworkInterface` in the contract has no field for one — so
                        // this is a thing nobody has measured, not an interface with no address.
                        "ip": outcome::measured(&n.ip_address),
                    })
                })
                .collect();

            let reading = status.borrow().reading.clone().unwrap_or_else(Provenance::service);

            let summary = format!(
                "System — {health}, CPU {cpu:.0}%, memory {mem:.0}% ({} of {}), up {}",
                ui.get_memory_used_text(),
                ui.get_memory_total_text(),
                ui.get_uptime_text()
            );

            View::new(summary)
                // Where every number above came from. A caller acting on these readings is
                // entitled to know that the service answered them, because when it did not, the
                // fallback's blind spots (the CPU model, the interface addresses) used to arrive
                // looking exactly like measurements.
                .with("source", reading.source.as_str())
                .with("degraded", reading.degraded())
                .with(
                    "degraded_reason",
                    reading
                        .reason
                        .as_deref()
                        .map(serde_json::Value::from)
                        .unwrap_or(serde_json::Value::Null),
                )
                // Said twice: this is the same text the person is looking at on the strip.
                .with("notice", ui.get_notice().to_string())
                .with("health", health)
                .with("health_score", ui.get_health_score() as f64)
                .with("health_summary", ui.get_health_summary().to_string())
                .with("cpu_percent", (cpu * 10.0).round() as f64 / 10.0)
                // Also null when nothing has measured it, which today is always: the snapshot
                // contract has no model string in it, so neither path can fill this in. It was
                // reported as `""`, and an audit read that as a CPU whose model is empty.
                .with("cpu_model", outcome::measured(&ui.get_cpu_model()))
                .with("load_average", serde_json::json!([
                    ui.get_load_avg_1().to_string(),
                    ui.get_load_avg_5().to_string(),
                    ui.get_load_avg_15().to_string(),
                ]))
                .with("memory_percent", (mem * 10.0).round() as f64 / 10.0)
                .with("memory_used", ui.get_memory_used_text().to_string())
                .with("memory_total", ui.get_memory_total_text().to_string())
                .with("memory_available", ui.get_memory_available_text().to_string())
                .with("swap_percent", (ui.get_swap_usage() * 10.0).round() as f64 / 10.0)
                .with("uptime", ui.get_uptime_text().to_string())
                .with("top_processes", serde_json::Value::Array(top))
                .with("disks", serde_json::Value::Array(disks))
                .with("interfaces", serde_json::Value::Array(interfaces))
        }
    };

    let weak = app.as_weak();
    let ui_for = move || weak.upgrade().ok_or_else(|| "System Monitor window is gone".to_string());

    let sort_ui = ui_for.clone();
    let search_ui = ui_for.clone();
    let kill_ui = ui_for;
    let kill_status = status;

    App::new("system-monitor")
        .describe(describe)
        .action(
            Action::new("sort_processes", "Order the process list by what it is using")
                .arg(Param::text("by").describe("cpu | memory")),
            move |args| {
                let ui = sort_ui()?;
                let column = match args["by"].as_str().unwrap_or_default().to_lowercase().as_str() {
                    "cpu" => 0,
                    "memory" | "mem" => 1,
                    other => return Err(format!("sort by cpu or memory, not `{other}`")),
                };
                ui.invoke_sort_by_column(column);
                Ok(serde_json::json!({ "sorted_by": args["by"].as_str().unwrap_or_default() }))
            },
        )
        .action(
            Action::new("filter_processes", "Show only processes whose name matches")
                .arg(Param::text("query").describe("Empty string clears the filter")),
            move |args| {
                let ui = search_ui()?;
                let query = args["query"].as_str().unwrap_or_default().to_string();
                ui.set_process_search(query.clone().into());
                ui.invoke_process_search_changed(query.into());
                let procs = ui.get_processes();
                Ok(serde_json::json!({ "showing": procs.row_count() }))
            },
        )
        .action(
            Action::new("kill_process", "End a running process by pid")
                .arg(Param::integer("pid"))
                .arg(Param::flag("force").describe("SIGKILL instead of SIGTERM").optional())
                // Everything else on this surface changes a view. This ends someone's work, and
                // there is no undo — so it must clear the caller's ceiling on its own.
                .risk("dangerous"),
            move |args| {
                let ui = kill_ui()?;
                let pid = args["pid"].as_i64().ok_or("`pid` must be a number")? as i32;
                let force = args["force"].as_bool().unwrap_or(false);
                // The same call the End and Force Kill buttons make. It does not return until
                // the process table has been read back, so `Ok` here means the pid is gone and
                // an `Err` names which of the ways it can fail happened.
                end_process(&ui, &kill_status, pid, force).map(|observed| observed.json())
            },
        )
        .serve();
}

/// One round of readings, and a note of where they came from.
///
/// The fallback used to be written `snapshot_via_service().unwrap_or_else(|_| snapshot_local())`,
/// which threw away both the reason and the fact that it had happened. It is worth keeping — a
/// monitor that goes blank because a service died is worse than one reading its own `sysinfo` —
/// but only as long as it is visible, so the value and the provenance now arrive together.
fn poll(ui: &SystemMonitorApp, status: &Rc<RefCell<Status>>, sort: &str, limit: u32) {
    let (snap, from_snapshot) = outcome::reading(snapshot_via_service(), snapshot_local);
    apply_snapshot(ui, &snap);

    let (mut procs, from_processes) =
        outcome::reading(processes_via_service(sort, limit), || processes_local(sort, limit));
    let filter = ui.get_process_search().to_string();
    if !filter.is_empty() {
        let lower = filter.to_lowercase();
        procs.retain(|p| p.name.to_lowercase().contains(&lower));
    }
    apply_processes(ui, &procs);

    status.borrow_mut().reading = Some(outcome::worse(from_snapshot, from_processes));
    show_status(ui, status);
}

fn wire(app: &SystemMonitorApp) -> Timer {
    let status = Rc::new(RefCell::new(Status::default()));

    // Initial readings, before the surface is published.
    poll(app, &status, "cpu", 50);

    // Polling timer — every 2 seconds
    // Published before the poll starts; the first `app.describe` may catch a fresh window, and
    // reporting zeroes honestly is better than delaying the surface for two seconds.
    publish_control(app, status.clone());

    let timer = Timer::default();
    let weak = app.as_weak();
    let poll_status = status.clone();
    timer.start(TimerMode::Repeated, std::time::Duration::from_secs(2), move || {
        let Some(ui) = weak.upgrade() else { return };
        let sort = if ui.get_sort_column() == 1 { "mem" } else { "cpu" };
        poll(&ui, &poll_status, sort, 100);
    });

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

    // End / Force Kill
    //
    // Both buttons go through the same `end_process` the `kill_process` action calls. The result
    // is discarded here because the notice strip is where a button reports itself, and
    // `end_process` has already written it.
    {
        let weak = app.as_weak();
        let status = status.clone();
        app.on_kill_process(move |pid| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = end_process(&ui, &status, pid, false);
        });
    }
    {
        let weak = app.as_weak();
        let status = status.clone();
        app.on_force_kill_process(move |pid| {
            let Some(ui) = weak.upgrade() else { return };
            let _ = end_process(&ui, &status, pid, true);
        });
    }

    // ── AI Explain, in the header ──
    //
    // This logged "(standalone mode)" and did nothing, while the agent rail three screens away
    // already had a working `companion::ask`. Same question, same readings, drawn into the
    // header's own panel instead of the rail's proposal card.
    {
        let weak = app.as_weak();
        app.on_ai_explain_pressed(move || {
            let Some(ui) = weak.upgrade() else { return };
            if let Some(hint) = companion::reach().hint() {
                ui.set_ai_response(hint.into());
                return;
            }
            let prompt = machine_question(&ui);
            ui.set_ai_is_working(true);
            ui.set_ai_response(SharedString::new());
            let back = ui.as_weak();
            std::thread::spawn(move || {
                let outcome = companion::ask(&prompt);
                let _ = back.upgrade_in_event_loop(move |ui| {
                    ui.set_ai_is_working(false);
                    ui.set_ai_response(match outcome {
                        Ok(text) => text.into(),
                        Err(e) => e.to_string().into(),
                    });
                });
            });
        });
    }
    {
        let weak = app.as_weak();
        // Clears the answer as well as the panel: reopening it later must not show a reading of
        // the machine as it was some minutes ago.
        app.on_ai_dismiss(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_ai_response(SharedString::new());
            }
        });
    }
    // There is nowhere to go back to from a window of its own, which is why this wrapper sets
    // `show-back: false` and the arrow is not drawn. The callback cannot fire; it logged a line
    // saying so, which would only ever have appeared if the claim were false.
    app.on_back_pressed(|| {});

    timer
}

/// The current readings, phrased as a question for the companion.
///
/// Handed over as readings. Describing them in prose first and asking the model to re-derive
/// them is how a monitor starts reporting numbers nobody measured.
fn machine_question(ui: &SystemMonitorApp) -> String {
    let facts = format!(
        "CPU {:.0}%, memory {} of {}, health {} ({})",
        ui.get_cpu_usage(),
        ui.get_memory_used_text(),
        ui.get_memory_total_text(),
        ui.get_health_status(),
        ui.get_health_summary()
    );
    format!(
        "Here are my machine's readings: {facts}. In at most three short lines say whether \
         anything needs attention and why. Use only these numbers; do not guess at causes \
         you cannot see."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(pid: u32, name: &str) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.to_string(),
            cpu_percent: 1.0,
            mem_percent: 1.0,
            mem_bytes: 1024,
            state: "S".to_string(),
            user: "yantrik".to_string(),
        }
    }

    #[test]
    fn a_selection_the_refreshed_list_still_shows_survives_it() {
        let procs = vec![running(7, "pi"), running(9, "chromium")];
        assert_eq!(selection_after_refresh(7, &procs), 7);
    }

    #[test]
    fn a_selection_whose_process_is_gone_is_dropped() {
        // The person selected a row and the process ended — here or anywhere else on the
        // machine. The next poll rebuilds the list without it, and End and Force Kill must
        // hide with the row they act on instead of staying aimed at a dead pid (#220).
        let procs = vec![running(9, "chromium")];
        assert_eq!(selection_after_refresh(7, &procs), -1);
    }

    #[test]
    fn a_selection_a_filter_hides_is_dropped() {
        // poll() filters before it applies, so a row the search stopped showing is gone
        // from the list the buttons belong to.
        let matching = vec![running(9, "chromium")];
        assert_eq!(selection_after_refresh(7, &matching), -1);
    }

    #[test]
    fn nothing_selected_stays_nothing_selected() {
        assert_eq!(selection_after_refresh(-1, &[]), -1);
        assert_eq!(selection_after_refresh(-1, &[running(7, "pi")]), -1);
    }
}
