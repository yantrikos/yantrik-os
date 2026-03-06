//! App grid — populate grid apps from installed apps, handle launch.

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app_context::AppContext;
use crate::{App, AppGridItem};

pub fn wire(ui: &App, ctx: &AppContext) {
    // Populate grid-apps model from installed apps
    let apps: Vec<AppGridItem> = ctx
        .installed_apps
        .iter()
        .map(|entry| AppGridItem {
            app_id: entry.app_id.clone().into(),
            name: entry.name.clone().into(),
            icon_char: entry.icon_char.clone().into(),
        })
        .collect();
    ui.set_grid_apps(ModelRc::new(VecModel::from(apps)));

    // Handle grid-launch-app — routes built-in apps to screens, external apps to processes
    let installed = ctx.installed_apps.clone();
    let ui_weak = ui.as_weak();
    ui.on_grid_launch_app(move |app_id| {
        let app_id_str = app_id.as_str();

        // Close grid first
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_app_grid_open(false);
        }

        // Built-in Yantrik apps → navigate to screen via launch-app callback
        // (dock.rs already handles the app_id → screen mapping)
        if let Some(entry) = installed.iter().find(|e| e.app_id == app_id_str) {
            if entry.exec == "__builtin__" {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.invoke_launch_app(app_id);
                }
                return;
            }
            tracing::info!(app = %entry.name, exec = %entry.exec, "Launching app from grid");
            let exec_clean = entry
                .exec
                .split_whitespace()
                .filter(|w| !w.starts_with('%'))
                .collect::<Vec<_>>();
            if let Some(cmd) = exec_clean.first() {
                let args = &exec_clean[1..];
                let _ = std::process::Command::new(cmd).args(args).spawn();
            }
        }
    });
}
