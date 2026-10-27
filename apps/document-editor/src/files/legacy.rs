//! The recovery file the builds before the framework kept, read once and set aside.
//!
//! Those builds wrote the unsaved document into one file, `yantrik/document-editor/draft.json`
//! under the state home. At the first start of this build it is opened again, as it always was,
//! and becomes a draft of its own in the framework's lane. Only once that draft is on disk is the
//! old file renamed to `draft.json.imported`: it is backed up, never deleted, and never read
//! again. A file that cannot be read is left exactly where it is, and the notice says so.

use std::io::Read as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::limits::{validate, MAX_BYTES};

/// The old file's name, in the old state directory.
pub const NAME: &str = "draft.json";
/// What it is renamed to once its draft is the lane's.
pub const ARCHIVED: &str = "draft.json.imported";
/// The old state directory's name, beside this app's own.
pub const OLD_DIR: &str = "document-editor";

/// The most of it that is read: what the old build could legitimately write, so a real one is
/// never refused, and an untrusted file stays bounded.
const LIMIT: usize = MAX_BYTES * 4 + 65536;

/// The document as the old build wrote it. Only these fields are read; the rest are ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct LegacyDoc {
    pub path: Option<PathBuf>,
    pub text: String,
    pub baseline: String,
}

/// What the old file held.
#[derive(Debug)]
pub enum Legacy {
    /// There is none.
    Absent,
    /// Its document, valid.
    Found(LegacyDoc),
    /// It could not be used, and why; it is left as it is.
    Unreadable(String),
}

/// The old state directory, beside this app's: `<state home>/yantrik/document-editor`.
pub fn old_dir(state_dir: &Path) -> PathBuf {
    state_dir.with_file_name(OLD_DIR)
}

/// Reads the old file at `<dir>/draft.json`, bounded by `limit` (the tests use a small one).
pub fn read(dir: &Path, limit: Option<usize>) -> Legacy {
    let path = dir.join(NAME);
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
        return Legacy::Unreadable(format!("The recovery file could not be read: {e}. It has been left alone."));
    }
    if bytes.len() > limit {
        return Legacy::Unreadable("The recovery file is larger than yDoc ever writes; it was not read.".into());
    }
    let doc: LegacyDoc = match serde_json::from_slice(&bytes) {
        Ok(doc) => doc,
        Err(e) => {
            return Legacy::Unreadable(format!("The recovery file could not be read: {e}. It has been left alone."))
        }
    };
    if let Err(e) = validate(&doc.text).and_then(|()| validate(&doc.baseline)) {
        return Legacy::Unreadable(format!("The recovery file holds something yDoc will not open: {e}"));
    }
    if doc.path.as_ref().is_some_and(|p| !p.is_absolute()) {
        return Legacy::Unreadable("The recovery file names a file by a relative path; it was not opened.".into());
    }
    Legacy::Found(doc)
}

/// Sets the old file aside as `draft.json.imported`, never over a file already there.
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
