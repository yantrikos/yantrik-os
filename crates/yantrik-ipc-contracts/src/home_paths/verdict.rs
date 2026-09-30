//! What an app asked by an agent may read or write, decided from [`stat`]'s answer.
//!
//! An app runs as the person, so anything the person can read it can read, and anything it reads
//! into its window an agent can read back through `describe`. The person choosing a file in the
//! window is not asked this; an agent naming one over the control surface is.

use std::path::Path;

use serde_json::Value;

use super::{expand, stat, where_programs_look, HIDDEN_RULE};

/// Whether an agent may have the file at `asked` read: in the home, outside its protected
/// places, there, and a file. `Err` says which of those it is not.
pub fn may_read_file(asked: &str, home: &Path) -> Result<(), String> {
    let answer = stat(asked, home);
    match (&answer["exists"], answer["kind"].as_str()) {
        (Value::Bool(true), Some("file")) => Ok(()),
        (Value::Bool(true), _) => Err(format!("{asked} is not a file")),
        (Value::Bool(false), _) => Err(format!("there is no file at {asked}")),
        _ => Err(refusal(asked, &answer)),
    }
}

/// Whether an agent may have a file written at `asked`: in the home and outside its protected
/// places, into a folder that is already there, and not through a link.
///
/// A file already at the path may be replaced - an agent asked to save over the draft it made is
/// the ordinary case - because what could run as the person when replaced is on the protected
/// list or in a hidden place (writes.rs), both refused before this is reached. The last part of
/// the path must not be a link: the write would land wherever the link points, and a link an
/// agent could place is exactly how "a file in the home" becomes somewhere else.
pub fn may_write_file(asked: &str, home: &Path) -> Result<(), String> {
    let answer = stat(asked, home);
    if !answer["exists"].is_boolean() {
        return Err(refusal(asked, &answer));
    }
    not_hidden(asked, home)?;
    let is_link = || {
        expand(asked.trim(), home)
            .and_then(|p| p.symlink_metadata().ok())
            .is_some_and(|m| m.file_type().is_symlink())
    };
    match (&answer["exists"], answer["kind"].as_str()) {
        (Value::Bool(true), Some("file")) if is_link() => {
            Err(format!("{asked} is a link; an agent writes a file, not through a link to one"))
        }
        (Value::Bool(true), Some("file")) => Ok(()),
        (Value::Bool(true), _) => Err(format!("{asked} is not a file")),
        (Value::Bool(false), _) => folder_is_there(asked, home),
        _ => Err(refusal(asked, &answer)),
    }
}

/// Whether an agent may have something made or moved to `asked`: a new folder, a new file, a
/// rename's new name, a pasted copy. Only where `stat` answers true or false - in the home,
/// outside its protected places, links followed and the part not there yet included - so a
/// folder named `applications` made in ~/.local/share, or renamed to that, is refused. Whether
/// a name is already taken is the operation's own business. Nowhere hidden, either (writes.rs):
/// a folder `.config` made in the home is where programs will look.
pub fn may_create(asked: &str, home: &Path) -> Result<(), String> {
    let answer = stat(asked, home);
    match &answer["exists"] {
        Value::Bool(_) => not_hidden(asked, home),
        _ => Err(refusal(asked, &answer)),
    }
}

/// Refuse a write into a place programs read their settings or startup from.
fn not_hidden(asked: &str, home: &Path) -> Result<(), String> {
    match expand(asked.trim(), home) {
        Some(path) if where_programs_look(&path, home) => Err(format!("{HIDDEN_RULE}; {asked} is hidden_place")),
        _ => Ok(()),
    }
}

/// A new file needs its folder already there: nothing an agent writes creates folders on the way.
fn folder_is_there(asked: &str, home: &Path) -> Result<(), String> {
    let Some(parent) = expand(asked.trim(), home).and_then(|p| p.parent().map(Path::to_path_buf)) else {
        return Err(format!("{asked} has no folder to go in"));
    };
    let folder = parent.to_string_lossy();
    let answer = stat(&folder, home);
    match (&answer["exists"], answer["kind"].as_str()) {
        (Value::Bool(true), Some("directory")) => Ok(()),
        (Value::Bool(true), _) => Err(format!("{folder} is not a folder")),
        (Value::Bool(false), _) => Err(format!("the folder {folder} does not exist")),
        _ => Err(refusal(&folder, &answer)),
    }
}

/// The refusal for a path `stat` would not answer for. Ends with the reason alone, so the words
/// in front, which name both rules, cannot be mistaken for it.
fn refusal(asked: &str, answer: &Value) -> String {
    format!(
        "an agent reads and writes files only in the person's home, outside its protected places; \
         {asked} is {}",
        answer["reason"].as_str().unwrap_or("not a path it may use")
    )
}
