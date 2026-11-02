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
//!   caller, never shown; `files_new_*` and `files_list` with no `path` act in the caller's own
//!   cursor (the home, until it moves). Two Minds keep two cursors. A caller the shell cannot name
//!   (a token it did not believe, the built-in companion) keeps none and gives explicit paths.
//! - **Delete and rename name their whole target.** The person's approval of a `files_delete`
//!   binds the arguments the card shows, so a Mind gives `path` (or `parent` and `name`); a bare
//!   `name` would be resolved against a cursor the Mind can move after the person has allowed it.
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
use crate::mind_view::{identity_now, Identity, requester_now, Requester};

/// The folder each Mind is "in", by the caller's identity. Never drawn anywhere.
static CURSORS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Who is calling, as far as a cursor is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Caller {
    /// The person: no cursor, their own Files screen.
    Person,
    /// A Mind the shell can tell from every other: `agent:<id>` or `mind:<name>`.
    Named(String),
    /// A Mind it cannot tell apart (a token it did not believe, the mind account with no attached
    /// mind, the built-in companion whose turns all arrive as one): no cursor, explicit paths.
    Unnamed,
}

impl Caller {
    pub(crate) fn is_mind(&self) -> bool {
        !matches!(self, Caller::Person)
    }

    /// The folder this caller is in: its cursor, when it has one.
    pub(crate) fn folder(&self) -> Option<String> {
        match self {
            Caller::Named(key) => Some(cursor_of(key)),
            Caller::Person | Caller::Unnamed => None,
        }
    }
}

/// Sort a requester, and who `identity_now` says it is, into a [`Caller`]. Only a Mind the shell
/// can tell from every other has a cursor.
pub(crate) fn caller_for(req: &Requester, who: Option<Identity>) -> Caller {
    match (req, who.and_then(|who| who.named_key())) {
        (Requester::Person, _) => Caller::Person,
        (Requester::Mind(_), Some(key)) => Caller::Named(key),
        (Requester::Mind(_), None) => Caller::Unnamed,
    }
}

pub(crate) fn caller_key() -> Caller {
    caller_for(&requester_now(), identity_now())
}

/// What a caller with no cursor is told when the call needed one.
pub(crate) const NEEDS_PATH: &str =
    "give a full `path`: the shell cannot tell your calls from another's, so it keeps no folder for you";

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

/// An entry and the folder it is in, as one of the two forms names it: `path`, or `parent` with
/// `name`. A bare `name` is refused: it would be resolved against a cursor that can move between
/// the person's approval and the call.
pub(crate) fn entry_in(args: &Value) -> Result<(String, String), String> {
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
        return Ok((parent.to_string(), leaf.to_string()));
    }
    if name.is_empty() {
        return Err("give `path`, or `parent` and `name`".into());
    }
    if parent.is_empty() {
        return Err("give the whole target as `path`, or `parent` and `name`; a bare `name` is not enough".into());
    }
    Ok((spelled(parent)?.trim_end_matches('/').to_string(), name.to_string()))
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
    let who = caller_key();
    let path = match (text(args, "path"), &who) {
        ("", Caller::Person) => on_screen(),
        ("", mind) => mind.folder().ok_or(NEEDS_PATH)?,
        (path, _) => spelled(path)?.to_string(),
    };
    mind::may_open(&path)?;
    let for_mind = who.is_mind();
    deferred(move || read_folder(&path, for_mind))
}

/// Take the Mind's cursor to `path` (when it has one) and say what is there.
fn go(key: Option<String>, path: String) -> Result<Value, String> {
    spelled(&path)?;
    mind::open_verdict(&path, &home())?;
    deferred(move || {
        let listing = read_folder(&path, true)?;
        if let Some(key) = key {
            move_cursor(&key, &path);
        }
        Ok(listing)
    })
}

/// A Mind's call to a `files_*` action: `None` for the person (the action goes on as written),
/// and for a Mind the answer, reached without the person's screen, selection or clipboard.
/// `files_list` and `files_stat` are not here, and `files_new_folder` and `files_new_file` are
/// the same action for everyone (`make_target`): none of them looks at the screen for a Mind.
pub(crate) fn intercept(action: &str, args: &Value) -> Option<Result<Value, String>> {
    match caller_key() {
        Caller::Person => None,
        who => Some(act(&who, action, args)),
    }
}

/// [`intercept`] once the caller is known.
pub(crate) fn act(who: &Caller, action: &str, args: &Value) -> Result<Value, String> {
    let key = match who {
        Caller::Named(key) => Some(key.clone()),
        _ => None,
    };
    let here = || who.folder().ok_or(NEEDS_PATH);
    match action {
        "files_go" => go(key, text(args, "path").to_string()),
        "files_enter" => {
            let name = crate::fileops::name(text(args, "name"))?;
            go(key, mind::into(&here()?, name))
        }
        "files_up" => go(key, mind::up_from(&here()?)),
        "files_refresh" => {
            let path = here()?;
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
        "files_rename" => rename(args),
        "files_delete" => delete(args),
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

fn rename(args: &Value) -> Result<Value, String> {
    let (parent, name) = entry_in(args)?;
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

fn delete(args: &Value) -> Result<Value, String> {
    let (parent, name) = entry_in(args)?;
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
        let (label, leaf) = entry_in(args)?;
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
        let guard = make.find("if !who.is_mind() && !explicit").expect("the person-only switch");
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
        let key = &Caller::Named("mind:test-refuse-1033".to_string());
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
    fn an_entry_is_named_by_path_or_by_parent_and_name() {
        let e = |v: Value| entry_in(&v).unwrap();
        assert_eq!(e(json!({ "path": "/home/p/a/b" })), ("/home/p/a".into(), "b".into()));
        assert_eq!(e(json!({ "path": "~/a/" })), ("~".into(), "a".into()));
        assert_eq!(e(json!({ "path": "/x" })), ("/".into(), "x".into()));
        assert_eq!(e(json!({ "parent": "/home/p/a/", "name": "b" })), ("/home/p/a".into(), "b".into()));
        for bad in [json!({}), json!({ "name": "b" }), json!({ "path": "relative/x" }), json!({ "path": "~" }), json!({ "parent": "docs", "name": "b" })] {
            assert!(entry_in(&bad).is_err(), "{bad}");
        }
    }

    /// #1033 review: the person's Allow binds the arguments the card showed. A bare `name` would
    /// be resolved against a cursor the Mind can move after that, so a Mind's delete and rename
    /// are refused without the whole target, whatever the cursor says, and the cursor never
    /// decides what they act on.
    #[test]
    fn a_minds_delete_and_rename_name_the_whole_target() {
        let key = "mind:test-bound-1033";
        let who = Caller::Named(key.to_string());
        for cursor in ["~/scratch", "~/Documents"] {
            move_cursor(key, cursor);
            for (action, args) in [
                ("files_delete", json!({ "name": "notes.txt" })),
                ("files_rename", json!({ "name": "notes.txt", "new_name": "n2.txt" })),
            ] {
                let said = act(&who, action, &args).unwrap_err();
                assert!(said.contains("whole target"), "{action} at {cursor}: {said}");
            }
        }
        // The same call with the target named reaches the path checks instead, and a path the
        // home does not hold is refused by them, not by the missing target.
        let said = act(&who, "files_delete", &json!({ "path": "/etc/passwd" })).unwrap_err();
        assert!(!said.contains("whole target"), "{said}");
    }

    #[test]
    fn a_caller_is_sorted_into_the_person_a_named_mind_or_an_unnamed_one() {
        let mind = |s: &str| Requester::Mind(s.to_string());
        let mind_id = |s: &str| Some(Identity::Mind(s.to_string()));
        assert_eq!(caller_for(&Requester::Person, None), Caller::Person);
        assert_eq!(caller_for(&Requester::Person, mind_id("x")), Caller::Person, "the person never gets a cursor");
        let (a, b) = (caller_for(&mind("x"), mind_id("a")), caller_for(&mind("x"), mind_id("b")));
        assert_eq!((&a, &b), (&Caller::Named("mind:a".into()), &Caller::Named("mind:b".into())));
        assert_ne!(a, b, "two Minds are two callers");
        assert_eq!(caller_for(&mind("an agent"), Some(Identity::Agent("7".into()))), Caller::Named("agent:7".into()));
        for unnamed in [None, Some(Identity::Companion)] {
            assert_eq!(caller_for(&mind("a mind"), unnamed), Caller::Unnamed);
        }
        assert!(Caller::Unnamed.folder().is_none() && Caller::Person.folder().is_none());
        assert!(Caller::Unnamed.is_mind() && !Caller::Person.is_mind());
    }

    /// A Mind the shell cannot name keeps no folder: what needs one is refused with a sentence,
    /// and moving "its" cursor moves nobody's.
    #[test]
    fn an_unnamed_mind_has_no_cursor() {
        for action in ["files_enter", "files_up", "files_refresh"] {
            assert_eq!(act(&Caller::Unnamed, action, &json!({ "name": "a" })).unwrap_err(), NEEDS_PATH, "{action}");
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
        let lib = &lib[..lib.find("#[cfg(test)]").unwrap()];
        assert!(lib.contains("caller_for(&requester_now(), identity_now())"), "caller_key reads who is asking");
        assert!(lib.contains("Caller::Person => None"), "intercept lets the person's call go on as written");
    }
}
