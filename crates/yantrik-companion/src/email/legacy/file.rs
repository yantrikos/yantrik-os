//! The file side of the migration: keep the person's `config.yaml`, and change it only after a
//! copy of it is safe.
//!
//! - [`check_target`] refuses a `config.yaml` that is a link or has other names: a rewrite would
//!   replace the link itself and leave its target (or the other name) holding the password.
//! - [`archive`] copies the original bytes to `config.yaml.before-1022-<stamp>`, mode 0600, never
//!   over anything already there, and never twice for the same bytes: a retry finds the copy it
//!   made and reuses it. It goes beside the config when that directory takes a new file, else in
//!   the person's own state directory, because on an installed machine `/opt/yantrik` belongs to
//!   root (#397) and only `config.yaml` itself is the person's. It holds the old password, so it
//!   is private, and it is never deleted by this code: the person decides.
//! - [`rewrite`] replaces the file atomically (whole or not at all) and keeps its mode. Where the
//!   directory takes no new file it writes the file in place instead, which is only done after
//!   the archive is synced and only to a regular file with one name that this user owns.
//!
//! None of it prints file contents.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// Why a `config.yaml` is left alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// It is a symbolic link.
    Link,
    /// It is not a plain file, or has more than one name.
    NotPlain,
}

impl Refusal {
    /// A fixed sentence for the notice.
    pub fn says(self) -> &'static str {
        match self {
            Refusal::Link => "it is a link to another file",
            Refusal::NotPlain => "it is not a plain file with a single name",
        }
    }
}

/// Whether `config` is a plain file with one name, which is the only kind that can be cleaned: a
/// link or a second name leaves the password in the file the rewrite does not reach.
pub fn check_target(config: &Path) -> Result<(), Refusal> {
    let Ok(meta) = std::fs::symlink_metadata(config) else { return Ok(()) };
    if meta.file_type().is_symlink() {
        return Err(Refusal::Link);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if meta.nlink() != 1 {
            return Err(Refusal::NotPlain);
        }
    }
    if !meta.is_file() {
        return Err(Refusal::NotPlain);
    }
    Ok(())
}

const ARCHIVE_PREFIX: &str = "before-1022-";

fn archive_name(config: &Path, stamp: &str, attempt: u32) -> String {
    let name = config.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "config.yaml".into());
    let suffix = if attempt == 0 { String::new() } else { format!("-{}", attempt + 1) };
    format!("{name}.{ARCHIVE_PREFIX}{stamp}{suffix}")
}

/// An archive of exactly these bytes that an earlier attempt made, in either place it goes.
fn existing(config: &Path, original: &[u8], dirs: &[&Path]) -> Option<PathBuf> {
    let base = config.file_name()?.to_string_lossy().to_string();
    let marker = format!("{base}.{ARCHIVE_PREFIX}");
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let named = entry.file_name().to_string_lossy().starts_with(&marker);
            let same = entry.metadata().is_ok_and(|m| m.is_file() && m.len() == original.len() as u64 && private(&m));
            if named && same && std::fs::read(&path).is_ok_and(|bytes| bytes == original) {
                return Some(path);
            }
        }
    }
    None
}

/// Whether a copy is as private as one made here would be: no access for group or others.
#[cfg(unix)]
fn private(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    meta.permissions().mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn private(_meta: &std::fs::Metadata) -> bool {
    true
}

/// Create a private file `path` holding `original`, synced. `AlreadyExists` means the name is taken.
fn create_private(path: &Path, original: &[u8]) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let kept = file.write_all(original).and_then(|()| file.sync_all());
    if let Err(e) = kept {
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    Ok(())
}

/// Keep `original` (the bytes read from `config`) and return where the copy is.
///
/// A copy made by an earlier attempt is reused: a rewrite that keeps failing must not leave a new
/// clear-text copy at every retry. Otherwise a new one is made beside the config, or in `fallback`
/// (made private if it has to be made) when the config's directory takes no new file. Created new,
/// 0600 from the moment it exists, owned by the config's owner where this process may say so, and
/// synced before it is reported.
pub fn archive(config: &Path, original: &[u8], stamp: &str, fallback: &Path) -> io::Result<PathBuf> {
    let beside = config.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    if let Some(path) = existing(config, original, &[beside, fallback]) {
        return Ok(path);
    }
    let mut last = io::Error::new(io::ErrorKind::AlreadyExists, "no free name for the archive");
    for attempt in 0..100 {
        let path = config.with_file_name(archive_name(config, stamp, attempt));
        match create_private(&path, original) {
            Ok(()) => {
                same_owner(config, &path);
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = e,
            // The directory is not ours to add to: the person's own state directory is.
            Err(_) => return archive_in(config, original, stamp, fallback),
        }
    }
    Err(last)
}

fn archive_in(config: &Path, original: &[u8], stamp: &str, dir: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let mut last = io::Error::new(io::ErrorKind::AlreadyExists, "no free name for the archive");
    for attempt in 0..100 {
        let path = dir.join(archive_name(config, stamp, attempt));
        match create_private(&path, original) {
            Ok(()) => return Ok(path),
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

/// Replace `config` with `text`, keeping the mode it had: atomically when the directory takes a
/// new file, else in place.
pub fn rewrite(config: &Path, text: &str) -> io::Result<()> {
    #[cfg(unix)]
    let mode = std::fs::metadata(config).ok().map(|m| m.permissions());
    match yantrik_foundation::json_store::write_text_atomically(config, text) {
        Ok(()) => {
            #[cfg(unix)]
            if let Some(mode) = mode {
                std::fs::set_permissions(config, mode)?;
            }
            Ok(())
        }
        Err(_) => rewrite_in_place(config, text),
    }
}

/// Write `text` over the file itself: opened without following a link, checked to be a regular
/// file with one name that this user owns, written from the start, cut to length and synced. Not
/// atomic, which is why [`archive`] comes first.
#[cfg(unix)]
fn rewrite_in_place(config: &Path, text: &str) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
    let mut file = std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NOFOLLOW).open(config)?;
    let meta = file.metadata()?;
    // SAFETY: `geteuid` has no preconditions.
    let me = unsafe { libc::geteuid() };
    if !meta.is_file() || meta.nlink() != 1 || meta.uid() != me {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "not a plain file of this user's"));
    }
    file.write_all(text.as_bytes())?;
    file.set_len(text.len() as u64)?;
    file.sync_all()
}

#[cfg(not(unix))]
fn rewrite_in_place(_config: &Path, _text: &str) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "no in-place rewrite here"))
}
