//! The window titles `describe shell` hands a mind (#1247, review R3-6).
//!
//! Yantrik's own windows have fixed titles. A foreign app's title follows what it has open:
//! gedit says `tokens.json (~/backup) - gedit`, a terminal editor its file. That names a protected
//! file to a mind that is not allowed to know the file is there. For a mind, a title with a secret
//! name in it is replaced by the app's display name and marked `title_hidden`; the person's own
//! view is whole.

use yantrik_ipc_contracts::home_paths::title_names_a_secret;

use crate::windows::{app_display_name, WindowEntry};

/// What `in_front` says when the front window's title is hidden and no listed window has it.
const A_WINDOW: &str = "a window";

/// What the title of a notification says when it is hidden.
const A_NOTICE: &str = "a notification";

/// Whether a title is hidden from the caller: it is a mind, and the title names a secret.
pub fn hides(title: &str, mind: bool) -> bool {
    mind && title_names_a_secret(title)
}

/// The app's id and display name as a mind is shown them. A window that declares no app id gets one
/// derived from its title, so an id can name a secret as the title did: `tokens.json - foo` has the
/// id `tokens.json`, and `tokens (copy 1).json - foo` the id `tokens`: a first word of a hidden
/// title that is the id. Then both are [`A_WINDOW`]'s words, whatever the title says (reviews R6-M1,
/// R7-L1). `title` is the title the id stands beside.
pub fn shown_app(app_id: &str, title: &str, mind: bool) -> (String, String) {
    let name = app_display_name(app_id);
    let derived = title.to_lowercase().split_whitespace().next() == Some(app_id) && hides(title, mind);
    if derived || hides(app_id, mind) || hides(&name, mind) {
        ("window".to_string(), A_WINDOW.to_string())
    } else {
        (app_id.to_string(), name)
    }
}

/// One cell of the window switcher as `describe shell` reports it: its `name` (which is the window's
/// title when the app has no name of its own) and its `title`, and `title_hidden: true` in place of
/// both, with the app's display name, when `mind` is asking and either names a secret.
pub fn cell(name: &str, title: &str, app_id: &str, mind: bool) -> serde_json::Value {
    let (app, shown) = shown_app(app_id, title, mind);
    if hides(title, mind) || hides(name, mind) {
        serde_json::json!({ "name": shown, "title": shown, "app": app, "title_hidden": true })
    } else {
        serde_json::json!({ "name": name, "title": title, "app": app })
    }
}

/// The title of a notification for `describe shell`: [`A_NOTICE`] when `mind` is asking and it
/// names a secret (a download's notice names the file), and whether it was hidden.
pub fn notice_title(title: &str, mind: bool) -> (String, bool) {
    if hides(title, mind) { (A_NOTICE.to_string(), true) } else { (title.to_string(), false) }
}

/// A dock button's `app`, its label and whether the label was hidden, for `describe shell`: the
/// label is the app's display name when `mind` is asking and it names a secret (an app with no name
/// of its own is labelled by its window's title), and the app is as [`shown_app`] gives it (reviews
/// R5-M2, R6-M1).
pub fn dock_label(label: &str, app_id: &str, mind: bool) -> (String, String, bool) {
    let (app, shown) = shown_app(app_id, label, mind);
    if hides(label, mind) { (app, shown, true) } else { (app, label.to_string(), false) }
}

/// One row of `windows` for `describe shell`: `title` and `app`, and `title_hidden: true` in place
/// of the title when `mind` is asking and the title names a secret.
pub fn row(window: &WindowEntry, mind: bool) -> serde_json::Value {
    let (app, shown) = shown_app(&window.app_id, &window.title, mind);
    if hides(&window.title, mind) {
        serde_json::json!({ "title": shown, "app": app, "title_hidden": true })
    } else {
        serde_json::json!({ "title": window.title, "app": app })
    }
}

/// `in_front`, the front window's title as `windows` lists it, for `mind` asking: the app's display
/// name when the title names a secret.
pub fn in_front(title: Option<String>, windows: &[WindowEntry], mind: bool) -> Option<String> {
    let title = title?;
    if !hides(&title, mind) {
        return Some(title);
    }
    Some(windows.iter().find(|w| w.title == title).map_or_else(|| A_WINDOW.to_string(), |w| shown_app(&w.app_id, &w.title, mind).1))
}

#[cfg(test)]
#[path = "control_window_titles_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "control_window_titles_regression_tests.rs"]
mod regression_tests;
