//! Container Manager wire module — Docker/Podman container management.
//!
//! Auto-detects container runtime (podman first, then docker).
//! Runs all commands in background threads to avoid blocking UI.
//! Refreshes container state on a 5-second timer.

use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::app_context::AppContext;
use crate::{App, ContainerData, ImageData, VolumeData};

// ═══════════════════════════════════════════════════════════════════════
// State
// ═══════════════════════════════════════════════════════════════════════

struct ContainerState {
    runtime: String, // "podman" or "docker"
    containers: Vec<ContainerInfo>,
    images: Vec<ImageInfo>,
    volumes: Vec<VolumeInfo>,
    log_text: String,
    log_container_name: String,
    dirty: bool,
}

#[derive(Clone)]
struct ContainerInfo {
    id: String,
    name: String,
    image: String,
    status: String,      // "running", "stopped", "paused", "restarting"
    status_text: String,  // raw status line
    ports: String,
    created: String,
}

#[derive(Clone)]
struct ImageInfo {
    id: String,
    repo_tag: String,
    size: String,
    created: String,
}

#[derive(Clone)]
struct VolumeInfo {
    name: String,
    driver: String,
    mount_point: String,
    size: String,
}

impl ContainerState {
    fn new() -> Self {
        Self {
            runtime: detect_runtime(),
            containers: Vec::new(),
            images: Vec::new(),
            volumes: Vec::new(),
            log_text: String::new(),
            log_container_name: String::new(),
            dirty: true,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Runtime detection and command helpers
// ═══════════════════════════════════════════════════════════════════════

/// Detect container runtime: prefer podman, fallback to docker.
fn detect_runtime() -> String {
    if cmd_exists("podman") {
        "podman".to_string()
    } else if cmd_exists("docker") {
        "docker".to_string()
    } else {
        // Default to docker even if not found — commands will just fail gracefully
        "docker".to_string()
    }
}

/// Check if a command exists using `which`.
fn cmd_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run a command and return stdout, or empty string on failure.
fn cmd_output(cmd: &str, args: &[&str]) -> String {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

// ═══════════════════════════════════════════════════════════════════════
// Refresh functions
// ═══════════════════════════════════════════════════════════════════════

fn refresh_all(state: &Arc<Mutex<ContainerState>>) {
    let runtime = state.lock().ok().map(|s| s.runtime.clone()).unwrap_or_else(|| "docker".to_string());
    refresh_containers(state, &runtime);
    refresh_images(state, &runtime);
    refresh_volumes(state, &runtime);
}

fn refresh_containers(state: &Arc<Mutex<ContainerState>>, runtime: &str) {
    // Use --format with Go template for reliable parsing
    let output = cmd_output(
        runtime,
        &["ps", "-a", "--format", "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.State}}\t{{.Status}}\t{{.Ports}}\t{{.CreatedAt}}"],
    );

    let mut containers = Vec::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 5 {
            continue;
        }

        let raw_state = parts[3].to_lowercase();
        let status = if raw_state.contains("running") || raw_state == "up" {
            "running".to_string()
        } else if raw_state.contains("paused") {
            "paused".to_string()
        } else if raw_state.contains("restarting") {
            "restarting".to_string()
        } else {
            "stopped".to_string()
        };

        containers.push(ContainerInfo {
            id: parts[0].to_string(),
            name: parts.get(1).unwrap_or(&"").to_string(),
            image: parts.get(2).unwrap_or(&"").to_string(),
            status,
            status_text: parts.get(4).unwrap_or(&"").to_string(),
            ports: parts.get(5).unwrap_or(&"").to_string(),
            created: parts.get(6).unwrap_or(&"").to_string(),
        });
    }

    // Sort: running first, then by name
    containers.sort_by(|a, b| {
        let a_running = a.status == "running";
        let b_running = b.status == "running";
        b_running.cmp(&a_running).then(a.name.cmp(&b.name))
    });

    if let Ok(mut s) = state.lock() {
        s.containers = containers;
        s.dirty = true;
    }
}

fn refresh_images(state: &Arc<Mutex<ContainerState>>, runtime: &str) {
    let output = cmd_output(
        runtime,
        &["images", "--format", "{{.ID}}\t{{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.CreatedAt}}"],
    );

    let mut images = Vec::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            continue;
        }

        let id_full = parts[0].to_string();
        let id_short = if id_full.len() > 12 {
            id_full[..12].to_string()
        } else {
            id_full.clone()
        };

        images.push(ImageInfo {
            id: id_short,
            repo_tag: parts.get(1).unwrap_or(&"").to_string(),
            size: parts.get(2).unwrap_or(&"").to_string(),
            created: parts.get(3).unwrap_or(&"").to_string(),
        });
    }

    if let Ok(mut s) = state.lock() {
        s.images = images;
        s.dirty = true;
    }
}

fn refresh_volumes(state: &Arc<Mutex<ContainerState>>, runtime: &str) {
    let output = cmd_output(
        runtime,
        &["volume", "ls", "--format", "{{.Name}}\t{{.Driver}}\t{{.Mountpoint}}"],
    );

    let mut volumes = Vec::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.is_empty() || parts[0].is_empty() {
            continue;
        }

        volumes.push(VolumeInfo {
            name: parts[0].to_string(),
            driver: parts.get(1).unwrap_or(&"local").to_string(),
            mount_point: parts.get(2).unwrap_or(&"").to_string(),
            size: String::new(), // Volume size requires inspect, skip for list view
        });
    }

    if let Ok(mut s) = state.lock() {
        s.volumes = volumes;
        s.dirty = true;
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Sync state to UI
// ═══════════════════════════════════════════════════════════════════════

fn sync_to_ui(ui: &App, state: &Arc<Mutex<ContainerState>>) {
    let snap = match state.lock() {
        Ok(mut s) => {
            if !s.dirty {
                return;
            }
            s.dirty = false;
            StateSnapshot {
                runtime: s.runtime.clone(),
                containers: s.containers.clone(),
                images: s.images.clone(),
                volumes: s.volumes.clone(),
                log_text: s.log_text.clone(),
                log_container_name: s.log_container_name.clone(),
            }
        }
        Err(_) => return,
    };

    // Runtime name
    ui.set_ct_runtime_name(snap.runtime.into());

    // Container counts
    let running = snap.containers.iter().filter(|c| c.status == "running").count() as i32;
    let stopped = snap.containers.iter().filter(|c| c.status != "running").count() as i32;
    let total = snap.containers.len() as i32;
    ui.set_ct_running_count(running);
    ui.set_ct_stopped_count(stopped);
    ui.set_ct_total_count(total);

    // Containers
    let ct_items: Vec<ContainerData> = snap
        .containers
        .iter()
        .map(|c| ContainerData {
            id: c.id.clone().into(),
            name: c.name.clone().into(),
            image: c.image.clone().into(),
            status: c.status.clone().into(),
            status_text: c.status_text.clone().into(),
            ports: c.ports.clone().into(),
            created: c.created.clone().into(),
            is_selected: false,
        })
        .collect();
    ui.set_ct_containers(ModelRc::new(VecModel::from(ct_items)));

    // Images
    let img_items: Vec<ImageData> = snap
        .images
        .iter()
        .map(|i| ImageData {
            id: i.id.clone().into(),
            repo_tag: i.repo_tag.clone().into(),
            size_text: i.size.clone().into(),
            created: i.created.clone().into(),
        })
        .collect();
    ui.set_ct_images(ModelRc::new(VecModel::from(img_items)));

    // Volumes
    let vol_items: Vec<VolumeData> = snap
        .volumes
        .iter()
        .map(|v| VolumeData {
            name: v.name.clone().into(),
            driver: v.driver.clone().into(),
            mount_point: v.mount_point.clone().into(),
            size_text: v.size.clone().into(),
        })
        .collect();
    ui.set_ct_volumes(ModelRc::new(VecModel::from(vol_items)));

    // Logs
    ui.set_ct_log_text(snap.log_text.into());
    ui.set_ct_log_container_name(snap.log_container_name.into());
}

struct StateSnapshot {
    runtime: String,
    containers: Vec<ContainerInfo>,
    images: Vec<ImageInfo>,
    volumes: Vec<VolumeInfo>,
    log_text: String,
    log_container_name: String,
}

// ═══════════════════════════════════════════════════════════════════════
// Wire function
// ═══════════════════════════════════════════════════════════════════════

pub fn wire(ui: &App, ctx: &AppContext) {
    let state = Arc::new(Mutex::new(ContainerState::new()));

    // Initial refresh in background
    {
        let state_clone = state.clone();
        std::thread::spawn(move || {
            refresh_all(&state_clone);
        });
    }

    // 5-second refresh timer
    let refresh_timer = Timer::default();
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        refresh_timer.start(TimerMode::Repeated, std::time::Duration::from_secs(5), move || {
            if let Some(ui) = ui_weak.upgrade() {
                // Only update when Container Manager (screen 26) is active
                if ui.get_current_screen() != 26 {
                    return;
                }
                sync_to_ui(&ui, &state_clone);
            }

            // Trigger background refresh
            let state_bg = state_clone.clone();
            std::thread::spawn(move || {
                refresh_all(&state_bg);
            });
        });
    }
    std::mem::forget(refresh_timer);

    // Immediate sync after short delay for initial data
    {
        let state_clone = state.clone();
        let ui_weak = ui.as_weak();
        let init_timer = Timer::default();
        init_timer.start(TimerMode::Repeated, std::time::Duration::from_millis(500), move || {
            if let Some(ui) = ui_weak.upgrade() {
                sync_to_ui(&ui, &state_clone);
            }
        });
        std::mem::forget(init_timer);
    }

    // ── Tab switch callback ──
    {
        let ui_weak = ui.as_weak();
        ui.on_ct_tab(move |_idx| {
            // Tab switching is handled by Slint property binding; this callback
            // is for the backend to know about tab changes if needed.
            let _ = ui_weak.upgrade();
        });
    }

    // ── Container: start ──
    {
        let state_clone = state.clone();
        ui.on_ct_start(move |id| {
            let st = state_clone.clone();
            let id = id.to_string();
            std::thread::spawn(move || {
                let runtime = st.lock().ok().map(|s| s.runtime.clone()).unwrap_or_else(|| "docker".to_string());
                let _ = std::process::Command::new(&runtime)
                    .args(["start", &id])
                    .output();
                std::thread::sleep(std::time::Duration::from_millis(500));
                refresh_containers(&st, &runtime);
            });
        });
    }

    // ── Container: stop ──
    {
        let state_clone = state.clone();
        ui.on_ct_stop(move |id| {
            let st = state_clone.clone();
            let id = id.to_string();
            std::thread::spawn(move || {
                let runtime = st.lock().ok().map(|s| s.runtime.clone()).unwrap_or_else(|| "docker".to_string());
                let _ = std::process::Command::new(&runtime)
                    .args(["stop", &id])
                    .output();
                std::thread::sleep(std::time::Duration::from_millis(500));
                refresh_containers(&st, &runtime);
            });
        });
    }

    // ── Container: restart ──
    {
        let state_clone = state.clone();
        ui.on_ct_restart(move |id| {
            let st = state_clone.clone();
            let id = id.to_string();
            std::thread::spawn(move || {
                let runtime = st.lock().ok().map(|s| s.runtime.clone()).unwrap_or_else(|| "docker".to_string());
                let _ = std::process::Command::new(&runtime)
                    .args(["restart", &id])
                    .output();
                std::thread::sleep(std::time::Duration::from_millis(500));
                refresh_containers(&st, &runtime);
            });
        });
    }

    // ── Container: remove ──
    {
        let state_clone = state.clone();
        ui.on_ct_remove(move |id| {
            let st = state_clone.clone();
            let id = id.to_string();
            std::thread::spawn(move || {
                let runtime = st.lock().ok().map(|s| s.runtime.clone()).unwrap_or_else(|| "docker".to_string());
                // Force remove (including running containers)
                let _ = std::process::Command::new(&runtime)
                    .args(["rm", "-f", &id])
                    .output();
                std::thread::sleep(std::time::Duration::from_millis(500));
                refresh_containers(&st, &runtime);
            });
        });
    }

    // ── Container: logs ──
    {
        let state_clone = state.clone();
        ui.on_ct_logs(move |id| {
            let st = state_clone.clone();
            let id_str = id.to_string();
            std::thread::spawn(move || {
                let runtime = st.lock().ok().map(|s| s.runtime.clone()).unwrap_or_else(|| "docker".to_string());

                // Get container name for display
                let name = st
                    .lock()
                    .ok()
                    .and_then(|s| {
                        s.containers
                            .iter()
                            .find(|c| c.id == id_str)
                            .map(|c| c.name.clone())
                    })
                    .unwrap_or_else(|| id_str.clone());

                let output = cmd_output(&runtime, &["logs", "--tail", "100", &id_str]);

                if let Ok(mut s) = st.lock() {
                    s.log_text = if output.is_empty() {
                        "(no logs)".to_string()
                    } else {
                        output
                    };
                    s.log_container_name = name;
                    s.dirty = true;
                }
            });
        });
    }

    // ── Run new container ──
    {
        let state_clone = state.clone();
        ui.on_ct_run(move |image| {
            let st = state_clone.clone();
            let image = image.to_string();
            std::thread::spawn(move || {
                let runtime = st.lock().ok().map(|s| s.runtime.clone()).unwrap_or_else(|| "docker".to_string());
                let _ = std::process::Command::new(&runtime)
                    .args(["run", "-d", &image])
                    .output();
                std::thread::sleep(std::time::Duration::from_secs(1));
                refresh_containers(&st, &runtime);
            });
        });
    }

    // ── Pull image ──
    {
        let state_clone = state.clone();
        ui.on_ct_pull(move |image| {
            let st = state_clone.clone();
            let image = image.to_string();
            std::thread::spawn(move || {
                let runtime = st.lock().ok().map(|s| s.runtime.clone()).unwrap_or_else(|| "docker".to_string());
                let _ = std::process::Command::new(&runtime)
                    .args(["pull", &image])
                    .output();
                std::thread::sleep(std::time::Duration::from_millis(500));
                refresh_images(&st, &runtime);
            });
        });
    }

    // ── Remove image ──
    {
        let state_clone = state.clone();
        ui.on_ct_remove_image(move |id| {
            let st = state_clone.clone();
            let id = id.to_string();
            std::thread::spawn(move || {
                let runtime = st.lock().ok().map(|s| s.runtime.clone()).unwrap_or_else(|| "docker".to_string());
                let _ = std::process::Command::new(&runtime)
                    .args(["rmi", &id])
                    .output();
                std::thread::sleep(std::time::Duration::from_millis(500));
                refresh_images(&st, &runtime);
            });
        });
    }

    // ── Create volume ──
    {
        let state_clone = state.clone();
        ui.on_ct_create_volume(move |name| {
            let st = state_clone.clone();
            let name = name.to_string();
            std::thread::spawn(move || {
                let runtime = st.lock().ok().map(|s| s.runtime.clone()).unwrap_or_else(|| "docker".to_string());
                let _ = std::process::Command::new(&runtime)
                    .args(["volume", "create", &name])
                    .output();
                std::thread::sleep(std::time::Duration::from_millis(500));
                refresh_volumes(&st, &runtime);
            });
        });
    }

    // ── Remove volume ──
    {
        let state_clone = state.clone();
        ui.on_ct_remove_volume(move |name| {
            let st = state_clone.clone();
            let name = name.to_string();
            std::thread::spawn(move || {
                let runtime = st.lock().ok().map(|s| s.runtime.clone()).unwrap_or_else(|| "docker".to_string());
                let _ = std::process::Command::new(&runtime)
                    .args(["volume", "rm", &name])
                    .output();
                std::thread::sleep(std::time::Duration::from_millis(500));
                refresh_volumes(&st, &runtime);
            });
        });
    }

    // ── Manual refresh ──
    {
        let state_clone = state.clone();
        ui.on_ct_refresh(move || {
            let st = state_clone.clone();
            std::thread::spawn(move || {
                refresh_all(&st);
            });
        });
    }

    // ── AI Summarize Logs callback ──
    let bridge = ctx.bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();
    let ui_weak = ui.as_weak();
    let ai_st = ai_state.clone();
    ui.on_ct_ai_summarize_logs(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let logs = ui.get_ct_log_text().to_string();
        let container = ui.get_ct_log_container_name().to_string();
        if logs.is_empty() { return; }

        // Take last 2000 chars of logs to avoid prompt overflow
        let log_tail = if logs.len() > 2000 {
            &logs[logs.len() - 2000..]
        } else {
            &logs
        };

        let prompt = super::ai_assist::container_log_prompt(&container, log_tail);

        super::ai_assist::ai_request(
            &ui.as_weak(),
            &bridge,
            &ai_st,
            super::ai_assist::AiAssistRequest {
                prompt,
                timeout_secs: 30,
                set_working: Box::new(|ui, v| ui.set_ct_ai_is_working(v)),
                set_response: Box::new(|ui, s| ui.set_ct_ai_response(s.into())),
                get_response: Box::new(|ui| ui.get_ct_ai_response().to_string()),
            },
        );
    });

    // ── AI Dismiss ──
    let ui_weak = ui.as_weak();
    ui.on_ct_ai_dismiss(move || {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_ct_ai_panel_open(false);
        }
    });
}
