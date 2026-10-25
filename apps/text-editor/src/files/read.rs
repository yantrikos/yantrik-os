//! Reading a file as the editor reads one: bounded, never through a link or into a FIFO, UTF-8
//! with no lossy conversion, and held to what a document may be.

use std::fs::OpenOptions;
use std::io::Read as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::Path;

use crate::tab::Inode;
use crate::text::limits::{validate, MAX_BYTES};

/// The file's text, or why it cannot be a document.
pub fn read_text(path: &Path) -> Result<String, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("Cannot read: {e}"))?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Only regular text files can be opened.".into());
    }
    if meta.len() > MAX_BYTES as u64 {
        return Err("File exceeds the 1 MiB editing limit. It was not loaded.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|_| "File is not valid UTF-8; no lossy conversion was performed.")?;
    validate(&text)?;
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
