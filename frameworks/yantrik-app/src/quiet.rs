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
//!
//! A hard link is not a link to follow: it is the other file itself, under a second name. So a
//! file the framework owns alone ([`read_own`]: a settings file, the file a settings import
//! comes from, a settings write's displaced file) is also refused when it has more than one
//! name (`st_nlink > 1`); the framework never makes a second name for one of its files (it
//! writes a temp and renames). A document ([`read_within`]) may be hard-linked by the person:
//! that is their own business, and a document's semantics stay as they were.
//!
//! `st_nlink` belongs to the file, not to the name, and changes whenever any name of it is
//! added or removed: a second name removed between the open and the `fstat` would make another
//! component's file look like ours alone (`ln email.json settings.json; rm settings.json`, in a
//! loop). So [`read_own`] also asks the path, after the `fstat` and before a byte is read
//! (`lstat`): the name we opened must still be this very file (same device and inode), and the
//! file must have that one name there too. Then, at an instant after the open, the file's only
//! name was our path. Our name removed, replaced or given a twin in between is refused, unread.
//! The same two looks are taken again after the read, and the bytes are dropped if either
//! changed. What this cannot tell apart, and does not try to: a file whose *other* name was
//! removed, or renamed away, so that ours is the only one left. Whoever did that has put the
//! content at our path themselves, which is the same as writing to our path, and nothing the
//! app does at read time can prevent it.

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

/// How the read is guarded, beyond being a regular file within the limit.
#[derive(Clone, Copy, Default)]
pub struct Guard<'a> {
    /// The file must have one name (no hard link to another component's file), checked on the
    /// opened descriptor and against the path before and after the read.
    pub single_name: bool,
    /// A needle the file's first [`PROBE`] bytes must contain, or it is refused before the rest
    /// is read. The store's mark (`"yantrik-app-settings"`) sits in the first tens of bytes of
    /// every file the framework writes, so a file without it gives up at most [`PROBE`] bytes,
    /// even when a race wins the name checks. `None` reads to the limit once the checks pass.
    pub needle: Option<&'a [u8]>,
}

/// How many bytes are read before the needle must have been seen. Large enough for the mark in
/// a pretty-printed file with leading whitespace, small enough that a refused file gives up
/// little: well under the 16 KiB a test calls "nothing of it".
pub const PROBE: u64 = 1024;

/// What is at `path`, read only if it is a regular file of at most `limit` bytes.
pub fn read_within(path: &Path, limit: u64) -> std::io::Result<Looked> {
    read_checked(path, limit, Guard::default())
}

/// [`read_within`] for a file the framework owns alone: a file with a second name (a hard link
/// to someone else's file) is refused too, and none of it is read.
pub fn read_own(path: &Path, limit: u64) -> std::io::Result<Looked> {
    read_checked(path, limit, Guard { single_name: true, needle: None })
}

/// [`read_own`] that also refuses, after at most [`PROBE`] bytes, a file whose start does not
/// carry `needle`: a file the framework owns alone, recognised by its mark before it is read in
/// full, so another component's file at the path gives up only its first [`PROBE`] bytes.
pub fn read_own_marked(path: &Path, limit: u64, needle: &[u8]) -> std::io::Result<Looked> {
    read_checked(path, limit, Guard { single_name: true, needle: Some(needle) })
}

#[cfg(unix)]
fn names(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    meta.nlink()
}

#[cfg(not(unix))]
fn names(_: &std::fs::Metadata) -> u64 {
    1
}

/// What the path is now, against the file that was opened from it.
enum Name {
    /// The opened file, under this one name only.
    Sole,
    /// The opened file, with another name besides.
    Shared,
    /// Not the opened file any more: removed, or replaced by another.
    Moved,
}

#[cfg(unix)]
fn name_now(path: &Path, of: &std::fs::Metadata) -> Name {
    use std::os::unix::fs::MetadataExt as _;
    match std::fs::symlink_metadata(path) {
        Ok(now) if now.file_type().is_file() && now.dev() == of.dev() && now.ino() == of.ino() => {
            if now.nlink() == 1 {
                Name::Sole
            } else {
                Name::Shared
            }
        }
        _ => Name::Moved,
    }
}

#[cfg(not(unix))]
fn name_now(_: &Path, _: &std::fs::Metadata) -> Name {
    Name::Sole
}

/// How many times a path that moved under a read is read afresh. Each try is checked whole; a
/// path still moving after these is refused. The app's own atomic writes rename over its file,
/// and a read that met one is not to be taken for a file that is not the app's.
const TRIES: usize = 3;

fn read_checked(path: &Path, limit: u64, guard: Guard) -> std::io::Result<Looked> {
    let mut moved = None;
    for _ in 0..TRIES {
        match read_once(path, limit, guard)? {
            Ok(looked) => return Ok(looked),
            Err(stat) => moved = Some(stat),
        }
    }
    Ok(Looked::Refused(moved.unwrap_or_default()))
}

/// One look: `Ok` with what it came to, or `Err` with the opened file's metadata when the path
/// no longer named that file at one of the looks (and nothing read from it is kept).
fn read_once(path: &Path, limit: u64, guard: Guard) -> std::io::Result<Result<Looked, Stat>> {
    let file = match open_quietly(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Ok(Looked::Missing)),
        Err(e) if refused_by_open(&e) => {
            // The link itself, or the node itself: its metadata, nothing it points at.
            return match std::fs::symlink_metadata(path) {
                Ok(meta) => Ok(Ok(Looked::Refused(Stat::of(&meta)))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Ok(Looked::Missing)),
                Err(e) => Err(e),
            };
        }
        Err(e) => return Err(e),
    };
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > limit || (guard.single_name && names(&meta) != 1) {
        return Ok(Ok(Looked::Refused(Stat::of(&meta))));
    }
    // The name asked again, before a byte is read: still this file, and its only name.
    if guard.single_name {
        match name_now(path, &meta) {
            Name::Sole => {}
            Name::Shared => return Ok(Ok(Looked::Refused(Stat::of(&meta)))),
            Name::Moved => return Ok(Err(Stat::of(&meta))),
        }
    }
    // The mark must be in the first PROBE bytes, or the rest is never read: a file that is not
    // the framework's gives up at most PROBE bytes, race won or not.
    let mut bytes = Vec::with_capacity((meta.len() as usize).min(PROBE as usize));
    if let Some(needle) = guard.needle {
        (&file).take(PROBE).read_to_end(&mut bytes)?;
        if find(&bytes, needle).is_none() {
            // Not the mark in the first PROBE bytes; if the whole file was shorter than that, it
            // simply is not ours, and if it was longer, the rest is never touched.
            return Ok(Ok(Looked::Refused(Stat::of(&meta))));
        }
    }
    let already = bytes.len() as u64;
    (&file).take(limit + 1 - already).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        // It grew past the limit while it was read: what was read is dropped, not used.
        return Ok(Ok(Looked::Refused(Stat::of(&meta))));
    }
    if guard.single_name {
        // And once more after: a name added or removed during the read drops what was read.
        let after = file.metadata()?;
        if names(&after) != 1 {
            return Ok(Ok(Looked::Refused(Stat::of(&after))));
        }
        match name_now(path, &after) {
            Name::Sole => {}
            Name::Shared => return Ok(Ok(Looked::Refused(Stat::of(&after)))),
            Name::Moved => return Ok(Err(Stat::of(&after))),
        }
    }
    Ok(Ok(Looked::Read(bytes)))
}

/// Where `needle` first appears in `haystack`, or `None`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
#[path = "quiet_tests.rs"]
pub(crate) mod tests;
