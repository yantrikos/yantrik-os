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
//! file the framework owns alone ([`read_own`], [`read_own_headed`]: a settings file, the file
//! a settings import comes from, a settings write's displaced file) is also refused when it has
//! more than one name (`st_nlink > 1`); the framework never makes a second name for one of its
//! files (it writes a temp and renames). A document ([`read_within`]) may be hard-linked by the person:
//! that is their own business, and a document's semantics stay as they were.
//!
//! `st_nlink` belongs to the file, not to the name, and changes whenever any name of it is
//! added or removed: a second name removed between the open and the `fstat` makes another
//! component's file look like ours alone (`ln email.json settings.json; rm settings.json`, in a
//! loop). The path is asked again (`lstat`: still this file, still one name) before and after
//! the read, but that is no proof either: the count drops before the name goes, so on ext4
//! `lstat` reports one name for a file whose other name is still there, about one look in three
//! under that loop (the review of #936 at 14fe65bd, finding F). The name checks are a first
//! layer, never the guarantee.
//!
//! The guarantee is the head ([`read_own_headed`]). Every file the framework reads as its own
//! (a settings file, the stale temps beside it, the file a settings import comes from, a
//! settings write's displaced file) is judged by its first bytes before the rest is read. They
//! are read [`STEP`] bytes at a time and handed to a [`Judge`], up to [`PROBE`] bytes, and only
//! a file the judge calls [`Head::Ours`] is read past them. Anything else is refused, and the
//! bytes looked at are dropped: never returned, logged, kept or fingerprinted. So another
//! component's file at the path, however the name checks were won, gives up at most [`PROBE`]
//! bytes, and one that does not begin like a file of the framework's gives up one [`STEP`].
//! What a judge looks for is a structure at a fixed place, not a string anywhere
//! (`settings::mark`): a value someone sets inside another component's file cannot make that
//! file begin with it. A head-judged file must also belong to this process's user (`st_uid`):
//! the framework made it, and a link to another user's readable file is not that.

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

/// What a file's first bytes, as far as they have been read, say about whose it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Head {
    /// It begins as a file the framework wrote: the rest may be read.
    Ours,
    /// It does not: nothing more of it is read.
    NotOurs,
    /// Too few bytes to tell yet.
    More,
}

/// Judges a file by its first bytes: `(the bytes so far, whether they are the whole file)`.
/// [`Head::More`] for the whole file, or once [`PROBE`] bytes are in, counts as
/// [`Head::NotOurs`].
pub type Judge<'a> = &'a dyn Fn(&[u8], bool) -> Head;

/// How the read is guarded, beyond being a regular file within the limit.
#[derive(Clone, Copy, Default)]
struct Guard<'a> {
    /// The file must have one name (no hard link to another component's file), checked on the
    /// opened descriptor and against the path before and after the read. A first layer only.
    single_name: bool,
    /// A file the framework wrote: it must belong to this process's user, and its first bytes
    /// must satisfy the judge before the rest is read. `None` reads to the limit.
    head: Option<Judge<'a>>,
}

/// The most bytes read of a file before its judge must have called it ours: room for a mark
/// after some whitespace, little enough that a refused file gives up next to nothing.
pub const PROBE: u64 = 256;

/// The head is read this much at a time, so a file that does not even begin like one of the
/// framework's (`[`, for an account list) gives up one step, not the whole probe.
pub const STEP: u64 = 64;

/// What is at `path`, read only if it is a regular file of at most `limit` bytes.
pub fn read_within(path: &Path, limit: u64) -> std::io::Result<Looked> {
    read_checked(path, limit, Guard::default())
}

/// [`read_within`] for a file that should have one name: a file with a second name (a hard
/// link) is refused too, and none of it is read. Content-blind, for what has no head to judge
/// (a document's fingerprint, the shell's `settings.yaml`); a file the framework wrote is read
/// with [`read_own_headed`].
pub fn read_own(path: &Path, limit: u64) -> std::io::Result<Looked> {
    read_checked(path, limit, Guard { single_name: true, head: None })
}

/// [`read_own`] for a file the framework wrote: refused too unless it belongs to this process's
/// user and `judge` calls its first bytes ours, so anything else at the path gives up at most
/// [`PROBE`] bytes, whatever a race did to the name checks.
pub fn read_own_headed(path: &Path, limit: u64, judge: Judge) -> std::io::Result<Looked> {
    read_checked(path, limit, Guard { single_name: true, head: Some(judge) })
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

/// Whether the file belongs to this process's user, who made every file the framework wrote.
#[cfg(unix)]
fn owned(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    // SAFETY: geteuid has no preconditions and cannot fail.
    meta.uid() == unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn owned(_: &std::fs::Metadata) -> bool {
    true
}

/// Reads `file`'s first bytes into `bytes`, a [`STEP`] at a time, until `judge` can tell:
/// `true` when the rest may be read. A file it cannot call ours within [`PROBE`] bytes, or by
/// its end, is not.
fn head_is_ours(file: &std::fs::File, bytes: &mut Vec<u8>, judge: Judge) -> std::io::Result<bool> {
    loop {
        let want = STEP.min(PROBE - bytes.len() as u64);
        let got = file.take(want).read_to_end(bytes)? as u64;
        let whole = got < want;
        match judge(bytes, whole) {
            Head::Ours => return Ok(true),
            Head::NotOurs => return Ok(false),
            Head::More if whole || bytes.len() as u64 >= PROBE => return Ok(false),
            Head::More => {}
        }
    }
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
    let foreign = guard.head.is_some() && !owned(&meta);
    if !meta.is_file() || meta.len() > limit || (guard.single_name && names(&meta) != 1) || foreign {
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
    // The head is judged before the rest is read: a file that is not the framework's gives up
    // at most PROBE bytes, race won or not, and they are dropped here.
    let mut bytes = Vec::with_capacity((meta.len() as usize).min(PROBE as usize));
    if let Some(judge) = guard.head {
        if !head_is_ours(&file, &mut bytes, judge)? {
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

#[cfg(test)]
#[path = "quiet_tests.rs"]
pub(crate) mod tests;
