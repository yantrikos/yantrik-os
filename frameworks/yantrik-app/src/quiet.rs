//! Reading a small file without being made to read anything else: the one way the framework
//! reads a file someone other than the app may have put there (a settings file, the file a
//! settings import comes from, a file a replace displaced) or fingerprints one.
//!
//! The file is opened without following a link (`O_NOFOLLOW`), without blocking on a FIFO
//! (`O_NONBLOCK`), without taking a terminal (`O_NOCTTY`) and without leaking into a child
//! (`O_CLOEXEC`). What was opened is then checked by its descriptor, not its path (`fstat`):
//! only a regular file within the limit is read, from that same descriptor, and never past the
//! limit even if it grows meanwhile. So no swap of the path between the check and the read can
//! lead the read elsewhere. Anything else (a link, a FIFO, a socket, a device, a directory, a
//! file too large) is [`Looked::Refused`] with its metadata only, and none of its bytes are read.
//!
//! Only the last component is not followed: the directories above are the app's own.

use std::io::Read as _;
use std::path::Path;

/// What is at a path, as far as it was looked at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Looked {
    /// Nothing.
    Missing,
    /// A regular file within the limit, and all of its bytes.
    Read(Vec<u8>),
    /// Something that is not read: what it is, never what it holds.
    Refused(Stat),
}

/// A thing's identity and change marks, from its metadata alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stat {
    pub dev: u64,
    pub ino: u64,
    pub len: u64,
    pub mtime_ns: i128,
}

impl Stat {
    fn of(meta: &std::fs::Metadata) -> Stat {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            Stat {
                dev: meta.dev(),
                ino: meta.ino(),
                len: meta.len(),
                mtime_ns: i128::from(meta.mtime()) * 1_000_000_000 + i128::from(meta.mtime_nsec()),
            }
        }
        #[cfg(not(unix))]
        {
            let mtime_ns = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos() as i128);
            Stat { dev: 0, ino: 0, len: meta.len(), mtime_ns }
        }
    }
}

/// Opens `path` for reading without following a link, blocking on a FIFO or taking a terminal.
#[cfg(unix)]
pub fn open_quietly(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC)
        .open(path)
}

#[cfg(not(unix))]
pub fn open_quietly(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

/// The open refused what is there without opening it: a link (`ELOOP`), or a socket or a FIFO
/// that cannot be opened so (`ENXIO`).
#[cfg(unix)]
fn refused_by_open(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(libc::ELOOP) | Some(libc::ENXIO))
}

#[cfg(not(unix))]
fn refused_by_open(_: &std::io::Error) -> bool {
    false
}

/// What is at `path`, read only if it is a regular file of at most `limit` bytes.
pub fn read_within(path: &Path, limit: u64) -> std::io::Result<Looked> {
    let file = match open_quietly(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Looked::Missing),
        Err(e) if refused_by_open(&e) => {
            // The link itself, or the node itself: its metadata, nothing it points at.
            return match std::fs::symlink_metadata(path) {
                Ok(meta) => Ok(Looked::Refused(Stat::of(&meta))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Looked::Missing),
                Err(e) => Err(e),
            };
        }
        Err(e) => return Err(e),
    };
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > limit {
        return Ok(Looked::Refused(Stat::of(&meta)));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        // It grew past the limit while it was read: what was read is dropped, not used.
        return Ok(Looked::Refused(Stat::of(&meta)));
    }
    Ok(Looked::Read(bytes))
}

#[cfg(test)]
#[path = "quiet_tests.rs"]
pub(crate) mod tests;
