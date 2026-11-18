//! What an apply is about to write, recorded before the first live write (#1205 round 2).
//!
//! A gateway plan mints a new token each time, so a retry after a failure between the live write and
//! the marker writes different bytes than the attempt that failed. Without a record, the retry would
//! read this code's own half-done file as the person's config and copy it. So before anything is
//! touched, each file gets the sha256 of every content this code has written to it (this attempt's
//! and earlier unfinished ones) and whether the backup beside it is this code's own. A live file that
//! matches is this code's; one that does not is the person's. The record is removed once the marker
//! is written. It holds hashes and paths, never a key.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::private_file::{self, Publish};

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct Progress {
    pub files: Vec<FileProgress>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(super) struct FileProgress {
    pub path: PathBuf,
    /// sha256 of every content this code wrote, or was about to write, to `path`.
    pub own: Vec<String>,
    /// The `.before-yantrik` beside it was made by this code's unfinished apply, not found.
    pub backup_ours: bool,
    /// Copies of the person's own file this unfinished apply made (the retry names them).
    #[serde(default)]
    pub replaced: Vec<PathBuf>,
}

impl FileProgress {
    /// The copies this record names that `named` (a marker's list) does not: made by an apply that
    /// failed after it, so no marker knows them yet.
    pub fn not_in(&self, named: &[PathBuf]) -> Vec<PathBuf> {
        self.replaced.iter().filter(|p| !named.contains(p)).cloned().collect()
    }

    pub fn is_own(&self, bytes: &[u8]) -> bool {
        self.own.contains(&sha(bytes))
    }
}

pub(super) fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn record_path(home: &Path, harness: &str) -> PathBuf {
    home.join(".config/yantrik/handoff").join(format!("{harness}.applying.json"))
}

pub(super) fn load(home: &Path, harness: &str) -> Progress {
    std::fs::read_to_string(record_path(home, harness)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub(super) fn save(home: &Path, harness: &str, p: &Progress) -> Result<(), String> {
    let text = serde_json::to_string(p).map_err(|e| e.to_string())?;
    private_file::write(&record_path(home, harness), text.as_bytes(), Publish::Replace, || Ok(()))
}

pub(super) fn clear(home: &Path, harness: &str) {
    let _ = std::fs::remove_file(record_path(home, harness));
}

impl Progress {
    pub fn set(&mut self, f: FileProgress) {
        match self.files.iter_mut().find(|x| x.path == f.path) {
            Some(x) => *x = f,
            None => self.files.push(f),
        }
    }

    pub fn file(&self, path: &Path) -> FileProgress {
        self.files.iter().find(|f| f.path == path).cloned().unwrap_or_else(|| FileProgress { path: path.to_path_buf(), ..Default::default() })
    }
}
