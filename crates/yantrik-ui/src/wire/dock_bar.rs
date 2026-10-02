//! The grounded dock's data: pinned and running apps as buttons, and the windows behind them.
//!
//! Read from the shell's own `window-list` model, which the system poll fills from the compositor
//! and the launch registry, and from the pinned list. One snapshot, grouped by app
//! (`dock_model`), so the dock, the window list and `describe shell` cannot disagree.
//!
//! Nothing here asks the compositor anything: it runs on the UI thread, inside the poll.

use std::cell::RefCell;

use slint::{ComponentHandle, Model, SharedString};

use super::pins;
use crate::apps::DesktopEntry;
use crate::dock_model::{self, LaunchOrder, Pin, Win};
use crate::{App, DockButton, DockWindow, Tr};

thread_local! {
    static ORDER: RefCell<LaunchOrder> = RefCell::new(LaunchOrder::default());
    /// The app that was in front when the dock last looked. Paging follows focus only when focus
    /// MOVES: Alt+Tab to an app on another page reveals that page, and a person paging by hand
    /// is not dragged back every three seconds.
    static FRONT: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn entry_of<'a>(installed: &'a [DesktopEntry], id: &str) -> Option<&'a DesktopEntry> {
    installed.iter().find(|e| e.app_id == id || pins::pin_id(&e.app_id) == id)
}

/// Whether the shell draws this app's tile from its own glyph set (so it has a name in
/// `APP_NAMES`, or is one of the launcher's built-ins), as opposed to a real icon from the theme.
fn has_own_glyph(tr: &Tr<'_>, id: &str) -> bool {
    pins::builtin_label(tr, id).is_some() || crate::windows::APP_NAMES.iter().any(|(app, _)| *app == id)
}

/// Put the dock's buttons and its window list on the screen.
pub fn publish(ui: &App, installed: &[DesktopEntry]) {
    let tr = ui.global::<Tr>();
    let wins: Vec<Win> = ui
        .get_window_list()
        .iter()
        .map(|w| Win { title: w.title.to_string(), app_id: w.app_id.to_string(), subtitle: w.subtitle.to_string() })
        .collect();
    // The list's first entry is the window in front (`windows::shell_windows`, `window_items`).
    let front = wins.first().map(|w| dock_model::dock_id(&w.app_id).to_string());

    let running = |id: &str| wins.iter().any(|w| dock_model::dock_id(&w.app_id) == id);
    let pinned: Vec<Pin> = super::settings::pinned_apps()
        .iter()
        .filter(|id| pins::is_pinnable(id))
        // The same promise `pins::publish` keeps: a pin that cannot open is not drawn — unless it
        // is open anyway, in which case it is a running app.
        .filter(|id| running(id) || super::dock::is_launchable(id, installed))
        .map(|id| Pin {
            app_id: id.clone(),
            label: pins::builtin_label(&tr, id)
                .map(|l| l.to_string())
                .or_else(|| entry_of(installed, id).map(|e| e.name.clone()))
                .unwrap_or_else(|| pins::humanise(id)),
        })
        .collect();

    let entries = ORDER.with(|order| {
        dock_model::entries(&pinned, &wins, front.as_deref(), &mut order.borrow_mut(), |id, group| {
            if id == "mind-view" {
                // A mind's desk: labelled as such, by what Mind View calls it.
                return group
                    .iter()
                    .map(|w| w.subtitle.clone())
                    .find(|s| !s.is_empty())
                    .unwrap_or_else(|| "Mind View".to_string());
            }
            entry_of(installed, id)
                .filter(|_| !has_own_glyph(&tr, id))
                .map(|e| e.name.clone())
                .unwrap_or_else(|| crate::windows::app_display_name(id))
        })
    });

    let buttons: Vec<DockButton> = entries
        .iter()
        .map(|e| {
            // A real icon only for apps this shell draws no glyph for, as the launcher does.
            let icon = if has_own_glyph(&tr, &e.app_id) {
                None
            } else {
                entry_of(installed, &e.app_id).and_then(|d| crate::icons::resolve(&d.icon))
            };
            DockButton {
                app_id: e.app_id.clone().into(),
                label: e.label.clone().into(),
                icon_id: super::app_grid::icon_id_for(&e.app_id).into(),
                has_icon: icon.is_some(),
                icon: icon.unwrap_or_default(),
                pinned: e.pinned,
                running: e.running(),
                focused: e.focused,
                windows: e.windows.len() as i32,
                title: e.windows.first().cloned().map(SharedString::from).unwrap_or_default(),
            }
        })
        .collect();

    let rows: Vec<DockWindow> = wins
        .iter()
        .enumerate()
        .map(|(i, w)| DockWindow {
            app_id: dock_model::dock_id(&w.app_id).into(),
            title: w.title.clone().into(),
            // labwc does not tell this shell which workspace a window is on, so none is claimed.
            workspace: SharedString::default(),
            current: i == 0,
        })
        .collect();

    // Page to the app that has just come to the front, when it is off the page.
    let moved = FRONT.with(|last| {
        let mut last = last.borrow_mut();
        let moved = *last != front;
        *last = front.clone();
        moved
    });
    let reveal = match (&front, moved) {
        (Some(f), true) => entries.iter().position(|e| e.app_id == *f).map_or(-1, |i| i as i32),
        _ => -1,
    };

    if let Some(model) = crate::models::changed(ui.get_dock_buttons(), buttons) {
        ui.set_dock_buttons(model);
    }
    if let Some(model) = crate::models::changed(ui.get_dock_windows(), rows) {
        ui.set_dock_windows(model);
    }
    ui.set_dock_reveal_index(reveal);
}

/// What `describe shell` says about the dock: its buttons in order, and the page being shown.
///
/// Read back from the models the screen is drawn from, not from a second computation, so it is the
/// dock as it is — including the buttons paged out of sight, marked `shown: false`.
pub fn for_describe(ui: &App) -> serde_json::Value {
    let buttons = ui.get_dock_buttons();
    let total = buttons.row_count();
    let page = dock_model::page(total, ui.get_dock_capacity().max(0) as usize, ui.get_dock_first().max(0) as usize);
    let list: Vec<serde_json::Value> = buttons
        .iter()
        .enumerate()
        .map(|(i, b)| {
            serde_json::json!({
                "app": b.app_id.to_string(),
                "label": b.label.to_string(),
                "pinned": b.pinned,
                "running": b.running,
                "windows": b.windows,
                "focused": b.focused,
                "shown": i >= page.first && i < page.first + page.shown,
            })
        })
        .collect();
    serde_json::json!({
        "buttons": list,
        "page": {
            "first": page.first,
            "shown": page.shown,
            "before": page.before,
            "after": page.after,
        },
        "needs_you": ui.get_cards_pending() > 0,
    })
}

