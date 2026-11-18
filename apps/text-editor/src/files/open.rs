//! Opening a file into a tab: the framework's opener, held to the editor's limits.

use std::path::Path;

pub use yantrik_app::documents::files::open::{described, Opened};

use super::rules::TextRules;

/// Opens the file at `path` for a tab. An agent may name only a file in the person's home,
/// outside its protected places; the person may open any text file they can read.
pub fn open(path: &Path, agent: bool) -> Result<Opened, String> {
    yantrik_app::documents::files::open::open(&TextRules, path, agent)
}
