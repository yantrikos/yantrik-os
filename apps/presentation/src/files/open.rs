//! Opening a deck: the framework's opener, held to the format, and the deck most recently
//! worked on. Each function runs in a job.

use std::path::{Path, PathBuf};

use yantrik_app::documents::files::open;
pub use yantrik_app::documents::files::open::{described, Opened};

use super::rules::DeckRules;
use crate::deck::EXTENSION;

/// Opens the deck at `path`. An agent may name only a file in the person's home, outside its
/// protected places; the person may open any deck they can read. A refusal names the path first,
/// as it always did, so a file that is not a deck costs nothing and is said.
pub fn open_deck(path: &Path, agent: bool) -> Result<Opened, String> {
    open::open(&DeckRules, path, agent).map_err(|e| format!("{}: {e}", path.display()))
}

/// The most recently changed deck in the default folder. This is what the ribbon's Open button
/// opens: the screen has no file dialog, and a person at the keyboard almost always means "the
/// one I was just working on". A mind or a file browser hands a path in through `open`.
pub fn most_recent_deck(dir: &Path) -> Result<PathBuf, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("could not read {}: {e}", dir.display()))?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some(EXTENSION) {
            continue;
        }
        let Ok(when) = entry.metadata().and_then(|m| m.modified()) else { continue };
        if best.as_ref().is_none_or(|(best_when, _)| when > *best_when) {
            best = Some((when, path));
        }
    }
    best.map(|(_, p)| p).ok_or_else(|| format!("there are no decks in {} yet", dir.display()))
}
