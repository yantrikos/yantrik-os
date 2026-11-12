//! Deciding, for one file, what Apply keeps of the person's own config before writing over it.
//!
//! The rule (the shared cases in harnesses/tests/fixtures/keep_live_cases.json are the rule; the
//! Python side, harnesses/lib/install/hermes_model.py, runs the same table): nothing of the person's is
//! overwritten without a copy. A live file is this code's own, and needs none, only if it equals the
//! backup or a content the in-progress record says this code wrote.

use std::path::{Path, PathBuf};

use super::progress::{sha, FileProgress};
use super::{backup_path, replaced, Marker, Touched};
use crate::private_file::{self, Publish};

/// What was decided for one file.
pub(super) struct Kept {
    pub backup: Option<PathBuf>,
    /// The backup was already there and neither a marker nor an unfinished apply of this code names it.
    pub adopted: bool,
    pub replaced: Vec<PathBuf>,
}

pub(super) fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

fn read_live(path: &Path) -> Result<Option<Vec<u8>>, String> {
    if !exists(path) {
        return Ok(None);
    }
    private_file::check_target(path)?;
    std::fs::read(path).map(Some).map_err(|e| format!("could not keep a copy of {}: {e}", path.display()))
}

/// The person's own file, kept once beside it. A copy already there is never replaced, so Revert can
/// return what they had. Every other state is read from what is on disk:
///
/// - a marker names the file and the live file is what was written: nothing to do;
/// - a marker names it and the live file has been edited since: it is copied first (#1205);
/// - the marker's backup is gone (a Revert that stopped partway): the live file is the person's, and
///   becomes the new backup;
/// - a backup no marker names: adopted, unless the record says this code's own unfinished apply made
///   it; a live file that is neither the backup nor this code's own is copied to `.yantrik-replaced-*`.
///
/// Nothing is written before the copy exists, so a stop at any point reads the same way next time.
pub(super) fn keep_original(path: &Path, known: &FileProgress, earlier: Option<&Marker>) -> Result<Kept, String> {
    let live = read_live(path)?;
    let mut carried = known.replaced.clone();
    if let Some(t) = earlier.and_then(|m| m.files.iter().find(|t| t.path == path)) {
        let backup_present = t.backup.as_ref().is_none_or(|b| exists(b));
        if backup_present {
            let edited = live.as_ref().is_some_and(|l| is_edited(t, known, l));
            let mut replaced = t.replaced.clone();
            if edited {
                replaced.push(replaced::keep_live(path, live.as_deref().unwrap_or_default())?);
            }
            return Ok(Kept { backup: t.backup.clone(), adopted: t.adopted, replaced });
        }
        carried = t.replaced.clone();
    }
    let b = backup_path(path);
    if exists(&b) {
        private_file::check_target(&b)?;
        let backup = std::fs::read(&b).map_err(|e| format!("could not read {}: {e}", b.display()))?;
        if let Some(l) = &live {
            if *l != backup && !known.is_own(l) {
                carried.push(replaced::keep_live(path, l)?);
            }
        }
        return Ok(Kept { backup: Some(b), adopted: !known.backup_ours, replaced: carried });
    }
    match live {
        // Nothing before this, or only what this code wrote (the backup was deleted): nothing to keep.
        None => Ok(Kept { backup: None, adopted: false, replaced: carried }),
        Some(l) if known.is_own(&l) => Ok(Kept { backup: None, adopted: false, replaced: carried }),
        Some(l) => {
            private_file::write(&b, &l, Publish::CreateOnly, || Ok(()))?;
            Ok(Kept { backup: Some(b), adopted: false, replaced: carried })
        }
    }
}

/// The live file is not what this code wrote: not the marker's hash, not any recorded content.
fn is_edited(t: &Touched, known: &FileProgress, live: &[u8]) -> bool {
    match &t.written {
        Some(h) => sha(live) != *h && !known.is_own(live),
        None => false,
    }
}
