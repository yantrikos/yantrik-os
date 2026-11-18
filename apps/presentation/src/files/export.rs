//! Writing a copy of the deck: Markdown or an outline.
//!
//! A name the caller gave is the shared save's to judge (a file there is refused). A name this app
//! chose, beside the deck with its stem, is replaced only when the file there is exactly what this
//! deck's last export of that name wrote (its path and fingerprint are remembered) and still begins
//! with the export's header. Anything else is somebody's own file, or another deck's export, and is
//! never written over: the window counts up (`talk-2.md`), as a deck without a name does, and an
//! agent's call is refused.

use std::path::{Path, PathBuf};

use yantrik_app::documents::disk::Disk;
use yantrik_app::documents::files::read::read_bytes;
use yantrik_app::documents::files::save as framework;
use yantrik_app::documents::{Fingerprint, SaveError};

use super::rules::ExportRules;
use super::save::{SaveFailed, SaveJob, Wrote};

/// The most of an earlier export that is read to recognise it.
const LIMIT: u64 = 16 * 1024 * 1024;

/// A name this app chooses for an export, and what it may replace.
#[derive(Debug, Clone)]
pub struct DefaultName {
    /// The folder beside the deck (or the default folder), made first if it is not there.
    pub dir: PathBuf,
    /// The deck's file stem, and the export's extension (`md`, `outline.md`).
    pub stem: String,
    pub extension: String,
    /// What an export of this deck begins with.
    pub header: String,
    /// The path and fingerprint of each default export this deck has made.
    pub remembered: Vec<(PathBuf, Fingerprint)>,
    /// A taken name is counted past (the window); otherwise it is refused (an agent).
    pub count_up: bool,
}

fn refused(sentence: impl Into<String>) -> Box<SaveFailed> {
    Box::new(SaveFailed { error: SaveError::new(sentence), kept: None })
}

impl DefaultName {
    /// Whether the file at `path` is an export of this deck that nothing has changed since.
    fn replaceable(&self, path: &Path) -> bool {
        let Some(bytes) = read_bytes(path, LIMIT) else { return false };
        let now = Fingerprint::of_bytes(&bytes);
        bytes.starts_with(self.header.as_bytes()) && self.remembered.iter().any(|(p, f)| p == path && *f == now)
    }

    /// The path to write: the first free name, or the first one holding this deck's own earlier
    /// export. Reads the disk, so it runs in the job.
    fn choose(&self) -> Result<PathBuf, Box<SaveFailed>> {
        for n in 1..=999 {
            let name = if n == 1 {
                format!("{}.{}", self.stem, self.extension)
            } else {
                format!("{}-{n}.{}", self.stem, self.extension)
            };
            let candidate = self.dir.join(name);
            match std::fs::symlink_metadata(&candidate) {
                Err(_) => return Ok(candidate),
                Ok(meta) if meta.is_file() && self.replaceable(&candidate) => return Ok(candidate),
                Ok(_) if self.count_up => continue,
                Ok(_) => {
                    return Err(refused(format!(
                        "{} is already there and is not an earlier export of this deck, so it was not replaced. \
                         Give `export_markdown` a path of your own.",
                        candidate.display()
                    )))
                }
            }
        }
        Err(refused(format!("{} already holds 999 exports of this deck; give one a path of its own", self.dir.display())))
    }
}

/// Writes a copy. With `default`, the name is chosen here, in the job; without, `job.target` is
/// the caller's and the shared save refuses a file already there.
pub fn export_copy(
    mut job: SaveJob,
    default: Option<&DefaultName>,
    disk: &dyn Disk,
) -> Result<Wrote, Box<SaveFailed>> {
    if let Some(default) = default {
        std::fs::create_dir_all(&default.dir)
            .map_err(|e| refused(format!("could not make {}: {e}", default.dir.display())))?;
        job.target = default.choose()?;
        // Chosen above as free or as this deck's own unchanged export, so replacing is as meant.
        job.overwrite = true;
    }
    framework::save(&ExportRules, &job, disk)
}
