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

        // Check installed .desktop apps first
        for entry in apps.iter() {
            if entry.app_id == app || entry.name.to_lowercase() == app {
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
            "terminal" => "foot",
            "browser" => "firefox-esr",
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
            _ => {
                tracing::warn!(app = %app, "Unknown app");
                return;
            }
        };

        match std::process::Command::new(cmd).spawn() {
            Ok(_) => tracing::info!(cmd, "App started"),
            Err(e) => tracing::error!(cmd, error = %e, "Failed to launch app"),
        }
    });
}
