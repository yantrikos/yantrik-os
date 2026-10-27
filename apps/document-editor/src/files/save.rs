//! Writing the document to a file: the framework's save, with yDoc's sentences.

use yantrik_app::documents::disk::Disk;

pub use yantrik_app::documents::files::save::{SaveFailed, SaveJob, Wrote};

use super::rules::DocRules;

/// Saves the document.
pub fn save(job: &SaveJob, disk: &dyn Disk) -> Result<Wrote, Box<SaveFailed>> {
    yantrik_app::documents::files::save::save(&DocRules, job, disk)
}
