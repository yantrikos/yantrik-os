//! The window titles `describe shell` hands a mind (#1247, review R3-6).
//!
//! Yantrik's own windows have fixed titles. A foreign app's title follows what it has open:
//! gedit says `tokens.json (~/backup) - gedit`, a terminal editor its file. That names a protected
//! file to a mind that is not allowed to know the file is there. For a mind, a title with a secret
//! name in it is replaced by the app's display name and marked `title_hidden`; the person's own
//! view is whole.

use yantrik_ipc_contracts::home_paths::{is_a_copy_name, is_protected, is_secret_name, names_a_protected_copy, NAME_MAX};

use crate::windows::{app_display_name, WindowEntry};

/// What `in_front` says when the front window's title is hidden and no listed window has it.
const A_WINDOW: &str = "a window";

/// What the title of a notification says when it is hidden.
const A_NOTICE: &str = "a notification";

/// Characters that may be part of a file's name inside a title. Every other character (spaces,
/// brackets, quotes of every kind, dashes, bullets, ellipses) ends a word (review M4).
const IN_A_NAME: &str = "._-~#%!:+@$";

/// Whether `title` names a secret: a word of it, or a run of words with the spaces between them kept
/// that is at most [`NAME_MAX`] bytes (no file's name is longer, so no name is missed), is a secret
/// name (also with the full stop, `?` or `:` a sentence puts after it: `Opening tokens.json.`), or
/// an editor's copy of a protected file (`%home%ann%.kube%config.swp`), or a word that is a path
/// (`~/.aws`, `/home/ann/.ssh/id_rsa`, `My Stuff/.kube`) is a protected place. `tokens.json*` (an
/// editor's unsaved mark) is the name `tokens.json`. The runs matter because names have spaces in
/// them: `Login Data`, `tokens (copy 1).json`, `tokens - Copy.json`, and a Dropbox conflict label
/// of any length (reviews R5-M3, R6-L1).
fn names_a_secret(title: &str) -> bool {
    let named = |span: &str| {
        let bare = span.trim_end_matches(['.', '?', '!', ':']);
        is_secret_name(span) || (bare.len() < span.len() && is_secret_name(bare)) || names_a_protected_copy(span)
    };
    let bounds = word_bounds(title, "");
    let spans = bounds.iter().enumerate().flat_map(|(i, &(start, first_end))| {
        // A run that starts with a public key (`id_rsa.pub - gedit`, `id_rsa.pub.bak - gedit`) is no
        // private key that has words after it: the secret starts of names would take it for one.
        // Unless it is a whole copy name (`client_secret.pub - Copy.json`), which `is_secret_name`
        // reads the same way (review R6-L2). A run cut inside a bracket is not whole: the
        // `id_rsa.pub (copy 1` of `id_rsa.pub (copy 1) - gedit` is no copy of anything.
        let public = title[start..first_end].to_lowercase().contains(".pub");
        bounds[i..]
            .iter()
            .take_while(move |&&(_, end)| end - start <= NAME_MAX)
            .enumerate()
            .map(move |(n, &(_, end))| (n, &title[start..end]))
            .filter(move |&(n, span)| n == 0 || !public || is_a_whole_copy_name(span))
            .map(|(_, span)| span)
    });
    spans.into_iter().any(named)
        || word_bounds(title, "/").into_iter().map(|(start, end)| &title[start..end]).filter(|word| word.contains('/')).any(is_a_protected_path)
}

/// Whether `span` is a copy name by its marker and closes every bracket it opens.
fn is_a_whole_copy_name(span: &str) -> bool {
    is_a_copy_name(span) && span.matches('(').count() == span.matches(')').count()
}

/// The byte range of each word of `title`: a run of letters, digits, [`IN_A_NAME`] and `also`.
fn word_bounds(title: &str, also: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut start = None;
    // The end of the title closes the last word like any character that is not part of one.
    for (at, c) in title.char_indices().chain(std::iter::once((title.len(), ' '))) {
        let in_a_word = c.is_alphanumeric() || IN_A_NAME.contains(c) || also.contains(c);
        match (in_a_word, start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                found.push((from, at));
                start = None;
            }
            _ => {}
        }
    }
    found
}

/// Whether `word`, with a `~/` start read as the home, is a path into a protected place. A word
/// that is not absolute is judged below `/`, since `PROTECTED` matches its parts anywhere: the
/// `Stuff/.kube` of `~/My Stuff/.kube` is a protected place (review R6-L3).
fn is_a_protected_path(word: &str) -> bool {
    let path = match (word.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => std::path::Path::new(&home).join(rest),
        _ => std::path::Path::new("/").join(word),
    };
    is_protected(&path)
}

/// Whether a title is hidden from the caller: it is a mind, and the title names a secret.
pub fn hides(title: &str, mind: bool) -> bool {
    mind && names_a_secret(title)
}

/// The app's id and display name as a mind is shown them. A window that declares no app id gets one
/// derived from its title, so an id can name a secret as the title did: `tokens.json - foo` has the
/// id `tokens.json`. Then both are [`A_WINDOW`]'s words, whatever the title says (review R6-M1).
pub fn shown_app(app_id: &str, mind: bool) -> (String, String) {
    let name = app_display_name(app_id);
    if hides(app_id, mind) || hides(&name, mind) {
        ("window".to_string(), A_WINDOW.to_string())
    } else {
        (app_id.to_string(), name)
    }
}

/// One cell of the window switcher as `describe shell` reports it: its `name` (which is the window's
/// title when the app has no name of its own) and its `title`, and `title_hidden: true` in place of
/// both, with the app's display name, when `mind` is asking and either names a secret.
pub fn cell(name: &str, title: &str, app_id: &str, mind: bool) -> serde_json::Value {
    let (app, shown) = shown_app(app_id, mind);
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
    let (app, shown) = shown_app(app_id, mind);
    if hides(label, mind) { (app, shown, true) } else { (app, label.to_string(), false) }
}

/// One row of `windows` for `describe shell`: `title` and `app`, and `title_hidden: true` in place
/// of the title when `mind` is asking and the title names a secret.
pub fn row(window: &WindowEntry, mind: bool) -> serde_json::Value {
    let (app, shown) = shown_app(&window.app_id, mind);
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
    Some(windows.iter().find(|w| w.title == title).map_or_else(|| A_WINDOW.to_string(), |w| shown_app(&w.app_id, mind).1))
}

#[cfg(test)]
#[path = "control_window_titles_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "control_window_titles_regression_tests.rs"]
mod regression_tests;
