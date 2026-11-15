//! The window titles `describe shell` hands a mind (#1247, review R3-6).
//!
//! Yantrik's own windows have fixed titles. A foreign app's title follows what it has open:
//! gedit says `tokens.json (~/backup) - gedit`, a terminal editor its file. That names a protected
//! file to a mind that is not allowed to know the file is there. For a mind, a title with a secret
//! name in it is replaced by the app's display name and marked `title_hidden`; the person's own
//! view is whole.

use yantrik_ipc_contracts::home_paths::{is_protected, is_secret_name};

use crate::windows::{app_display_name, WindowEntry};

/// What `in_front` says when the front window's title is hidden and no listed window has it.
const A_WINDOW: &str = "a window";

/// What the title of a notification says when it is hidden.
const A_NOTICE: &str = "a notification";

/// Characters that may be part of a file's name inside a title. Every other character (spaces,
/// brackets, quotes of every kind, dashes, bullets, ellipses) ends a word (review M4).
const IN_A_NAME: &str = "._-~#%!:+@$";

/// Whether `title` names a secret: a word of it is a secret name (also with the full stop, `?` or
/// `:` a sentence puts after it: `Opening tokens.json.`), or a word that is a path (`~/.aws`,
/// `/home/ann/.ssh/id_rsa`) is a protected place. `tokens.json*` (an editor's unsaved mark) is the
/// name `tokens.json`.
fn names_a_secret(title: &str) -> bool {
    let named = |word: &str| is_secret_name(word) || is_secret_name(word.trim_end_matches(['.', '?', '!', ':']));
    words(title, "").any(named) || words(title, "/").filter(|word| word.contains('/')).any(is_a_protected_path)
}

/// The words of `title`: runs of letters, digits, [`IN_A_NAME`] and `also`.
fn words<'a>(title: &'a str, also: &'static str) -> impl Iterator<Item = &'a str> {
    title.split(move |c: char| !(c.is_alphanumeric() || IN_A_NAME.contains(c) || also.contains(c))).filter(|word| !word.is_empty())
}

/// Whether `word`, with a `~/` start read as the home, is an absolute path into a protected place.
fn is_a_protected_path(word: &str) -> bool {
    let path = match word.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => std::path::Path::new(&home).join(rest),
            None => return false,
        },
        None => std::path::PathBuf::from(word),
    };
    path.is_absolute() && is_protected(&path)
}

/// Whether a title is hidden from the caller: it is a mind, and the title names a secret.
pub fn hides(title: &str, mind: bool) -> bool {
    mind && names_a_secret(title)
}

/// One cell of the window switcher as `describe shell` reports it: its `name` (which is the window's
/// title when the app has no name of its own) and its `title`, and `title_hidden: true` in place of
/// both, with the app's display name, when `mind` is asking and either names a secret.
pub fn cell(name: &str, title: &str, app_id: &str, mind: bool) -> serde_json::Value {
    if hides(title, mind) || hides(name, mind) {
        let shown = app_display_name(app_id);
        serde_json::json!({ "name": shown, "title": shown, "app": app_id, "title_hidden": true })
    } else {
        serde_json::json!({ "name": name, "title": title, "app": app_id })
    }
}

/// The title of a notification for `describe shell`: [`A_NOTICE`] when `mind` is asking and it
/// names a secret (a download's notice names the file), and whether it was hidden.
pub fn notice_title(title: &str, mind: bool) -> (String, bool) {
    if hides(title, mind) { (A_NOTICE.to_string(), true) } else { (title.to_string(), false) }
}

/// One row of `windows` for `describe shell`: `title` and `app`, and `title_hidden: true` in place
/// of the title when `mind` is asking and the title names a secret.
pub fn row(window: &WindowEntry, mind: bool) -> serde_json::Value {
    if hides(&window.title, mind) {
        serde_json::json!({ "title": app_display_name(&window.app_id), "app": window.app_id, "title_hidden": true })
    } else {
        serde_json::json!({ "title": window.title, "app": window.app_id })
    }
}

/// `in_front`, the front window's title as `windows` lists it, for `mind` asking: the app's display
/// name when the title names a secret.
pub fn in_front(title: Option<String>, windows: &[WindowEntry], mind: bool) -> Option<String> {
    let title = title?;
    if !hides(&title, mind) {
        return Some(title);
    }
    Some(windows.iter().find(|w| w.title == title).map_or_else(|| A_WINDOW.to_string(), |w| app_display_name(&w.app_id)))
}

#[cfg(test)]
#[path = "control_window_titles_tests.rs"]
mod tests;
