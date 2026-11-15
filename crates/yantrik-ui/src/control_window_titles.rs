//! The window titles `describe shell` hands a mind (#1247, review R3-6).
//!
//! Yantrik's own windows have fixed titles. A foreign app's title follows what it has open:
//! gedit says `tokens.json (~/backup) - gedit`, a terminal editor its file. That names a protected
//! file to a mind that is not allowed to know the file is there. For a mind, a title with a secret
//! name in it is replaced by the app's display name and marked `title_hidden`; the person's own
//! view is whole.

use yantrik_ipc_contracts::home_paths::is_secret_name;

use crate::windows::{app_display_name, WindowEntry};

/// What `in_front` says when the front window's title is hidden and no listed window has it.
const A_WINDOW: &str = "a window";

/// Whether a word of `title` is a secret name. Words are split where a title puts a file's name
/// beside other text: spaces, brackets, quotes, path separators and the like; `tokens.json*`
/// (an editor's unsaved mark) is the name `tokens.json`.
fn names_a_secret(title: &str) -> bool {
    title
        .split(|c: char| c.is_whitespace() || "()[]{}<>\"'`/\\,;|*•—".contains(c))
        .filter(|word| !word.is_empty())
        .any(is_secret_name)
}

/// One row of `windows` for `describe shell`: `title` and `app`, and `title_hidden: true` in place
/// of the title when `mind` is asking and the title names a secret.
pub fn row(window: &WindowEntry, mind: bool) -> serde_json::Value {
    if mind && names_a_secret(&window.title) {
        serde_json::json!({ "title": app_display_name(&window.app_id), "app": window.app_id, "title_hidden": true })
    } else {
        serde_json::json!({ "title": window.title, "app": window.app_id })
    }
}

/// `in_front`, the front window's title as `windows` lists it, for `mind` asking: the app's display
/// name when the title names a secret.
pub fn in_front(title: Option<String>, windows: &[WindowEntry], mind: bool) -> Option<String> {
    let title = title?;
    if !(mind && names_a_secret(&title)) {
        return Some(title);
    }
    Some(windows.iter().find(|w| w.title == title).map_or_else(|| A_WINDOW.to_string(), |w| app_display_name(&w.app_id)))
}

#[cfg(test)]
#[path = "control_window_titles_tests.rs"]
mod tests;
