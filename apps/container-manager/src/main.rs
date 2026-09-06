//! Yantrik Container Manager — standalone app binary.
//!
//! Manages Docker/Podman containers via `std::process::Command`.

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-container-manager");

    // One window per app: a second launch defers to the running one (the shell focuses it).
    let Some(_instance) = instance::claim("container-manager") else { return };

    let app = ContainerManagerApp::new().unwrap();

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

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

// ── The control surface ──────────────────────────────────────────────
//
// The companion has docker tools of its own, and they shell out exactly as this app does. What
// this adds is the *user's* view: which containers they are looking at, which one is selected,
// what the log pane is showing. See `yantrik_app_runtime::control`.
//
// Stopping and removing are declared `dangerous`. Starting a container is recoverable; removing
// one, or the volume under it, is not.

/// Container whose name or id matches `needle`, as shown in the list.
fn container_named(ui: &ContainerManagerApp, needle: &str) -> Option<(String, String)> {
    let want = needle.trim().to_lowercase();
    if want.is_empty() {
        return None;
    }
    let model = ui.get_containers();
    let rows: Vec<ContainerData> = (0..model.row_count()).filter_map(|i| model.row_data(i)).collect();
    rows.iter()
        .find(|c| c.name.to_lowercase() == want || c.id.to_lowercase().starts_with(&want))
        .or_else(|| rows.iter().find(|c| c.name.to_lowercase().contains(&want)))
        .map(|c| (c.id.to_string(), c.name.to_string()))
}

fn publish_control(app: &ContainerManagerApp) {
    use yantrik_app_runtime::control::{Action, App, Param, View};

    let describe = {
        let weak = app.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return View::new("Containers — closing");
            };

            let model = ui.get_containers();
            let containers: Vec<serde_json::Value> = (0..model.row_count())
                .filter_map(|i| model.row_data(i))
                .map(|c| {
                    serde_json::json!({
                        "name": c.name.to_string(),
                        "id": c.id.to_string(),
                        "image": c.image.to_string(),
                        "status": c.status.to_string(),
                        "detail": c.status_text.to_string(),
                        "ports": c.ports.to_string(),
                    })
                })
                .collect();

            let image_model = ui.get_images();
            let images: Vec<serde_json::Value> = (0..image_model.row_count().min(30))
                .filter_map(|i| image_model.row_data(i))
                .map(|i| {
                    serde_json::json!({
                        "tag": i.repo_tag.to_string(),
                        "size": i.size_text.to_string(),
                    })
                })
                .collect();

            let volume_model = ui.get_volumes();
            let volumes: Vec<serde_json::Value> = (0..volume_model.row_count().min(30))
                .filter_map(|i| volume_model.row_data(i))
                .map(|v| {
                    serde_json::json!({
                        "name": v.name.to_string(),
                        "driver": v.driver.to_string(),
                    })
                })
                .collect();

            let runtime = ui.get_runtime_name().to_string();
            let summary = if containers.is_empty() {
                format!("Containers — no {runtime} containers")
            } else {
                format!(
                    "Containers — {} running of {} on {runtime}, {} images",
                    ui.get_running_count(),
                    ui.get_total_count(),
                    images.len()
                )
            };

            let mut view = View::new(summary)
                .with("runtime", runtime)
                .with("tab", match ui.get_active_tab() {
                    1 => "images",
                    2 => "volumes",
                    _ => "containers",
                })
                .with("running", ui.get_running_count())
                .with("stopped", ui.get_stopped_count())
                .with("total", ui.get_total_count())
                .with("containers", serde_json::Value::Array(containers))
                .with("images", serde_json::Value::Array(images))
                .with("volumes", serde_json::Value::Array(volumes));

            // Only when the pane is actually open. A log tail is the largest thing this surface
            // can carry, and sending one nobody asked for would make every describe expensive.
            if ui.get_show_logs() {
                let log = ui.get_log_text().to_string();
                let tail: String = log.chars().rev().take(4000).collect::<Vec<_>>().into_iter().rev().collect();
                view = view.with(
                    "open_logs",
                    serde_json::json!({
                        "container": ui.get_log_container_name().to_string(),
                        "tail": tail,
                    }),
                );
            }
            view
        }
    };

    let weak = app.as_weak();
    let ui_for = move || weak.upgrade().ok_or_else(|| "Containers window is gone".to_string());

    let refresh_ui = ui_for.clone();
    let start_ui = ui_for.clone();
    let stop_ui = ui_for.clone();
    let restart_ui = ui_for.clone();
    let logs_ui = ui_for.clone();
    let remove_ui = ui_for;

    App::new("containers")
        .describe(describe)
        .action(Action::new("refresh", "Re-read containers, images and volumes"), move |_| {
            let ui = refresh_ui()?;
            ui.invoke_ct_refresh();
            Ok(serde_json::json!({
                "running": ui.get_running_count(),
                "total": ui.get_total_count(),
            }))
        })
        .action(
            Action::new("start", "Start a stopped container").arg(Param::text("container")),
            move |args| {
                let ui = start_ui()?;
                let want = args["container"].as_str().unwrap_or_default();
                let (id, name) = container_named(&ui, want)
                    .ok_or_else(|| format!("no container here is called \"{want}\""))?;
                ui.invoke_ct_start(id.into());
                Ok(serde_json::json!({ "started": name }))
            },
        )
        .action(
            // Recoverable, but it interrupts whatever the container was serving.
            Action::new("stop", "Stop a running container")
                .arg(Param::text("container"))
                .risk("sensitive"),
            move |args| {
                let ui = stop_ui()?;
                let want = args["container"].as_str().unwrap_or_default();
                let (id, name) = container_named(&ui, want)
                    .ok_or_else(|| format!("no container here is called \"{want}\""))?;
                ui.invoke_ct_stop(id.into());
                Ok(serde_json::json!({ "stopped": name }))
            },
        )
        .action(
            Action::new("restart", "Restart a container")
                .arg(Param::text("container"))
                .risk("sensitive"),
            move |args| {
                let ui = restart_ui()?;
                let want = args["container"].as_str().unwrap_or_default();
                let (id, name) = container_named(&ui, want)
                    .ok_or_else(|| format!("no container here is called \"{want}\""))?;
                ui.invoke_ct_restart(id.into());
                Ok(serde_json::json!({ "restarted": name }))
            },
        )
        .action(
            Action::new("show_logs", "Open the log pane for a container, and read the tail")
                .arg(Param::text("container")),
            move |args| {
                let ui = logs_ui()?;
                let want = args["container"].as_str().unwrap_or_default();
                let (id, name) = container_named(&ui, want)
                    .ok_or_else(|| format!("no container here is called \"{want}\""))?;
                ui.invoke_ct_logs(id.into());
                let log = ui.get_log_text().to_string();
                let tail: String = log.lines().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
                Ok(serde_json::json!({ "container": name, "tail": tail }))
            },
        )
        .action(
            // No undo, and the container's writable layer goes with it.
            Action::new("remove", "Delete a container")
                .arg(Param::text("container"))
                .risk("dangerous"),
            move |args| {
                let ui = remove_ui()?;
                let want = args["container"].as_str().unwrap_or_default();
                let (id, name) = container_named(&ui, want)
                    .ok_or_else(|| format!("no container here is called \"{want}\""))?;
                ui.invoke_ct_remove(id.into());
                Ok(serde_json::json!({ "removed": name }))
            },
        )
        .serve();
}

fn wire(app: &ContainerManagerApp) {
    let rt = runtime_cmd();
    app.set_runtime_name(rt.into());

    // Initial load
    refresh(app);

    // Published after the first read, so a describe reports real containers.
    publish_control(app);

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
