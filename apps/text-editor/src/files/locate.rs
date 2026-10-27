//! Finding a file that moved while it was open: the framework's search, reading as the editor reads.

use std::path::{Path, PathBuf};

use super::read::{read_text, Inode};

/// Where the file that was at `original`, holding `on_disk`, is now; `None` when it was not found.
pub fn locate(original: &Path, on_disk: &str, inode: Option<Inode>) -> Option<PathBuf> {
    yantrik_app::documents::files::locate::locate(original, on_disk, inode, read_text)
}
