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
                return dock_model::mind_view_label(group);
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

    // Keyed by app: a window's title changes every few seconds (a browser tab, a terminal), and a
    // replaced model clears the dock's hover label and closes a list the person is reading. Only
    // a different set of apps replaces it.
    if let Some(model) = crate::models::update(ui.get_dock_buttons(), buttons, |b| b.app_id.clone()) {
        ui.set_dock_buttons(model);
    }
    if let Some(model) = crate::models::changed(ui.get_dock_windows(), rows) {
        ui.set_dock_windows(model);
    }
    ui.set_dock_reveal_index(reveal);
    // The launcher's Running section reads these rows, so an app that opens while it is up shows.
    super::launcher::refresh_running(ui);
}

/// What `describe shell` says about the dock: its buttons in order, and the page being shown.
///
/// Read back from the models the screen is drawn from, not from a second computation, so it is the
/// dock as it is — including the buttons paged out of sight, marked `shown: false`.
pub fn for_describe(ui: &App) -> serde_json::Value {
    let buttons = ui.get_dock_buttons();
    let rows: Vec<DescribeRow> = buttons
        .iter()
        .map(|b| DescribeRow {
            app: b.app_id.to_string(),
            label: b.label.to_string(),
            pinned: b.pinned,
            running: b.running,
            windows: b.windows,
            focused: b.focused,
        })
        .collect();
    describe_value(&rows, ui.get_dock_capacity().max(0) as usize, ui.get_dock_first().max(0) as usize, ui.get_cards_pending() > 0)
}

/// One button as `describe shell` reports it.
pub struct DescribeRow {
    pub app: String,
    pub label: String,
    pub pinned: bool,
    pub running: bool,
    pub windows: i32,
    pub focused: bool,
}

/// The `dock` field of `describe shell`, from plain data so its shape is tested without a screen.
pub fn describe_value(rows: &[DescribeRow], capacity: usize, first: usize, needs_you: bool) -> serde_json::Value {
    let page = dock_model::page(rows.len(), capacity, first);
    let list: Vec<serde_json::Value> = rows
        .iter()
        .enumerate()
        .map(|(i, b)| {
            serde_json::json!({
                "app": b.app,
                "label": b.label,
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
        "needs_you": needs_you,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(app: &str, running: bool) -> DescribeRow {
        DescribeRow { app: app.into(), label: app.into(), pinned: true, running, windows: i32::from(running), focused: false }
    }

    /// `describe shell` → `dock` is read by minds; its keys are a contract.
    #[test]
    fn describe_shell_dock_has_buttons_page_and_needs_you() {
        let rows = [row("files", false), row("notes", true), row("terminal", true), row("browser", false)];
        let v = describe_value(&rows, 2, 1, true);
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort();
        assert_eq!(keys, ["buttons", "needs_you", "page"]);
        assert_eq!(v["needs_you"], true);
        let b = v["buttons"].as_array().unwrap();
        assert_eq!(b.len(), 4, "every button, paged out of sight or not");
        for k in ["app", "label", "pinned", "running", "windows", "focused", "shown"] {
            assert!(b[0].get(k).is_some(), "a button reports `{k}`");
        }
        let shown: Vec<bool> = b.iter().map(|x| x["shown"].as_bool().unwrap()).collect();
        assert_eq!(shown, [false, true, true, false], "the page is the two after the first");
        assert_eq!((v["page"]["first"].as_u64(), v["page"]["before"].as_u64(), v["page"]["after"].as_u64()), (Some(1), Some(1), Some(1)));
    }

    /// A mind's own desk is one button named by what Mind View calls it; the apps a mind opens
    /// inside it never become buttons of their own, because `windows::shell_windows` removes them
    /// before the dock sees the list. Pinned as a source scan: the filter is one line, and losing it
    /// would put a mind's apps on the person's dock.
    #[test]
    fn a_minds_apps_stay_off_the_dock() {
        let src = include_str!("../windows.rs");
        let f = &src[src.find("pub fn shell_windows").unwrap()..];
        let f = &f[..f.find("\n}\n").unwrap()];
        assert!(f.contains("mind_view::app_pids()") && f.contains("launched.retain(|app| !in_mind_view.contains(&app.pid))"), "shell_windows drops the apps a mind opened:\n{f}");
        // And the dock reads only that list.
            }

    /// The dock's windows come from the shell's own window list, and only from it.
    #[test]
    fn the_dock_reads_the_shells_window_list() {
        let whole = include_str!("dock_bar.rs");
        let me: String = whole.split("#[cfg(test)]").next().unwrap().split_whitespace().collect();
        assert!(me.contains("ui.get_window_list()"));
        assert!(!me.contains("compositor_snapshot") && !me.contains("running::running"), "no second source of windows");
    }
}
