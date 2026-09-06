//! The shell describes itself.
//!
//! Every app under `apps/` now publishes `app.describe` / `app.act`, and the desktop is the one
//! window that matters most: it is where the companion lives. Without this, "what is on my
//! desktop right now" was answerable only by screenshotting the shell and asking a vision model
//! to read a status bar we wrote ourselves.
//!
//! Published on the same bus under `app-shell`, so `list_apps` finds it beside the others.
//!
//! The surface is deliberately narrow. The shell already serves the companion — memory, tools and
//! answers all arrive over `companion.*` on its own socket — so this covers only what those cannot
//! say: which screen is up, which windows are open, which services are running, and what the
//! status bar is reporting about the machine.

use slint::ComponentHandle;
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param, View};

use crate::App;

/// The screens a caller may ask for by name.
///
/// Not every screen the shell can render. Boot, onboarding and login are states the shell enters
/// on its own and jumping into one would leave the session somewhere it cannot get back from;
/// locking has its own action, because locking a machine is an act rather than a view change.
const SCREENS: &[(&str, i32)] = &[
    ("desktop", 1),
    ("bond", 4),
    ("personality", 5),
    ("memory", 6),
    ("settings", 7),
    ("files", 8),
    ("notifications", 9),
    ("system", 10),
    ("terminal", 16),
    ("permissions", 28),
];

fn screen_name(id: i32) -> &'static str {
    match id {
        0 => "boot",
        2 => "onboarding",
        3 => "lock",
        11 => "image-viewer",
        12 => "text-editor",
        13 => "media-player",
        21 => "email",
        27 => "snippets",
        32 => "login",
        other => SCREENS
            .iter()
            .find(|(_, id)| *id == other)
            .map(|(name, _)| *name)
            .unwrap_or("unknown"),
    }
}

/// Join names the way a person would read them out: "a", "a and b", "a, b and c".
///
/// This line is the first thing anyone sees of the desktop, and "calendar and email and notes"
/// reads like a machine wrote it.
fn list_of(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Publish the desktop on the service bus. Call from the UI thread before `run()`.
pub fn publish(ui: &App) {
    let describe = {
        let weak = ui.as_weak();
        move || {
            let Some(ui) = weak.upgrade() else {
                return View::new("Yantrik — shutting down");
            };
            let screen = ui.get_current_screen();
            let bond = ui.get_bond_data();

            let windows = ui.get_window_list();
            let open: Vec<serde_json::Value> = {
                use slint::Model;
                (0..windows.row_count())
                    .filter_map(|i| windows.row_data(i))
                    .map(|w| {
                        serde_json::json!({
                            "title": w.title.to_string(),
                            "app": w.app_id.to_string(),
                        })
                    })
                    .collect()
            };

            let service_model = ui.get_services();
            let services: Vec<serde_json::Value> = {
                use slint::Model;
                (0..service_model.row_count())
                    .filter_map(|i| service_model.row_data(i))
                    .map(|s| {
                        serde_json::json!({
                            "id": s.id.to_string(),
                            "status": s.status.to_string(),
                            "note": s.note.to_string(),
                        })
                    })
                    .collect()
            };

            let down: Vec<&str> = services
                .iter()
                .filter(|s| {
                    matches!(s["status"].as_str(), Some("failed") | Some("stopped"))
                })
                .filter_map(|s| s["id"].as_str())
                .collect();

            // The one line worth reading first: where the user is, what is open, and whether
            // anything is wrong. Trouble comes before window count, because trouble is the
            // reason to look.
            let summary = if !down.is_empty() {
                format!(
                    "Yantrik — {} screen, {} windows open, {} not running",
                    screen_name(screen),
                    open.len(),
                    list_of(&down)
                )
            } else {
                format!(
                    "Yantrik — {} screen, {} windows open, CPU {}%, memory {}",
                    screen_name(screen),
                    open.len(),
                    ui.get_bar_cpu_percent(),
                    ui.get_bar_mem_text()
                )
            };

            View::new(summary)
                .with("screen", screen_name(screen))
                .with("screen_id", screen)
                .with("windows", serde_json::Value::Array(open))
                .with("services", serde_json::Value::Array(services))
                .with("companion_online", ui.get_companion_online())
                .with("companion_status", ui.get_companion_status().to_string())
                .with("thinking", ui.get_is_thinking())
                .with("pending_suggestions", ui.get_pending_count())
                .with("memories", ui.get_memory_count())
                .with("bond", bond.bond_level.to_string())
                .with("bond_score", bond.bond_score as f64)
                .with("active_project", ui.get_active_project().to_string())
                .with("clock", ui.get_clock_text().to_string())
                .with("date", ui.get_date_text().to_string())
                .with("cpu_percent", ui.get_bar_cpu_percent())
                .with("memory", ui.get_bar_mem_text().to_string())
                .with("memory_percent", ui.get_bar_mem_percent())
                .with("disk", ui.get_bar_disk_text().to_string())
                .with("disk_percent", ui.get_bar_disk_percent())
                .with("wifi", ui.get_wifi_connected())
                .with(
                    "battery",
                    if ui.get_battery_available() {
                        serde_json::json!({
                            "percent": ui.get_battery_level(),
                            "charging": ui.get_battery_charging(),
                        })
                    } else {
                        serde_json::Value::Null
                    },
                )
                .with("do_not_disturb", ui.get_dnd_mode())
                .with("incognito", ui.get_settings_incognito_mode())
        }
    };

    let weak = ui.as_weak();
    let ui_for = move || weak.upgrade().ok_or_else(|| "the shell is gone".to_string());

    let open_ui = ui_for.clone();
    let screen_ui = ui_for.clone();
    let focus_ui = ui_for.clone();
    let dnd_ui = ui_for.clone();
    let lock_ui = ui_for;

    ControlSurface::new("shell")
        .describe(describe)
        .action(
            Action::new("open_app", "Launch an app, or focus it if it is already running")
                .arg(Param::text("name").describe("App id, e.g. notes, email, terminal, files")),
            move |args| {
                let ui = open_ui()?;
                let name = args["name"].as_str().unwrap_or_default().trim().to_string();
                if name.is_empty() {
                    return Err("`name` is empty".into());
                }
                // The launcher's own path: it resolves the binary, enforces one window per app,
                // and focuses the running one instead of starting a second.
                ui.invoke_launch_app(name.clone().into());
                Ok(serde_json::json!({ "launching": name }))
            },
        )
        .action(
            Action::new("show_screen", "Switch the shell to one of its screens").arg(
                Param::text("screen")
                    .describe("desktop, files, settings, notifications, memory, system, terminal, permissions, bond, personality"),
            ),
            move |args| {
                let ui = screen_ui()?;
                let want = args["screen"].as_str().unwrap_or_default().trim().to_lowercase();
                let id = SCREENS
                    .iter()
                    .find(|(name, _)| *name == want)
                    .map(|(_, id)| *id)
                    .ok_or_else(|| {
                        let names: Vec<&str> = SCREENS.iter().map(|(n, _)| *n).collect();
                        format!("no screen called `{want}`; there is: {}", names.join(", "))
                    })?;
                // Set and invoke, in that order — the same pair every caller in the shell uses.
                // `navigate` is what loads a screen's data; setting the property alone shows an
                // empty one.
                ui.set_current_screen(id);
                ui.invoke_navigate(id);
                Ok(serde_json::json!({ "showing": want }))
            },
        )
        .action(
            Action::new("focus_window", "Bring an open window to the front")
                .arg(Param::text("title").describe("Window title, or part of one")),
            move |args| {
                let ui = focus_ui()?;
                let want = args["title"].as_str().unwrap_or_default().trim().to_lowercase();
                let windows = ui.get_window_list();
                let title = {
                    use slint::Model;
                    let rows: Vec<_> =
                        (0..windows.row_count()).filter_map(|i| windows.row_data(i)).collect();
                    rows.iter()
                        .find(|w| w.title.to_lowercase() == want)
                        .or_else(|| rows.iter().find(|w| w.title.to_lowercase().contains(&want)))
                        .map(|w| w.title.to_string())
                        .ok_or_else(|| {
                            let open: Vec<String> =
                                rows.iter().map(|w| w.title.to_string()).collect();
                            if open.is_empty() {
                                "no windows are open".to_string()
                            } else {
                                format!("no open window matches `{want}`; there is: {}", open.join(", "))
                            }
                        })?
                };
                ui.invoke_switch_window(title.clone().into());
                Ok(serde_json::json!({ "focused": title }))
            },
        )
        .action(
            Action::new("set_do_not_disturb", "Hold or release notifications")
                .arg(Param::flag("on")),
            move |args| {
                let ui = dnd_ui()?;
                let on = args["on"].as_bool().ok_or("`on` must be true or false")?;
                ui.set_dnd_mode(on);
                Ok(serde_json::json!({ "do_not_disturb": on }))
            },
        )
        .action(
            // Locking is not a view change: the person has to type their way back in. It gets its
            // own action and its own risk rather than hiding inside `show_screen`.
            Action::new("lock", "Lock the session").risk("sensitive"),
            move |_| {
                let ui = lock_ui()?;
                ui.invoke_lock_screen();
                Ok(serde_json::json!({ "locked": true }))
            },
        )
        .serve();
}
