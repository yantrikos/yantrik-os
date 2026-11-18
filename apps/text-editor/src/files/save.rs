//! Writing a tab to a file: the framework's save, with the editor's sentences.

use yantrik_app::documents::disk::Disk;

pub use super::rules::CHANGED_ON_DISK;
pub use yantrik_app::documents::files::save::{SaveFailed, SaveJob, Wrote};

use super::rules::TextRules;

/// Saves a tab.
pub fn save(job: &SaveJob, disk: &dyn Disk) -> Result<Wrote, Box<SaveFailed>> {
    yantrik_app::documents::files::save::save(&TextRules, job, disk)
}
