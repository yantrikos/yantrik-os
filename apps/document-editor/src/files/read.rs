//! Reading a file as yDoc reads one: the framework's reader, held to yDoc's limits.

use std::path::Path;

pub use yantrik_app::documents::files::read::Inode;

use super::rules::DocRules;

/// The file's text, or why it cannot be a document.
pub fn read_text(path: &Path) -> Result<String, String> {
    yantrik_app::documents::files::read::read_text(&DocRules, path)
}
