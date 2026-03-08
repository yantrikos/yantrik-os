//! App grid — populate grid apps from installed apps, handle launch + search.

use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app_context::AppContext;
use crate::apps::DesktopEntry;
use crate::{App, AppGridItem};

pub fn wire(ui: &App, ctx: &AppContext) {
    let installed = ctx.installed_apps.clone();

    // Populate grid-apps model from installed apps
    populate_grid(ui, &installed, "");

    // Handle search — filter apps by query
    let installed_search = installed.clone();
    let ui_weak = ui.as_weak();
    ui.on_grid_search_apps(move |query| {
        if let Some(ui) = ui_weak.upgrade() {
            populate_grid(&ui, &installed_search, query.as_str());
        }
    });

    // Handle grid-launch-app — routes built-in apps to screens, external apps to processes
    let ui_weak = ui.as_weak();
    ui.on_grid_launch_app(move |app_id| {
        let app_id_str = app_id.as_str();

        // Close grid first
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_app_grid_open(false);
        }

        // Built-in Yantrik apps → navigate to screen via launch-app callback
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

fn populate_grid(ui: &App, installed: &Arc<Vec<DesktopEntry>>, query: &str) {
    let query_lower = query.to_lowercase();
    let apps: Vec<AppGridItem> = installed
        .iter()
        .filter(|entry| {
            if query_lower.is_empty() {
                return true;
            }
            entry.name.to_lowercase().contains(&query_lower)
                || entry.app_id.to_lowercase().contains(&query_lower)
                || entry.categories.to_lowercase().contains(&query_lower)
                || entry.comment.to_lowercase().contains(&query_lower)
        })
        .map(|entry| AppGridItem {
            app_id: entry.app_id.clone().into(),
            name: entry.name.clone().into(),
            icon_char: entry.icon_char.clone().into(),
        })
        .collect();
    ui.set_grid_apps(ModelRc::new(VecModel::from(apps)));
}
