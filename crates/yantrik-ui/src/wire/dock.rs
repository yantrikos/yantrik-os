//! Dock wiring — on_launch_app callback.

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::App;

/// Wire on_launch_app callback.
pub fn wire(ui: &App, ctx: &AppContext) {
    let apps = ctx.installed_apps.clone();
    let ui_weak = ui.as_weak();

    ui.on_launch_app(move |app_id| {
        let app = app_id.to_string();
        tracing::info!(app = %app, "Launching app");

        // Check installed .desktop apps first (skip built-in Yantrik apps)
        for entry in apps.iter() {
            if entry.app_id == app || entry.name.to_lowercase() == app {
                if entry.exec == "__builtin__" {
                    break; // Fall through to built-in screen routing below
                }
                let parts: Vec<&str> = entry.exec.split_whitespace().collect();
                if let Some((bin, args)) = parts.split_first() {
                    match std::process::Command::new(bin).args(args).spawn() {
                        Ok(_) => tracing::info!(name = %entry.name, "App started"),
                        Err(e) => {
                            tracing::error!(name = %entry.name, error = %e, "Failed to launch")
                        }
                    }
                }
                return;
            }
        }

        // Fallback: hardcoded commands
        let cmd = match app.as_str() {
            "terminal" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(14);
                    ui.invoke_navigate(14);
                }
                return;
            }
            "browser" => {
                // Launch visible Chromium with Wayland + separate user-data-dir
                // (headless instance may be holding the default profile lock)
                match std::process::Command::new("chromium")
                    .args([
                        "--ozone-platform=wayland",
                        "--no-first-run",
                        "--no-default-browser-check",
                        "--disable-gpu",
                        "--user-data-dir=/tmp/chromium-visible",
                    ])
                    .env("WAYLAND_DISPLAY", "wayland-0")
                    .env("XDG_RUNTIME_DIR", "/run/user/1000")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    Ok(_) => tracing::info!("Browser launched (visible mode)"),
                    Err(e) => tracing::error!(error = %e, "Failed to launch browser"),
                }
                return;
            }
            "files" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(8);
                    ui.invoke_navigate(8);
                }
                return;
            }
            "settings" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(7);
                    ui.invoke_navigate(7);
                }
                return;
            }
            "notes" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(15);
                    ui.invoke_navigate(15);
                }
                return;
            }
            "editor" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_editor_file_name("untitled".into());
                    ui.set_editor_file_content("".into());
                    ui.set_editor_is_modified(false);
                    ui.set_editor_is_readonly(false);
                    ui.set_current_screen(12);
                    ui.invoke_navigate(12);
                }
                return;
            }
            "bond" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(4);
                    ui.invoke_navigate(4);
                }
                return;
            }
            "personality" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(5);
                    ui.invoke_navigate(5);
                }
                return;
            }
            "memory" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(6);
                    ui.invoke_navigate(6);
                }
                return;
            }
            "notifications" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(9);
                    ui.invoke_navigate(9);
                }
                return;
            }
            "system" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(10);
                    ui.invoke_navigate(10);
                }
                return;
            }
            "media" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(13);
                    ui.invoke_navigate(13);
                }
                return;
            }
            "email" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(17);
                    ui.invoke_navigate(17);
                }
                return;
            }
            "calendar" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(18);
                    ui.invoke_navigate(18);
                }
                return;
            }
            "packages" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(21);
                    ui.invoke_navigate(21);
                }
                return;
            }
            "network" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(22);
                    ui.invoke_navigate(22);
                }
                return;
            }
            "sysmonitor" | "system_monitor" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(23);
                    ui.invoke_navigate(23);
                }
                return;
            }
            "weather" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(19);
                    ui.invoke_navigate(19);
                }
                return;
            }
            "music" | "music_player" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(20);
                    ui.invoke_navigate(20);
                }
                return;
            }
            "downloads" | "download_manager" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(24);
                    ui.invoke_navigate(24);
                }
                return;
            }
            "snippets" | "snippet_manager" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(25);
                    ui.invoke_navigate(25);
                }
                return;
            }
            "containers" | "container_manager" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(26);
                    ui.invoke_navigate(26);
                }
                return;
            }
            "devices" | "device_dashboard" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(27);
                    ui.invoke_navigate(27);
                }
                return;
            }
            "permissions" | "permission_dashboard" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(28);
                    ui.invoke_navigate(28);
                }
                return;
            }
            "launchpad" => {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_current_screen(1);
                    ui.invoke_navigate(1);
                    ui.set_app_grid_open(true);
                }
                return;
            }
            _ => {
                tracing::warn!(app = %app, "Unknown app");
                return;
            }
        };
    });
}
