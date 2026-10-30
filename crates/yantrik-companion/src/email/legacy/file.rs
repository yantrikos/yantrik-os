//! The file side of the migration: keep the person's `config.yaml`, and change it only after a
//! copy of it is safe.
//!
//! - [`archive`] copies the original bytes to `config.yaml.before-1022-<stamp>`, mode 0600, never
//!   over anything already there. It holds the old clear-text password, so it is private, and it is
//!   never deleted by this code: the person decides.
//! - [`rewrite`] replaces the file atomically (whole or not at all) and keeps its mode, so another
//!   reader of the file is not locked out.
//!
//! Neither prints file contents.

use std::io;
use std::path::{Path, PathBuf};

/// Where the archive of `config` goes for `stamp`, and the next free name if that one is taken.
fn archive_path(config: &Path, stamp: &str, attempt: u32) -> PathBuf {
    let name = config.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "config.yaml".into());
    let suffix = if attempt == 0 { String::new() } else { format!("-{}", attempt + 1) };
    config.with_file_name(format!("{name}.before-1022-{stamp}{suffix}"))
}

/// Copy `original` (the bytes read from `config`) to its archive and return where it went.
///
/// Created new, mode 0600 from the moment it exists, and owned by the owner of `config` where this
/// process is allowed to say so. Synced before it is reported, so a rewrite that follows cannot
/// outlive the copy it depends on.
pub fn archive(config: &Path, original: &[u8], stamp: &str) -> io::Result<PathBuf> {
    use std::io::Write as _;
    let mut last = io::Error::new(io::ErrorKind::AlreadyExists, "no free name for the archive");
    for attempt in 0..100 {
        let path = archive_path(config, stamp, attempt);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                let kept = file.write_all(original).and_then(|()| file.sync_all());
                if let Err(e) = kept {
                    let _ = std::fs::remove_file(&path);
                    return Err(e);
                }
                same_owner(config, &path);
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// Give `copy` the owner of `original`. Best effort: only the owner or root may, and when the
/// shell runs as the file's owner (the usual case) it already is.
#[cfg(unix)]
fn same_owner(original: &Path, copy: &Path) {
    use std::os::unix::fs::MetadataExt as _;
    if let Ok(meta) = std::fs::metadata(original) {
        let _ = std::os::unix::fs::chown(copy, Some(meta.uid()), Some(meta.gid()));
    }
}

#[cfg(not(unix))]
fn same_owner(_original: &Path, _copy: &Path) {}

/// Replace `config` with `text`, atomically, keeping the mode it had.
pub fn rewrite(config: &Path, text: &str) -> io::Result<()> {
    #[cfg(unix)]
    let mode = std::fs::metadata(config).ok().map(|m| m.permissions());
    yantrik_foundation::json_store::write_text_atomically(config, text)?;
    #[cfg(unix)]
    if let Some(mode) = mode {
        std::fs::set_permissions(config, mode)?;
    }
    Ok(())
}
