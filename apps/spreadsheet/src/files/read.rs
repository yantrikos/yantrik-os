//! Reading a file as ySheets reads one: the framework's reader, held to the format.

use std::path::{Path, PathBuf};

pub use yantrik_app::documents::files::read::Inode;

use super::rules::SheetRules;

/// The file's text, or why it cannot be a sheet. The reason reads as the end of "<path> ...".
pub fn read_text(path: &Path) -> Result<String, String> {
    yantrik_app::documents::files::read::read_text(&SheetRules, path)
}

/// Where the file that was at `original`, holding `on_disk`, is now; `None` when it was not found.
pub fn locate(original: &Path, on_disk: &str, inode: Option<Inode>) -> Option<PathBuf> {
    yantrik_app::documents::files::locate::locate(original, on_disk, inode, read_text)
}
