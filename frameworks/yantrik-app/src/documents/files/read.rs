//! Reading a file as a document app reads one: bounded, never through a link or into a FIFO,
//! UTF-8 with no lossy conversion, and held to what the app's document may be.

use std::fs::OpenOptions;
use std::io::Read as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::Path;

use super::FileRules;

/// Where a file is, as the disk names it (device and inode): how a file moved while it was open
/// is told from another file with the same bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inode {
    pub dev: u64,
    pub ino: u64,
}

/// The file's text, or why it cannot be a document.
pub fn read_text(rules: &dyn FileRules, path: &Path) -> Result<String, String> {
    let max = rules.max_bytes() as u64;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| rules.cannot_read(path, &e))?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err(rules.not_regular());
    }
    if meta.len() > max {
        return Err(rules.too_big(path, meta.len()));
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    // Strict, not lossy: replacing a bad byte and saving would write the damage back over the
    // original, which is worse than refusing to open it.
    let text = String::from_utf8(bytes).map_err(|_| rules.not_utf8())?;
    rules.validate(&text)?;
    Ok(text)
}

/// The bytes of a regular file of at most `limit` bytes, read without following a link or
/// blocking on a FIFO; `None` for anything else.
pub fn read_bytes(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

/// Where the file at `path` is, by device and inode, without following a link.
pub fn inode(path: &Path) -> Option<Inode> {
    std::fs::symlink_metadata(path).ok().map(|m| Inode { dev: m.dev(), ino: m.ino() })
}
