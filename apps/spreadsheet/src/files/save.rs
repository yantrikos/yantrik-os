//! Writing the sheet to a CSV file: the framework's save, with ySheets' sentences and its two
//! rules of its own: only `.csv`, and `save` never replaces.

use std::path::Path;

use yantrik_app::documents::disk::Disk;
use yantrik_app::documents::files::agent;
use yantrik_app::documents::files::save as framework;
pub use yantrik_app::documents::files::save::{SaveFailed, SaveJob, Wrote};
use yantrik_app::documents::files::FileRules;
use yantrik_app::documents::SaveError;

use super::rules::SheetRules;

/// A save that did not land, and whether it was because a file is already there and nobody said to
/// replace it (the window's typed-path dialog asks about that, once).
#[derive(Debug, Clone)]
pub struct Refusal {
    pub failed: SaveFailed,
    pub there: bool,
}

fn nothing(sentence: impl Into<String>, there: bool) -> Box<Refusal> {
    Box::new(Refusal { failed: SaveFailed { error: SaveError::new(sentence), kept: None }, there })
}

/// A save that landed, and whether it took the place of a file that was there.
#[derive(Debug, Clone)]
pub struct Saved {
    pub wrote: Wrote,
    pub replaced: bool,
}

/// Whether `path` ends in `.csv`, which is all this sheet writes.
pub fn is_csv(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() == Some("csv")
}

/// Saves the sheet. A file already at the target is replaced only if `job.overwrite` says so or it
/// is the sheet's own; with `must_be_new` (the control surface's `save`) nothing is replaced, the
/// sheet's own file included, and the refusal names `save_over`.
pub fn save_csv(job: SaveJob, must_be_new: bool, disk: &dyn Disk) -> Result<Saved, Box<Refusal>> {
    // An agent is told where it may not write before it is told anything else about the name.
    agent::may_write(job.agent, &job.target).map_err(|why| nothing(why, false))?;
    if !is_csv(&job.target) {
        return Err(nothing(
            format!("{} does not end in .csv; this sheet saves CSV files only", job.target.display()),
            false,
        ));
    }
    let own = job.own.as_deref() == Some(job.target.as_path());
    let exists = std::fs::symlink_metadata(&job.target).is_ok();
    if exists && (must_be_new || (!job.overwrite && !own)) {
        // A file made at the name between this look and the write is refused by the write itself.
        return Err(nothing(SheetRules.already_there(&job.target), true));
    }
    framework::save(&SheetRules, &job, disk)
        .map(|wrote| Saved { wrote, replaced: exists })
        .map_err(|failed| Box::new(Refusal { failed: *failed, there: false }))
}
