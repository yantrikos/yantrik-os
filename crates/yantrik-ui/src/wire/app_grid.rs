//! App grid — populate grid apps from installed apps, handle launch, search and categories.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app_context::AppContext;
use crate::apps::DesktopEntry;
use crate::icons;
use crate::{App, AppGridItem, CategoryItem};

pub fn wire(ui: &App, ctx: &AppContext) {
    let installed = ctx.installed_apps.clone();

    // The two filters compose: whichever one changes, the other is re-applied from here.
    let query = Rc::new(RefCell::new(String::new()));
    let category = Rc::new(RefCell::new(String::from("all")));

    ui.set_grid_categories(ModelRc::new(VecModel::from(categories_for(&installed))));
    populate_grid(ui, &installed, "", "all");

    {
        let installed = installed.clone();
        let query = query.clone();
        let category = category.clone();
        let ui_weak = ui.as_weak();
        ui.on_grid_search_apps(move |q| {
            *query.borrow_mut() = q.to_string();
            if let Some(ui) = ui_weak.upgrade() {
                populate_grid(&ui, &installed, &query.borrow(), &category.borrow());
            }
        });
    }
    {
        let installed = installed.clone();
        let query = query.clone();
        let category = category.clone();
        let ui_weak = ui.as_weak();
        ui.on_grid_category_selected(move |id| {
            *category.borrow_mut() = id.to_string();
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_grid_active_category(id.clone());
                populate_grid(&ui, &installed, &query.borrow(), &category.borrow());
            }
        });
    }

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

/// "All" plus every category that has at least one app, in CATEGORY_TABLE order.
fn categories_for(installed: &Arc<Vec<DesktopEntry>>) -> Vec<CategoryItem> {
    let mut out = vec![CategoryItem {
        id: "all".into(),
        name: "All".into(),
        count: installed.len() as i32,
    }];
    for (_, id) in icons::CATEGORY_TABLE {
        let count = installed
            .iter()
            .filter(|e| icons::category_id(&e.categories) == *id)
            .count();
        if count > 0 {
            out.push(CategoryItem {
                id: SharedString::from(*id),
                name: SharedString::from(icons::category_label(id)),
                count: count as i32,
            });
        }
    }
    out
}

fn populate_grid(ui: &App, installed: &Arc<Vec<DesktopEntry>>, query: &str, category: &str) {
    let query_lower = query.to_lowercase();
    let apps: Vec<AppGridItem> = installed
        .iter()
        .filter(|entry| category == "all" || icons::category_id(&entry.categories) == category)
        .filter(|entry| {
            if query_lower.is_empty() {
                return true;
            }
            entry.name.to_lowercase().contains(&query_lower)
                || entry.app_id.to_lowercase().contains(&query_lower)
                || entry.categories.to_lowercase().contains(&query_lower)
                || entry.comment.to_lowercase().contains(&query_lower)
        })
        .map(|entry| {
            let icon = icons::resolve(&entry.icon);
            AppGridItem {
                app_id: entry.app_id.clone().into(),
                name: entry.name.clone().into(),
                icon_char: entry.icon_char.clone().into(),
                has_icon: icon.is_some(),
                icon: icon.unwrap_or_default(),
                category: SharedString::from(icons::category_id(&entry.categories)),
            }
        })
        .collect();
    ui.set_grid_apps(ModelRc::new(VecModel::from(apps)));
}
