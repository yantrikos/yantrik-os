//! The recovery file the builds before the framework kept, read once and set aside.
//!
//! Those builds wrote every unsaved tab into one file, `<state>/drafts.json`. At the first start
//! of this build its tabs are opened again, as they always were, and each becomes a draft of its
//! own in the framework's lane. Only once every one of those drafts is on disk is the old file
//! renamed to `drafts.json.imported`: it is backed up, never deleted, and never read again. A
//! file that cannot be read is left exactly where it is, and the notice says so.

use std::io::Read as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::text::budget::{MAX_OPEN_BYTES, MAX_TABS};
use crate::text::limits::validate;

/// The old file's name, in the state directory.
pub const NAME: &str = "drafts.json";
/// What it is renamed to once its drafts are the lane's.
pub const ARCHIVED: &str = "drafts.json.imported";

/// The most of it that is read: what the old build could legitimately write (its own bound), so a
/// real one is never refused, and an untrusted file stays bounded. About 258 MiB.
const LIMIT: usize = 4 * MAX_OPEN_BYTES + MAX_TABS * 16 * 1024 + 65536;

/// One tab as the old build wrote it. Who opened it was never written.
#[derive(Debug, Clone, Deserialize)]
pub struct LegacyTab {
    pub path: Option<PathBuf>,
    pub text: String,
    pub baseline: String,
}

/// What the old file held.
#[derive(Debug)]
pub enum Legacy {
    /// There is none.
    Absent,
    /// Its tabs, each a valid document.
    Tabs(Vec<LegacyTab>),
    /// It could not be used, and why; it is left as it is.
    Unreadable(String),
}

/// Reads the old file at `<state_dir>/drafts.json`, bounded by `limit` (the tests use a small one).
pub fn read(state_dir: &Path, limit: Option<usize>) -> Legacy {
    let path = state_dir.join(NAME);
    if std::fs::symlink_metadata(&path).is_err() {
        return Legacy::Absent;
    }
    let limit = limit.unwrap_or(LIMIT);
    let mut bytes = Vec::new();
    let read = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
        .and_then(|f| f.take(limit as u64 + 1).read_to_end(&mut bytes));
    if let Err(e) = read {
        return Legacy::Unreadable(format!("Recovery file could not be read: {e}"));
    }
    if bytes.len() > limit {
        return Legacy::Unreadable("Recovery file is larger than the editor ever writes; it was not read.".into());
    }
    let tabs: Vec<LegacyTab> = match serde_json::from_slice(&bytes) {
        Ok(tabs) => tabs,
        Err(e) => return Legacy::Unreadable(format!("Recovery file could not be read: {e}")),
    };
    if tabs.len() > MAX_TABS {
        return Legacy::Unreadable("Recovery contains too many documents.".into());
    }
    let weight: usize = tabs.iter().map(|t| t.text.len().max(t.baseline.len())).sum();
    if weight > MAX_OPEN_BYTES {
        return Legacy::Unreadable("Recovery holds more text than the editor keeps open at once.".into());
    }
    for t in &tabs {
        if let Err(e) = validate(&t.text).and_then(|()| validate(&t.baseline)) {
            return Legacy::Unreadable(e);
        }
    }
    Legacy::Tabs(tabs)
}

/// Sets the old file aside as `drafts.json.imported`, never over a file already there.
pub fn archive(state_dir: &Path) -> Result<(), String> {
    let from = state_dir.join(NAME);
    let mut to = state_dir.join(ARCHIVED);
    let mut n = 1;
    while std::fs::symlink_metadata(&to).is_ok() {
        n += 1;
        to = state_dir.join(format!("{ARCHIVED}.{n}"));
    }
    // A second name, then the first removed: a file made at `to` meanwhile is never replaced.
    std::fs::hard_link(&from, &to).map_err(|e| e.to_string())?;
    std::fs::remove_file(&from).map_err(|e| e.to_string())
}
