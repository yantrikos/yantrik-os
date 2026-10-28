//! The recovery file the builds before the framework kept, read once and set aside.
//!
//! Those builds wrote the unsaved deck into one file, `recovery.json` in this app's state folder.
//! At the first start of this build it is opened again, as it always was, and becomes a draft of
//! its own in the framework's lane. Only once that draft is on disk is the old file renamed to
//! `recovery.json.imported`: it is backed up, never deleted, and never read again. A file that
//! cannot be read is left exactly where it is, and the notice says so.

use std::io::Read as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::deck::{bounded, check_version, Content, MAX_FILE_BYTES};

/// The old file's name, in this app's state folder.
pub const NAME: &str = "recovery.json";
/// What it is renamed to once its draft is the lane's.
pub const ARCHIVED: &str = "recovery.json.imported";

/// The deck as the old build wrote it. Only these fields are read.
#[derive(Debug, Clone, Deserialize)]
pub struct LegacyDeck {
    pub path: Option<PathBuf>,
    pub content: Content,
}

/// What the old file held.
#[derive(Debug)]
pub enum Legacy {
    /// There is none.
    Absent,
    /// Its deck, valid.
    Found(LegacyDeck),
    /// It could not be used, and why; it is left as it is.
    Unreadable(String),
}

/// Reads the old file at `<dir>/recovery.json`, bounded by the size of a deck.
pub fn read(dir: &Path) -> Legacy {
    let path = dir.join(NAME);
    if std::fs::symlink_metadata(&path).is_err() {
        return Legacy::Absent;
    }
    let unreadable = |why: String| Legacy::Unreadable(format!("The recovery file at {} {why}. It has been left alone.", path.display()));
    let mut bytes = Vec::new();
    let read = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
        .and_then(|f| f.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes));
    if let Err(e) = read {
        return unreadable(format!("could not be read ({e})"));
    }
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return unreadable("is larger than a deck ever is, so it was not read".into());
    }
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => return unreadable(format!("could not be read ({})", crate::deck::LoadError::NotJson(e.to_string()))),
    };
    if let Err(e) = check_version(&value) {
        return unreadable(format!("could not be read ({e})"));
    }
    let deck: LegacyDeck = match serde_json::from_value(value) {
        Ok(d) => d,
        Err(e) => return unreadable(format!("could not be read ({})", crate::deck::LoadError::Malformed(e.to_string()))),
    };
    if let Err(e) = bounded(deck.content.slides.len()) {
        return unreadable(format!("could not be read ({e})"));
    }
    if deck.path.as_ref().is_some_and(|p| !p.is_absolute()) {
        return unreadable("names its deck by a relative path, so it was not opened".into());
    }
    Legacy::Found(deck)
}

/// Sets the old file aside as `recovery.json.imported`, never over a file already there.
pub fn archive(dir: &Path) -> Result<(), String> {
    let from = dir.join(NAME);
    let mut to = dir.join(ARCHIVED);
    let mut n = 1;
    while std::fs::symlink_metadata(&to).is_ok() {
        n += 1;
        to = dir.join(format!("{ARCHIVED}.{n}"));
    }
    // A second name, then the first removed: a file made at `to` meanwhile is never replaced.
    std::fs::hard_link(&from, &to).map_err(|e| e.to_string())?;
    std::fs::remove_file(&from).map_err(|e| e.to_string())
}
