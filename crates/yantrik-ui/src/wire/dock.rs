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
