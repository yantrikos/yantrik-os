//! The launcher's search, and its Running section.
//!
//! One typed query filters both sections of the launcher: the apps that are running (the dock's
//! own rows, so the launcher, the dock and `describe shell` cannot disagree about what is open)
//! and everything installed. The filter is here, pure, so the two sections cannot match
//! differently, and so it is tested without a screen.
//!
//! Running comes from `dock-buttons`, which is built from the person's window list: an app a mind
//! opened is not in that list (`windows::shell_windows`), so it is never offered here either.

use std::cell::RefCell;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::{App, DockButton};

thread_local! {
    /// What the person has typed. Kept here, not read back from the field, because the dock's
    /// refresh (every few seconds while windows come and go) has to re-apply it to Running.
    static QUERY: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Whether `query` finds something with these fields. Every word has to match one of them,
/// case-insensitively, so "term foo" finds a Terminal whose window says "foo". An empty query
/// matches everything.
pub fn matches(query: &str, fields: &[&str]) -> bool {
    let fields: Vec<String> = fields.iter().map(|f| f.to_lowercase()).collect();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| fields.iter().any(|f| f.contains(word)))
}

pub fn query() -> String {
    QUERY.with(|q| q.borrow().clone())
}

/// A new query: remember it and redraw Running. (All apps is filtered by `app_grid`.)
pub fn set_query(ui: &App, query: &str) {
    QUERY.with(|q| *q.borrow_mut() = query.to_string());
    ui.set_grid_query(query.into());
    refresh_running(ui);
}

/// Put the running apps that match the query into the launcher.
///
/// Called when the launcher opens, when the query changes, and from the dock's refresh, so an app
/// that opens or closes while the launcher is up appears or goes without a keypress.
pub fn refresh_running(ui: &App) {
    let query = query();
    let rows: Vec<DockButton> = ui
        .get_dock_buttons()
        .iter()
        .filter(|b| b.running)
        .filter(|b| matches(&query, &[b.label.as_str(), b.app_id.as_str(), b.title.as_str()]))
        .collect();
    if let Some(model) = crate::models::update(ui.get_grid_running(), rows, |b| b.app_id.clone()) {
        ui.set_grid_running(model);
    }
}

/// Empty the query and the Running section, for a launcher that has just opened.
pub fn reset(ui: &App) {
    QUERY.with(|q| q.borrow_mut().clear());
    ui.set_grid_query("".into());
    ui.set_grid_running(ModelRc::new(VecModel::default()));
    refresh_running(ui);
}

/// The `launcher` field of `describe shell`: whether it is open, what is typed, and what the
/// person would see — the running apps with their window counts, and how many apps match.
pub fn for_describe(ui: &App) -> serde_json::Value {
    let running: Vec<serde_json::Value> = ui
        .get_grid_running()
        .iter()
        .map(|b| serde_json::json!({ "app": b.app_id.to_string(), "label": b.label.to_string(), "windows": b.windows }))
        .collect();
    describe_value(ui.get_app_grid_open(), &ui.get_grid_query(), running, ui.get_grid_apps().row_count())
}

/// The same, from plain data so its shape is tested without a screen.
pub fn describe_value(open: bool, query: &str, running: Vec<serde_json::Value>, apps_shown: usize) -> serde_json::Value {
    serde_json::json!({
        "open": open,
        "query": query,
        "running": running,
        "apps_shown": apps_shown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_query_matches_everything() {
        assert!(matches("", &["Files"]));
        assert!(matches("   ", &[]));
    }

    #[test]
    fn every_word_must_match_some_field() {
        assert!(matches("term", &["Terminal", "terminal", "~/src"]));
        assert!(matches("term src", &["Terminal", "terminal", "~/src"]));
        assert!(!matches("term docs", &["Terminal", "terminal", "~/src"]));
    }

    #[test]
    fn matching_ignores_case() {
        assert!(matches("FILES", &["files"]));
        assert!(matches("files", &["Files"]));
    }

    /// `describe shell` → `launcher` is read by minds; its keys are a contract.
    #[test]
    fn describe_launcher_has_open_query_running_and_apps_shown() {
        let v = describe_value(true, "te", vec![serde_json::json!({"app": "terminal", "label": "Terminal", "windows": 2})], 3);
        assert_eq!(v["open"], true);
        assert_eq!(v["query"], "te");
        assert_eq!(v["running"][0]["windows"], 2);
        assert_eq!(v["apps_shown"], 3);
    }

    /// A mind's apps stay off the person's launcher. Running is read from the dock's rows, and
    /// the dock's rows from the person's window list, which drops what a mind opened
    /// (`a_minds_apps_stay_off_the_dock` in dock_bar.rs holds that half). This holds the other:
    /// the launcher takes nothing from anywhere else.
    #[test]
    fn running_comes_only_from_the_docks_rows() {
        let src = include_str!("launcher.rs");
        let f = &src[src.find("pub fn refresh_running").unwrap()..src.find("/// Empty the query").unwrap()];
        assert!(f.contains("get_dock_buttons()"), "Running is built from the dock's rows:\n{f}");
        for forbidden in ["mind_view", "launched", "app_pids", "window_list"] {
            assert!(!f.contains(forbidden), "Running must not read `{forbidden}` itself:\n{f}");
        }
    }

    /// The dock's refresh has to keep Running current while the launcher is open.
    #[test]
    fn the_docks_refresh_updates_the_launchers_running_section() {
        let src = include_str!("dock_bar.rs");
        let f = &src[src.find("pub fn publish").unwrap()..src.find("pub fn for_describe").unwrap()];
        assert!(f.contains("launcher::refresh_running"), "dock_bar::publish refreshes the launcher's Running section:\n{f}");
    }
}
