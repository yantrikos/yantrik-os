//! Opening a file as the document: the framework's opener, held to yDoc's limits.

use std::path::Path;

pub use yantrik_app::documents::files::open::{described, Opened};

use super::rules::DocRules;

/// Opens the file at `path`. An agent may name only a file in the person's home, outside its
/// protected places; the person may open any text file they can read.
pub fn open(path: &Path, agent: bool) -> Result<Opened, String> {
    yantrik_app::documents::files::open::open(&DocRules, path, agent)
}
