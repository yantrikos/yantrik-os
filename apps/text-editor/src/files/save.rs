//! Writing a tab to a file: whole or not at all, never over a file the tab did not expect to
//! find there, and never through a link.
//!
//! The rules are the ones the editor has always kept, with the framework's disk underneath:
//!
//! - The folder is resolved first (a link to a folder is followed, as it always was); the file
//!   itself must be a regular file with one name, not read-only, and never a link. The exchange
//!   refuses a link again at the instant it replaces, and swaps back anything that is not a
//!   regular file (documents/exchange.rs).
//! - Saving to the tab's own file checks that it still holds what the tab last read or wrote,
//!   byte for byte; anything else is another program's edit, and nothing is written.
//! - A file already at a Save As path is refused unless the caller said `overwrite`, or it is the
//!   tab's own file, carried there by a move (#86).
//! - A new file is published with a link, so a file made meanwhile is never replaced.
//! - What the replace took the place of comes back with it. When that is not what the check
//!   found, another program wrote it in between, and it is kept beside the file (kept.rs).
//!
//! Each failure says how much reached the disk, so nothing is called saved that is not.

use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use yantrik_app::documents::disk::{Disk, DiskError, Displaced, Step};
use yantrik_app::documents::{Durability, Fingerprint, Replaced, SaveError, Witnessed};

use super::kept::{keep_bytes, keep_unread, Kept};
use super::locate::locate;
use super::open::{described, Opened};
use super::read::{read_bytes, read_text};
use crate::agent_rule;
use crate::tab::Inode;
use crate::text::limits::{validate, MAX_BYTES};

/// The exchange reads back at most this much of the file it replaced (the framework's limit).
const READ_BACK: u64 = 64 * 1024 * 1024;

/// What the editor says when a tab's file no longer holds what the tab last read or wrote.
pub const CHANGED_ON_DISK: &str = "File changed on disk. Your draft is intact; use Save As to keep both versions.";

/// What the editor says when a Save As path is taken.
pub const ALREADY_THERE: &str = "That file already exists. Choose another name; nothing was overwritten. Save \
                                 As with overwrite=true replaces it.";

/// One save of one tab, as the job is given it.
#[derive(Debug, Clone)]
pub struct SaveJob {
    pub text: String,
    /// The path asked for: the tab's own file, or a Save As path.
    pub target: PathBuf,
    /// The tab's own file, if it has one.
    pub own: Option<PathBuf>,
    /// What the tab last read from its file or wrote there.
    pub on_disk: Option<String>,
    /// Our bytes a failed save may have left in the tab's file.
    pub unsure: Option<Fingerprint>,
    pub inode: Option<Inode>,
    /// The caller was told a file is at `target` and said to replace it.
    pub overwrite: bool,
    /// An agent's call: the write rule holds (agent_rule.rs).
    pub agent: bool,
    /// The files the other open tabs hold, each with its name as anyone may read it: a save onto
    /// one is refused, so one file is never two tabs (whose drafts could not both come back).
    pub others: Vec<(PathBuf, String)>,
    /// Where another program's version is kept when no name beside the file can be made.
    pub elsewhere: PathBuf,
}

/// A save that landed.
#[derive(Debug, Clone)]
pub struct Wrote {
    /// The file as it now is: where, what it holds, whether an agent is shown it.
    pub file: Opened,
    pub written: Fingerprint,
    pub witnessed: Witnessed,
    /// Another program's version the replace took the place of, kept.
    pub kept: Option<Kept>,
}

/// A save that did not land.
#[derive(Debug, Clone)]
pub struct SaveFailed {
    pub error: SaveError,
    /// Another program's version the replace took the place of before a later step failed.
    pub kept: Option<Kept>,
}

impl SaveFailed {
    fn nothing(sentence: impl Into<String>) -> Box<SaveFailed> {
        Box::new(SaveFailed { error: SaveError::new(sentence), kept: None })
    }

    fn disk(e: &DiskError, written: &Fingerprint, kept: Option<Kept>) -> Box<SaveFailed> {
        let sentence = format!("Save failed: {}. Your draft is still open.", e.error);
        Box::new(SaveFailed { error: SaveError::after_writing(e.step.durability(), written.clone(), sentence), kept })
    }
}

/// What a replace may take the place of without anything being lost.
enum Expect {
    /// The tab's own file, as it last read or wrote it (or our unsure bytes).
    Own,
    /// A file the caller agreed to replace, as the check found it: its bytes' fingerprint when it
    /// was small enough to read back, and where it is.
    Consented { fingerprint: Option<Fingerprint>, stamp: Option<Stamp> },
}

/// What says a file is the one the check saw: where it is, its length and its modification time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: (i64, i64),
}

impl Stamp {
    fn of(meta: &std::fs::Metadata) -> Stamp {
        Stamp { dev: meta.dev(), ino: meta.ino(), len: meta.len(), mtime: (meta.mtime(), meta.mtime_nsec()) }
    }
}

/// Saves a tab.
pub fn save(job: &SaveJob, disk: &dyn Disk) -> Result<Wrote, Box<SaveFailed>> {
    validate(&job.text).map_err(SaveFailed::nothing)?;
    agent_rule::may_write(job.agent, &job.target).map_err(SaveFailed::nothing)?;
    if !job.target.is_absolute() {
        return Err(SaveFailed::nothing("Use an absolute file path."));
    }
    let parent = job.target.parent().ok_or_else(|| SaveFailed::nothing("Choose a file name."))?;
    let name = job.target.file_name().ok_or_else(|| SaveFailed::nothing("Choose a file name."))?;
    let parent = std::fs::canonicalize(parent).map_err(|e| SaveFailed::nothing(format!("Cannot open folder: {e}")))?;
    let path = parent.join(name);
    if let Some((_, tab)) = job.others.iter().find(|(p, _)| *p == path) {
        return Err(SaveFailed::nothing(format!(
            "That file is open in another tab ({tab}); close it there, or choose another name. Nothing was written."
        )));
    }
    let own = job.own.as_deref() == Some(path.as_path());
    let meta = match std::fs::symlink_metadata(&path) {
        // The tab's own file is not where it was. That is almost never a deletion (Files moves
        // with `rename`), so the refusal goes looking for it; `overwrite` is the caller having
        // read that and said to write it here anyway.
        Err(_) if own && !job.overwrite => return Err(SaveFailed::nothing(stranded(job, &path))),
        Err(_) => return create(job, disk, path),
        Ok(meta) => meta,
    };
    let moved_here = !own && its_own_moved_file(job, &path);
    if !own && !job.overwrite && !moved_here {
        return Err(SaveFailed::nothing(ALREADY_THERE));
    }
    if !meta.is_file() || meta.nlink() > 1 {
        return Err(SaveFailed::nothing("This path is linked or is not a regular file. Use Save As."));
    }
    if meta.permissions().readonly() {
        return Err(SaveFailed::nothing("This file is read-only. Use Save As."));
    }
    let expect = if own || moved_here {
        if !holds_ours(job, &path) {
            return Err(Box::new(SaveFailed { error: SaveError::conflict(CHANGED_ON_DISK), kept: None }));
        }
        Expect::Own
    } else {
        let fingerprint = read_bytes(&path, READ_BACK).map(|b| Fingerprint::of_bytes(&b));
        Expect::Consented { fingerprint, stamp: Some(Stamp::of(&meta)) }
    };
    replace(job, disk, path, expect)
}

/// Whether the file at `path` holds what the tab last read or wrote, or our own unsure bytes.
fn holds_ours(job: &SaveJob, path: &Path) -> bool {
    let Some(bytes) = read_bytes(path, MAX_BYTES as u64) else { return false };
    job.on_disk.as_deref().is_some_and(|t| t.as_bytes() == bytes)
        || job.unsure.as_ref() == Some(&Fingerprint::of_bytes(&bytes))
}

/// Writes a file nothing is at yet: a link publishes it, so one made meanwhile is never replaced.
fn create(job: &SaveJob, disk: &dyn Disk, path: PathBuf) -> Result<Wrote, Box<SaveFailed>> {
    let written = Fingerprint::of_bytes(job.text.as_bytes());
    match disk.create_new(&path, job.text.as_bytes()) {
        Ok(()) => Ok(landed(job, path, written, Witnessed::Same, None)),
        Err(e) if e.step == Step::Rename && std::fs::symlink_metadata(&path).is_ok() => {
            Err(SaveFailed::nothing(ALREADY_THERE))
        }
        Err(e) => Err(SaveFailed::disk(&e, &written, None)),
    }
}

/// Replaces the file at `path` by an exchange, keeping whatever another program put there.
fn replace(job: &SaveJob, disk: &dyn Disk, path: PathBuf, expect: Expect) -> Result<Wrote, Box<SaveFailed>> {
    let written = Fingerprint::of_bytes(job.text.as_bytes());
    let exchanged = disk.exchange(&path, job.text.as_bytes()).map_err(|e| SaveFailed::disk(&e, &written, None))?;
    let (witnessed, kept) = judge(job, disk, &path, &expect, exchanged.displaced);
    if let Some(e) = exchanged.unsure {
        // In place, its durability unknown: not called saved, and what it displaced kept.
        let sentence = format!("Save failed: {}. Your draft is still open.", e.error);
        let error = SaveError::after_writing(Durability::Unknown, written, sentence);
        return Err(Box::new(SaveFailed { error, kept }));
    }
    Ok(landed(job, path, written, witnessed, kept))
}

/// What the replace took the place of, judged against what the check found there.
fn judge(job: &SaveJob, disk: &dyn Disk, path: &Path, expect: &Expect, displaced: Displaced) -> (Witnessed, Option<Kept>) {
    match displaced {
        // Gone between the check and the replace: nothing of anyone's was lost.
        Displaced::Nothing => (Witnessed::Same, None),
        Displaced::Unknown => (Witnessed::Unknown, None),
        Displaced::Bytes(bytes) => {
            let fingerprint = Fingerprint::of_bytes(&bytes);
            let expected = bytes == job.text.as_bytes()
                || match expect {
                    Expect::Own => {
                        job.on_disk.as_deref().is_some_and(|t| t.as_bytes() == bytes)
                            || job.unsure.as_ref() == Some(&fingerprint)
                    }
                    Expect::Consented { fingerprint: Some(seen), .. } => *seen == fingerprint,
                    Expect::Consented { fingerprint: None, .. } => false,
                };
            if expected {
                (Witnessed::Same, None)
            } else {
                (Witnessed::Replaced(Replaced::OutsideEdit), Some(keep_bytes(disk, path, &bytes, &job.elsewhere)))
            }
        }
        Displaced::Unread(temp) => {
            // The file the caller agreed to replace, too large to read back: still the same file,
            // the same length and the same modification time, so it goes as agreed.
            let consented = match expect {
                Expect::Consented { stamp: Some(seen), .. } => {
                    std::fs::symlink_metadata(&temp).is_ok_and(|m| Stamp::of(&m) == *seen)
                }
                _ => false,
            };
            if consented && std::fs::remove_file(&temp).is_ok() {
                return (Witnessed::Same, None);
            }
            (Witnessed::Replaced(Replaced::OutsideEdit), Some(keep_unread(path, &temp)))
        }
    }
}

fn landed(job: &SaveJob, path: PathBuf, written: Fingerprint, witnessed: Witnessed, kept: Option<Kept>) -> Wrote {
    Wrote { file: described(path, job.text.clone()), written, witnessed, kept }
}

/// Is the file at `path` this tab's own, carried there by a move?
///
/// True only when the file this tab was opened from is no longer where it was, AND what is at
/// `path` has the same name and exactly the bytes the tab last agreed with. That is this document,
/// moved: writing into it is what a plain Save would have done, so a Save As onto it is not a
/// clobber and is not refused as one.
fn its_own_moved_file(job: &SaveJob, path: &Path) -> bool {
    let (Some(origin), Some(on_disk)) = (&job.own, &job.on_disk) else { return false };
    origin.file_name() == path.file_name()
        && std::fs::symlink_metadata(origin).is_err()
        && read_text(path).ok().as_deref() == Some(on_disk.as_str())
}

/// Why a save cannot write to the file this tab came from, when that file is not there, and
/// where it went if that can be found: a refusal with no next step in it is the same as losing
/// the work.
fn stranded(job: &SaveJob, path: &Path) -> String {
    let found = job.on_disk.as_deref().and_then(|on_disk| locate(path, on_disk, job.inode));
    match found {
        Some(now) => format!(
            "{} is not there any more — the same file looks to be at {} now. Nothing was written and \
             your draft is intact: Save As to {} to write your changes into it.",
            path.display(),
            now.display(),
            now.display()
        ),
        None => format!(
            "{} was moved or deleted, so there is nothing there to write into. Nothing was written and \
             your draft is intact: Save As with overwrite=true to write it here, or Save As to wherever \
             it went.",
            path.display()
        ),
    }
}
