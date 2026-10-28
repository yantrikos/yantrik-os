//! Writing a deck, or a copy of it, to a file: the framework's save, with yPresent's sentences,
//! and the one place a deck is given a name when nobody chose one.

use std::path::PathBuf;

use yantrik_app::documents::disk::Disk;
use yantrik_app::documents::files::save as framework;
pub use yantrik_app::documents::files::save::{SaveFailed, SaveJob, Wrote};
use yantrik_app::documents::SaveError;

use super::names::unused_path;
use super::rules::DeckRules;

/// Where a save with no path of its own goes: the deck's title in the default folder `dir`.
#[derive(Debug, Clone)]
pub struct Unnamed {
    pub title: String,
    pub dir: PathBuf,
}

fn refused(sentence: impl Into<String>) -> Box<SaveFailed> {
    Box::new(SaveFailed { error: SaveError::new(sentence), kept: None })
}

/// The target of a save that has none: the default folder made if it is not there, and a name in
/// it nothing is using. The folder is this app's own fixed place, so making it is not a path an
/// agent chose, and the agent rule still judges the file inside it.
fn choose(unnamed: &Unnamed) -> Result<PathBuf, Box<SaveFailed>> {
    let dir = &unnamed.dir;
    std::fs::create_dir_all(dir).map_err(|e| refused(format!("could not make {}: {e}", dir.display())))?;
    unused_path(dir, &unnamed.title).map_err(refused)
}

/// Saves the deck. With `unnamed`, the name is chosen here, in the job, where the disk may be read.
pub fn save_deck(
    mut job: SaveJob,
    unnamed: Option<Unnamed>,
    disk: &dyn Disk,
) -> Result<Wrote, Box<SaveFailed>> {
    if let Some(unnamed) = &unnamed {
        job.target = choose(unnamed)?;
    }
    framework::save(&DeckRules, &job, disk)
}
