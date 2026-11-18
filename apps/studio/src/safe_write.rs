//! Writing a file Studio owns without following a link someone planted, and without leaving a
//! half-written or empty file after a crash.
//!
//! A new file is created with `O_CREAT | O_EXCL | O_NOFOLLOW`: it is a file nobody had, or the
//! write fails, and a symlink already standing at the name is never written through. Replacing a
//! file writes a new one beside it, flushes it to the disk (`sync_all`), renames it over the old
//! one, and flushes the folder, so after a crash the old file or the whole new one is there and
//! never an empty one.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// A file that did not exist, opened for writing, at this mode.
pub fn create_new(path: &Path, mode: u32) -> std::io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

/// Write `bytes` to a file that does not exist yet, and flush them.
pub fn write_new(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    let mut file = create_new(path, mode)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Where the new copy is written before it replaces `path`.
fn beside(path: &Path) -> PathBuf {
    path.with_extension("json.new")
}

/// Put `bytes` at `path`, replacing what is there, all of it or none of it. A leftover from an
/// earlier crash at the temporary name is removed first (a link there is removed, not followed).
pub fn replace(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let temporary = beside(path);
    let written = match write_new(&temporary, bytes, mode) {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(&temporary);
            write_new(&temporary, bytes, mode)
        }
        other => other,
    };
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("{} could not be written ({e})", temporary.display()));
    }
    if let Err(e) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("{} could not be put in place ({e})", path.display()));
    }
    // The rename is only durable once the folder that holds it is.
    if let Some(parent) = path.parent() {
        if let Ok(folder) = File::open(parent) {
            let _ = folder.sync_all();
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "safe_write_tests.rs"]
mod tests;
