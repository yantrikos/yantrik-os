//! Yantrik Container Manager — standalone app binary.
//!
//! Manages Docker/Podman containers via `std::process::Command`.

use slint::{ComponentHandle, ModelRc, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-container-manager");

    let app = ContainerManagerApp::new().unwrap();
    wire(&app);
    app.run().unwrap();
}

// ── Runtime detection ────────────────────────────────────────────────

fn runtime_cmd() -> &'static str {
    if which("podman") {
        "podman"
    } else {
        "docker"
    }
}

fn which(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── Container listing ────────────────────────────────────────────────

fn list_containers() -> Vec<ContainerData> {
    let rt = runtime_cmd();
    let output = std::process::Command::new(rt)
        .args(["ps", "-a", "--format", "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.State}}\t{{.Status}}\t{{.Ports}}\t{{.CreatedAt}}"])
        .output();

    let Ok(out) = output else { return vec![] };
    if !out.status.success() {
        return vec![];
    }

    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.splitn(7, '\t').collect();
            let get = |i: usize, def: &str| -> slint::SharedString {
                parts.get(i).copied().unwrap_or(def).into()
            };
            ContainerData {
                id: get(0, ""),
                name: get(1, ""),
                image: get(2, ""),
                status: get(3, "stopped"),
                status_text: get(4, ""),
                ports: get(5, ""),
                created: get(6, ""),
                is_selected: false,
            }
        })
        .collect()
}

fn list_images() -> Vec<ImageData> {
    let rt = runtime_cmd();
    let output = std::process::Command::new(rt)
        .args(["images", "--format", "{{.ID}}\t{{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.CreatedAt}}"])
        .output();

    let Ok(out) = output else { return vec![] };
    if !out.status.success() {
        return vec![];
    }

    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.splitn(4, '\t').collect();
            let get = |i: usize| -> slint::SharedString {
                parts.get(i).copied().unwrap_or("").into()
            };
            ImageData {
                id: get(0),
                repo_tag: get(1),
                size_text: get(2),
                created: get(3),
            }
        })
        .collect()
}

fn list_volumes() -> Vec<VolumeData> {
    let rt = runtime_cmd();
    let output = std::process::Command::new(rt)
        .args(["volume", "ls", "--format", "{{.Name}}\t{{.Driver}}\t{{.Mountpoint}}"])
        .output();

    let Ok(out) = output else { return vec![] };
    if !out.status.success() {
        return vec![];
    }

    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            let get = |i: usize| -> slint::SharedString {
                parts.get(i).copied().unwrap_or("").into()
            };
            VolumeData {
                name: get(0),
                driver: get(1),
                mount_point: get(2),
                size_text: "".into(),
            }
        })
        .collect()
}

// ── Refresh helper ───────────────────────────────────────────────────

fn refresh(app: &ContainerManagerApp) {
    let containers = list_containers();
    let running = containers.iter().filter(|c| c.status.as_str() == "running").count() as i32;
    let total = containers.len() as i32;
    let stopped = total - running;
    app.set_containers(ModelRc::new(VecModel::from(containers)));
    app.set_running_count(running);
    app.set_stopped_count(stopped);
    app.set_total_count(total);
    app.set_images(ModelRc::new(VecModel::from(list_images())));
    app.set_volumes(ModelRc::new(VecModel::from(list_volumes())));
}

// ── Wire all callbacks ───────────────────────────────────────────────

fn wire(app: &ContainerManagerApp) {
    let rt = runtime_cmd();
    app.set_runtime_name(rt.into());

    // Initial load
    refresh(app);

    // Tab switch
    app.on_ct_tab(|_tab| {
        tracing::debug!("Switched to tab {}", _tab);
    });

    // Refresh
    {
        let weak = app.as_weak();
        app.on_ct_refresh(move || {
            if let Some(ui) = weak.upgrade() {
                refresh(&ui);
            }
        });
    }

    // Start container
    {
        let weak = app.as_weak();
        app.on_ct_start(move |id| {
            let _ = std::process::Command::new(runtime_cmd())
                .args(["start", &id.to_string()])
                .output();
            if let Some(ui) = weak.upgrade() {
                refresh(&ui);
            }
        });
    }

    // Stop container
    {
        let weak = app.as_weak();
        app.on_ct_stop(move |id| {
            let _ = std::process::Command::new(runtime_cmd())
                .args(["stop", &id.to_string()])
                .output();
            if let Some(ui) = weak.upgrade() {
                refresh(&ui);
            }
        });
    }

    // Restart container
    {
        let weak = app.as_weak();
        app.on_ct_restart(move |id| {
            let _ = std::process::Command::new(runtime_cmd())
                .args(["restart", &id.to_string()])
                .output();
            if let Some(ui) = weak.upgrade() {
                refresh(&ui);
            }
        });
    }

    // Remove container
    {
        let weak = app.as_weak();
        app.on_ct_remove(move |id| {
            let _ = std::process::Command::new(runtime_cmd())
                .args(["rm", "-f", &id.to_string()])
                .output();
            if let Some(ui) = weak.upgrade() {
                refresh(&ui);
            }
        });
    }

    // View logs
    {
        let weak = app.as_weak();
        app.on_ct_logs(move |id| {
            let output = std::process::Command::new(runtime_cmd())
                .args(["logs", "--tail", "200", &id.to_string()])
                .output();
            if let Some(ui) = weak.upgrade() {
                let log_text = output
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_else(|e| format!("Failed to get logs: {e}"));
                ui.set_log_text(log_text.into());
                ui.set_log_container_name(id);
                ui.set_show_logs(true);
            }
        });
    }

    // Run new container
    {
        let weak = app.as_weak();
        app.on_ct_run(move |image| {
            let _ = std::process::Command::new(runtime_cmd())
                .args(["run", "-d", &image.to_string()])
                .output();
            if let Some(ui) = weak.upgrade() {
                refresh(&ui);
            }
        });
    }

    // Pull image
    {
        let weak = app.as_weak();
        app.on_ct_pull(move |image| {
            let _ = std::process::Command::new(runtime_cmd())
                .args(["pull", &image.to_string()])
                .output();
            if let Some(ui) = weak.upgrade() {
                refresh(&ui);
            }
        });
    }

    // Remove image
    {
        let weak = app.as_weak();
        app.on_ct_remove_image(move |id| {
            let _ = std::process::Command::new(runtime_cmd())
                .args(["rmi", &id.to_string()])
                .output();
            if let Some(ui) = weak.upgrade() {
                refresh(&ui);
            }
        });
    }

    // Create volume
    {
        let weak = app.as_weak();
        app.on_ct_create_volume(move |name| {
            let _ = std::process::Command::new(runtime_cmd())
                .args(["volume", "create", &name.to_string()])
                .output();
            if let Some(ui) = weak.upgrade() {
                refresh(&ui);
            }
        });
    }

    // Remove volume
    {
        let weak = app.as_weak();
        app.on_ct_remove_volume(move |name| {
            let _ = std::process::Command::new(runtime_cmd())
                .args(["volume", "rm", &name.to_string()])
                .output();
            if let Some(ui) = weak.upgrade() {
                refresh(&ui);
            }
        });
    }

    // AI stubs
    app.on_ai_summarize_logs(|| {
        tracing::info!("AI log summarization requested (standalone mode)");
    });
    app.on_ai_dismiss(|| {});
}
