//! What a Mind's `files_*` calls do instead of driving the person's Files screen (#1033).
//!
//! The `files_*` actions were written for the Files screen: `files_go` took the screen to a
//! folder, and `files_new_folder`, `files_rename` and the rest then acted on "the folder on
//! screen". For a Mind that moved the person's view and acted on whatever they had left open. A
//! Mind's call now never touches the screen, `current-screen`, the selection or the clipboard:
//!
//! - **Explicit paths.** `files_new_folder`, `files_new_file`, `files_rename`, `files_delete` and
//!   `files_list` take `path` (the entry itself, or the folder for a listing), or `parent` and
//!   `name`. Nothing about the person's screen is read.
//! - **A private cursor.** `files_go`, `files_enter` and `files_up` move a folder kept per
//!   caller, never shown; a call with neither `path` nor `parent` acts in the caller's own cursor
//!   (the home, until it moves). Two Minds keep two cursors.
//! - Whatever acts on the person's selection, clipboard or Trash (`files_select`, `files_copy`,
//!   `files_cut`, `files_paste`, `files_trash_selected`, `files_undo_trash`, `files_toggle_trash`,
//!   `files_cancel`, `files_terminal`) or opens a window (`files_open`) is refused for a Mind
//!   with a sentence.
//!
//! The checks are the ones the Files screen ran for a Mind, unchanged: only the person's home,
//! outside its protected places, links followed (`control_files_mind`). The person's own calls
//! never come here: [`intercept`] answers `None` for them.

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::control_files_mind as mind;
use crate::mind_view::{mind_id_now, requester_now, Requester};

/// The folder each Mind is "in", by the caller's identity. Never drawn anywhere.
static CURSORS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Who is calling, as a key for its cursor: `None` for the person. A Mind the shell cannot name
/// shares the cursor of its kind ("an agent", "a mind"), never the person's.
pub(crate) fn caller_key() -> Option<String> {
    match requester_now() {
        Requester::Person => None,
        Requester::Mind(who) => Some(mind_id_now().unwrap_or(who)),
    }
}

pub(crate) fn cursor_of(key: &str) -> String {
    let all = CURSORS.lock().unwrap_or_else(|e| e.into_inner());
    all.iter().find(|(k, _)| k == key).map(|(_, p)| p.clone()).unwrap_or_else(|| "~".to_string())
}

pub(crate) fn move_cursor(key: &str, to: &str) {
    let mut all = CURSORS.lock().unwrap_or_else(|e| e.into_inner());
    match all.iter_mut().find(|(k, _)| k == key) {
        Some((_, at)) => *at = to.to_string(),
        None => all.push((key.to_string(), to.to_string())),
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

/// A folder or path as a caller may spell it: absolute, or from the home.
fn spelled(path: &str) -> Result<&str, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("`path` is empty".into());
    }
    if path == "~" || path.starts_with("~/") || path.starts_with('/') {
        Ok(path)
    } else {
        Err(format!("`{path}` is not an absolute path; give it from the root or as ~/…"))
    }
}

fn text<'a>(args: &'a Value, key: &str) -> &'a str {
    args[key].as_str().unwrap_or_default().trim()
}

/// An entry and the folder it is in, as one of the three forms names it: `path`; `parent` with
/// `name`; or `name` alone, in `fallback` (the Mind's cursor).
pub(crate) fn entry_in(args: &Value, fallback: Option<&str>) -> Result<Option<(String, String)>, String> {
    let (path, parent, name) = (text(args, "path"), text(args, "parent"), text(args, "name"));
    if !path.is_empty() {
        let path = spelled(path)?.trim_end_matches('/');
        let (parent, leaf) = path.rsplit_once('/').ok_or_else(|| format!("`{path}` names no entry"))?;
        let parent = match (parent, path.starts_with('/')) {
            ("", true) => "/",
            ("", false) => "~",
            (parent, _) => parent,
        };
        if leaf.is_empty() {
            return Err(format!("`{path}` names no entry"));
        }
        return Ok(Some((parent.to_string(), leaf.to_string())));
    }
    if name.is_empty() {
        return Err("give `path`, or `parent` and `name`, or `name` in the folder you are in".into());
    }
    if !parent.is_empty() {
        return Ok(Some((spelled(parent)?.trim_end_matches('/').to_string(), name.to_string())));
    }
    Ok(fallback.map(|folder| (folder.to_string(), name.to_string())))
}

/// The folder read back for an answer, checked again on what it is now, links resolved: a folder
/// swapped for a link since the check on the UI thread would otherwise be read wherever it leads.
pub(crate) fn read_folder(path: &str, for_mind: bool) -> Result<Value, String> {
    if for_mind {
        let home = home();
        let home = std::fs::canonicalize(&home).unwrap_or(home);
        if let Ok(resolved) = std::fs::canonicalize(crate::filebrowser::expand_home(path)) {
            mind::open_verdict(&resolved.to_string_lossy(), &home)?;
        }
    }
    crate::control_files_create::listing(path)
}

/// Run `work` off the UI thread when the socket allows, and answer with what it returns.
pub(crate) fn deferred(work: impl FnOnce() -> Result<Value, String> + Send + 'static) -> Result<Value, String> {
    yantrik_app_runtime::control::answer_later(work)
        .map(|()| json!({ "answering": "off the UI thread" }))
        .or_else(|work| work())
}

/// `files_list`: what a folder holds, for anyone, without touching the screen. No `path` means the
/// caller's cursor; for the person, the folder on screen (`on_screen`, read on the UI thread).
pub(crate) fn list(args: &Value, on_screen: impl FnOnce() -> String) -> Result<Value, String> {
    let key = caller_key();
    let path = match (text(args, "path"), &key) {
        ("", Some(key)) => cursor_of(key),
        ("", None) => on_screen(),
        (path, _) => spelled(path)?.to_string(),
    };
    mind::may_open(&path)?;
    let for_mind = key.is_some();
    deferred(move || read_folder(&path, for_mind))
}

/// Take the Mind's cursor to `path` and say what is there.
fn go(key: String, path: String) -> Result<Value, String> {
    spelled(&path)?;
    mind::open_verdict(&path, &home())?;
    deferred(move || {
        let listing = read_folder(&path, true)?;
        move_cursor(&key, &path);
        Ok(listing)
    })
}

/// A Mind's call to a `files_*` action: `None` for the person (the action goes on as written),
/// and for a Mind the answer, reached without the person's screen, selection or clipboard.
/// `files_list` and `files_stat` are not here, and `files_new_folder` and `files_new_file` are
/// the same action for everyone (`make_target`): none of them looks at the screen for a Mind.
pub(crate) fn intercept(action: &str, args: &Value) -> Option<Result<Value, String>> {
    let key = caller_key()?;
    Some(act(&key, action, args))
}

/// [`intercept`] once the caller is known.
pub(crate) fn act(key: &str, action: &str, args: &Value) -> Result<Value, String> {
    let here = || cursor_of(key);
    match action {
        "files_go" => go(key.to_string(), text(args, "path").to_string()),
        "files_enter" => {
            let name = crate::fileops::name(text(args, "name"))?;
            go(key.to_string(), mind::into(&here(), name))
        }
        "files_up" => go(key.to_string(), mind::up_from(&here())),
        "files_refresh" => {
            let path = here();
            mind::open_verdict(&path, &home())?;
            deferred(move || read_folder(&path, true))
        }
        "files_view" => {
            let view = text(args, "view");
            if view != "grid" && view != "list" {
                return Err(format!("`view` is grid or list, not `{view}`"));
            }
            Ok(json!({
                "view": view,
                "changed": false,
                "note": "grid or list is how the person's own Files screen is drawn; it was left as it is",
            }))
        }
        "files_rename" => rename(key, args),
        "files_delete" => delete(key, args),
        "files_open" => Err(crate::shell_view::sentence("files")),
        other => Err(refusal(other)),
    }
}

/// What a Mind is told for an action that works on the person's own screen state.
pub(crate) fn refusal(action: &str) -> String {
    format!(
        "{action} works on the person's own Files screen, selection and clipboard, which a Mind's \
         call never touches; use files_go or files_list with a path, and files_new_folder, \
         files_new_file, files_rename and files_delete with `path`."
    )
}

fn rename(key: &str, args: &Value) -> Result<Value, String> {
    let (parent, name) = entry_in(args, Some(&cursor_of(key)))?.ok_or("nothing to rename")?;
    let new = text(args, "new_name").to_string();
    crate::fileops::name(&new)?;
    let home = home();
    mind::rename_verdict(&parent, &name, &new, &home)?;
    deferred(move || {
        // Again on the worker, on what the folder is now.
        mind::rename_verdict(&parent, &name, &new, &home)?;
        let dir = crate::filebrowser::expand_home(&parent);
        crate::fileops::rename_no_replace(&dir.join(&name), &dir.join(&new))?;
        Ok(json!({ "renamed": dir.join(&name).display().to_string(), "to": dir.join(&new).display().to_string() }))
    })
}

fn delete(key: &str, args: &Value) -> Result<Value, String> {
    let (parent, name) = entry_in(args, Some(&cursor_of(key)))?.ok_or("nothing to delete")?;
    crate::fileops::name(&name)?;
    let home = home();
    mind::entry_verdict(&parent, &name, &home)?;
    deferred(move || {
        mind::entry_verdict(&parent, &name, &home)?;
        let path: PathBuf = crate::filebrowser::expand_home(&parent).join(&name);
        crate::fileops::trash(&path, &crate::fileops::trash_root())?;
        Ok(json!({ "requested_trash": name, "trashed": path.display().to_string() }))
    })
}

/// Where `files_new_folder` and `files_new_file` make their entry, and what to make.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Make {
    /// Every level of this `~/…` path, making the missing ones.
    Levels(String),
    /// `name` inside `dir`, with `label` the folder as the caller spelled it.
    In { dir: PathBuf, label: String, name: String },
}

/// Work out where `files_new_*` makes its entry from `path`, `parent` and `name`, or from the
/// folder `fallback` (the Mind's cursor, or the folder on screen for the person).
pub(crate) fn make_target(args: &Value, folder: bool, fallback: &str, home: &std::path::Path) -> Result<Make, String> {
    let (path, parent, name) = (text(args, "path"), text(args, "parent"), text(args, "name"));
    if !path.is_empty() {
        let full = crate::filebrowser::expand_home(spelled(path)?.trim_end_matches('/'));
        if folder {
            if let Ok(rest) = full.strip_prefix(home) {
                if !rest.as_os_str().is_empty() {
                    return Ok(Make::Levels(format!("~/{}", rest.display())));
                }
            }
        }
        let (label, leaf) = entry_in(args, None)?.ok_or("`path` names no entry")?;
        return Ok(Make::In { dir: crate::filebrowser::expand_home(&label), label, name: leaf });
    }
    if name.is_empty() {
        return Err("`name` is empty".into());
    }
    let label = if parent.is_empty() { fallback.to_string() } else { spelled(parent)?.trim_end_matches('/').to_string() };
    Ok(Make::In { dir: crate::filebrowser::expand_home(&label), label, name: name.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Mind's path never reads or moves the person's screen: nothing above the tests can.
    #[test]
    fn a_minds_path_cannot_reach_the_screen() {
        let src = include_str!("control_files_cursor.rs");
        let src = &src[..src.find("#[cfg(test)]").unwrap()];
        for token in ["get_current_screen", "set_current_screen", "ensure_files_screen", "invoke_", "get_file_browser", "set_file_", "ui.", "raise_shell"] {
            assert!(!src.contains(token), "control_files_cursor must not touch the screen: `{token}`");
        }
        let make = include_str!("control_files_make.rs");
        let make = &make[..make.find("#[cfg(test)]").unwrap_or(make.len())];
        let guard = make.find("if key.is_none() && !explicit").expect("the person-only switch");
        assert!(make.find("ensure_files_screen(").unwrap() > guard, "a Mind's make never switches the screen");
    }

    #[test]
    fn two_minds_keep_two_cursors_and_start_at_home() {
        let (a, b) = ("mind:test-a-1033", "mind:test-b-1033");
        assert_eq!(cursor_of(a), "~");
        move_cursor(a, "/home/p/docs");
        assert_eq!(cursor_of(a), "/home/p/docs");
        assert_eq!(cursor_of(b), "~", "another Mind's cursor did not move");
        move_cursor(b, "/home/p/music");
        move_cursor(a, "/home/p/docs/x");
        assert_eq!((cursor_of(a).as_str(), cursor_of(b).as_str()), ("/home/p/docs/x", "/home/p/music"));
    }

    #[test]
    fn a_mind_gets_a_sentence_for_what_acts_on_the_persons_screen() {
        let key = "mind:test-refuse-1033";
        for action in [
            "files_select", "files_copy", "files_cut", "files_paste", "files_trash_selected",
            "files_undo_trash", "files_toggle_trash", "files_cancel", "files_terminal", "files_anything_new",
        ] {
            let said = act(key, action, &json!({})).unwrap_err();
            assert_eq!(said, refusal(action));
            assert!(said.contains("never touches"), "{said}");
        }
        assert_eq!(
            act(key, "files_open", &json!({ "name": "a.txt" })).unwrap_err(),
            "Files can't open in your view; use the shell's files_* actions."
        );
        let view = act(key, "files_view", &json!({ "view": "grid" })).unwrap();
        assert_eq!(view["changed"], false);
        assert!(act(key, "files_view", &json!({ "view": "tiles" })).is_err());
    }

    #[test]
    fn an_entry_is_named_by_path_by_parent_and_name_or_by_the_cursor() {
        let e = |v: Value, fallback| entry_in(&v, fallback).unwrap();
        assert_eq!(e(json!({ "path": "/home/p/a/b" }), None), Some(("/home/p/a".into(), "b".into())));
        assert_eq!(e(json!({ "path": "~/a/" }), None), Some(("~".into(), "a".into())));
        assert_eq!(e(json!({ "path": "/x" }), None), Some(("/".into(), "x".into())));
        assert_eq!(e(json!({ "parent": "/home/p/a/", "name": "b" }), None), Some(("/home/p/a".into(), "b".into())));
        assert_eq!(e(json!({ "name": "b" }), Some("~/docs")), Some(("~/docs".into(), "b".into())));
        assert_eq!(e(json!({ "name": "b" }), None), None);
        for bad in [json!({}), json!({ "path": "relative/x" }), json!({ "path": "~" }), json!({ "parent": "docs", "name": "b" })] {
            assert!(entry_in(&bad, None).is_err(), "{bad}");
        }
    }

    #[test]
    fn new_folder_and_file_targets() {
        let home = std::path::Path::new("/home/p");
        std::env::set_var("HOME", "/home/p");
        let t = |v: Value, folder, at: &str| make_target(&v, folder, at, home);
        assert_eq!(t(json!({ "path": "/home/p/a/b" }), true, "~").unwrap(), Make::Levels("~/a/b".into()));
        assert_eq!(
            t(json!({ "path": "/home/p/a/b.txt" }), false, "~").unwrap(),
            Make::In { dir: "/home/p/a".into(), label: "/home/p/a".into(), name: "b.txt".into() }
        );
        assert_eq!(
            t(json!({ "parent": "/home/p/a", "name": "n" }), true, "~").unwrap(),
            Make::In { dir: "/home/p/a".into(), label: "/home/p/a".into(), name: "n".into() }
        );
        assert_eq!(
            t(json!({ "name": "n" }), true, "/home/p/cur").unwrap(),
            Make::In { dir: "/home/p/cur".into(), label: "/home/p/cur".into(), name: "n".into() }
        );
        assert!(t(json!({}), true, "~").is_err());
        assert!(t(json!({ "path": "docs/x" }), true, "~").is_err());
    }

    /// The person's `files_go` is as it was: it switches their screen and navigates; only a Mind is
    /// answered before that.
    #[test]
    fn the_persons_go_still_moves_their_screen() {
        let src = include_str!("control_files.rs");
        let go = &src[src.find("Action::new(\"files_go\"").unwrap()..];
        let go = &go[..go.find("Action::new(\"files_enter\"").unwrap()];
        assert!(go.find("files_cursor::intercept(").unwrap() < go.find("ensure_files_screen(").unwrap());
        assert!(go.contains("invoke_file_navigate_to_path"));
        // Every Mind-only branch sits behind `caller_key`, which is `None` for the person.
        let lib = include_str!("control_files_cursor.rs");
        assert!(lib.contains("Requester::Person => None"));
    }
}
