//! Where a mind may take the Files screen, and what it may do with the folder it finds there.
//!
//! The Files screen runs as the person and shows whatever folder it was left in. A mind reading
//! it back through `describe shell`, or acting on it through `files_*`, sees and changes what the
//! file tools themselves would never reach: /etc, another account's home, ~/.ssh (#443). So a
//! mind may open only folders in the person's home outside its protected places, the same rule
//! as `files_stat` (yantrik_ipc_contracts::home_paths), links followed to where they really lead;
//! and every other action first checks the folder on screen, because the person may have left
//! Files anywhere. The person, clicking or typing `yos` in their own terminal, goes wherever they
//! can see.

use std::path::Path;

use yantrik_ipc_contracts::home_paths;

use crate::mind_view::{requester_now, Requester};

/// What `describe shell` says in place of a folder a mind may not see.
pub const HIDDEN: &str = "the folder is outside what a mind may see";

fn a_mind_is_calling() -> bool {
    requester_now() != Requester::Person
}

fn home() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

/// Refuse a mind taking Files to `path`, a folder given as the Files screen spells it.
pub fn may_open(path: &str) -> Result<(), String> {
    if !a_mind_is_calling() {
        return Ok(());
    }
    open_verdict(path, &home())
}

/// Refuse a mind acting on the folder Files is showing, `label` as the screen shows it (`~`,
/// `~/Documents`, `/etc`, or `Trash`, which is no folder and lists names from anywhere).
pub fn here(label: &str) -> Result<(), String> {
    if !a_mind_is_calling() {
        return Ok(());
    }
    here_verdict(label, &home())
}

/// Refuse a mind opening the entry `name` in the folder Files is showing: the folder must be one
/// it may see, and the entry itself there and not protected, links followed - a link in a
/// home folder to /etc/shadow is an entry in the home.
pub fn may_open_entry(label: &str, name: &str) -> Result<(), String> {
    if !a_mind_is_calling() {
        return Ok(());
    }
    entry_verdict(label, name, &home())
}

/// Refuse a mind making `name` in the folder Files is showing - a new folder, a new file, or a
/// rename's new name - where it would be a protected place. Each step of "make a folder x in
/// ~/.local/share, save a .desktop into it, rename x to applications" was allowed on its own.
pub fn may_make(label: &str, name: &str) -> Result<(), String> {
    if !a_mind_is_calling() {
        return Ok(());
    }
    make_verdict(label, name, &home())
}

/// Refuse a mind acting on these entries of the folder on screen - renaming, trashing,
/// selecting, copying or cutting them - unless each is one it may open: `files_rename .ssh keys`
/// would otherwise take the keys out from under their protection.
pub fn may_touch(label: &str, names: &[String]) -> Result<(), String> {
    if !a_mind_is_calling() {
        return Ok(());
    }
    let home = home();
    names.iter().try_for_each(|name| entry_verdict(label, name, &home))
}

/// Refuse a mind pasting the person's clipboard into the folder on screen unless every source
/// may be read and everything it would create may be made (control_files_paste.rs).
pub fn may_paste(label: &str) -> Result<(), String> {
    if !a_mind_is_calling() {
        return Ok(());
    }
    paste_now_verdict(crate::wire::files::clipboard_now(), label, &home())
}

/// [`may_paste`] with the clipboard as read and `home` given. A clipboard that could not be read
/// refuses: "nothing to check" is not what an unreadable one means.
pub fn paste_now_verdict(
    clipboard: Result<Option<(Vec<std::path::PathBuf>, bool)>, String>,
    label: &str,
    home: &Path,
) -> Result<(), String> {
    match clipboard? {
        Some((sources, _cut)) => crate::control_files_paste::paste_verdict(&sources, label, home),
        None => Ok(()),
    }
}

/// Refuse a mind renaming the entry `name` to `new`: the entry must be one it may open, the new
/// name one it may make, and - for a folder - everything below it must be able to land under
/// the new name. `cfg` holding `autostart/evil.desktop`, renamed `.config`, would otherwise
/// create ~/.config/autostart with only the top name checked.
pub fn may_rename(label: &str, name: &str, new: &str) -> Result<(), String> {
    if !a_mind_is_calling() {
        return Ok(());
    }
    rename_verdict(label, name, new, &home())
}

/// [`may_rename`] with `home` given.
pub fn rename_verdict(label: &str, name: &str, new: &str, home: &Path) -> Result<(), String> {
    entry_verdict(label, name, home)?;
    make_verdict(label, new, home)?;
    let folder = home_paths::expand(label, home).ok_or_else(|| format!("{label} is not a folder"))?;
    let real = folder.canonicalize().map_err(|e| format!("the folder on screen cannot be read: {e}"))?;
    home_paths::may_land(&folder.join(name), &real.join(new), home)
}

/// Undoing a trash puts things back where they were deleted from, which may be anywhere the
/// person deleted from; which items it restores is decided by the last trash, not by the caller.
pub fn may_undo_trash() -> Result<(), String> {
    if a_mind_is_calling() {
        return Err("undoing a trash restores the person's files wherever they were; a mind does not".to_string());
    }
    Ok(())
}

/// [`may_make`] with `home` given.
pub fn make_verdict(label: &str, name: &str, home: &Path) -> Result<(), String> {
    here_verdict(label, home)?;
    if name.contains('/') || name == ".." || name == "." {
        return Err(format!("`{name}` is not a name for something in this folder"));
    }
    home_paths::may_create(&into(label, name), home)
}

/// Trash lists what was deleted from anywhere, under the names it had, so it is no folder a
/// mind looks into; `describe shell` hides it too.
pub fn may_show_trash() -> Result<(), String> {
    if a_mind_is_calling() {
        return Err("Trash holds what was deleted from anywhere; a mind does not open it".to_string());
    }
    Ok(())
}

/// Whether what Files shows is hidden from this caller, and why.
///
/// Runs on the UI thread, inside `describe shell`: a describe answers from the live view-model
/// and cannot be handed off the thread the way an action's `answer_later` can. The cost is
/// `requester_now` (a short /proc walk when nothing cheaper decides) and, for a mind, a `stat`
/// of the folder on screen, which was just listed and so is in the kernel's cache.
pub fn hidden_here(label: &str) -> Option<&'static str> {
    (a_mind_is_calling() && here_verdict(label, &home()).is_err()).then_some(HIDDEN)
}

/// The Files part of `describe shell` with everything about the folder taken out: its name, its
/// entries and recent row, the selection, the preview, and the notice and operation lines, which
/// name the files they are about. Said to be hidden, so an empty listing is not read as empty.
pub fn hide_folder(mut files: serde_json::Value, why: &str) -> serde_json::Value {
    use serde_json::{json, Value};
    let Some(map) = files.as_object_mut() else { return files };
    for (key, blank) in [
        ("path", Value::Null),
        ("entries", json!([])),
        ("recent", json!([])),
        ("shown", json!(0)),
        ("total", json!(0)),
        ("selected", Value::Null),
        ("selection_count", json!(0)),
        ("preview_name", json!("")),
        ("notice", json!("")),
        ("operation", json!("")),
    ] {
        map.insert(key.to_string(), blank);
    }
    map.insert("hidden".to_string(), json!(why));
    files
}

/// The folder `files_up` loads from `label`: the same function the callback uses, so the path
/// checked is the path loaded. (`Path::new("~").parent()` is "", not the folder above the home.)
pub fn up_from(label: &str) -> String {
    crate::filebrowser::parent_path(label)
}

/// The folder `files_enter` loads for `name` in `label`, as the callback computes it.
pub fn into(label: &str, name: &str) -> String {
    crate::filebrowser::child_path(label, name)
}

/// Whether a mind may open `path` in Files, with `home` as the person's home.
pub fn open_verdict(path: &str, home: &Path) -> Result<(), String> {
    let answer = home_paths::stat(path, home);
    match (&answer["exists"], answer["kind"].as_str()) {
        (serde_json::Value::Bool(true), Some("directory")) => Ok(()),
        (serde_json::Value::Bool(true), _) => Err(format!("{path} is not a folder")),
        (serde_json::Value::Bool(false), _) => Err(format!("there is no folder at {path}")),
        _ => Err(format!(
            "a mind's Files stays in the person's home, outside its protected places; {path} is {}",
            answer["reason"].as_str().unwrap_or("not one it may open")
        )),
    }
}

/// [`here`] with `home` given. The refusal does not name the folder: which folder the person
/// has open is itself something the mind is not shown.
pub fn here_verdict(label: &str, home: &Path) -> Result<(), String> {
    open_verdict(label, home).map_err(|_| {
        "Files is showing a folder outside what a mind may see; `files_go` to a folder in the \
         person's home first"
            .to_string()
    })
}

/// [`may_open_entry`] with `home` given.
pub fn entry_verdict(label: &str, name: &str, home: &Path) -> Result<(), String> {
    here_verdict(label, home)?;
    let path = into(label, name);
    let answer = home_paths::stat(&path, home);
    match &answer["exists"] {
        serde_json::Value::Bool(true) => Ok(()),
        serde_json::Value::Bool(false) => Err(format!("nothing is at {path}")),
        _ => Err(format!(
            "a mind opens only what is in the person's home, outside its protected places; {path} is {}",
            answer["reason"].as_str().unwrap_or("not one it may open")
        )),
    }
}

#[cfg(test)]
#[path = "control_files_mind_tests.rs"]
mod tests;
