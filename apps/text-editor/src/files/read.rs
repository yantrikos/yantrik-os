//! Reading a file as the editor reads one: the framework's reader, held to the editor's limits.

use std::path::Path;

pub use yantrik_app::documents::files::read::Inode;

use super::rules::TextRules;

/// The file's text, or why it cannot be a document.
pub fn read_text(path: &Path) -> Result<String, String> {
    yantrik_app::documents::files::read::read_text(&TextRules, path)
}
