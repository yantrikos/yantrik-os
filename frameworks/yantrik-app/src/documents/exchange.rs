//! The replace that keeps what it replaced ([`super::disk::Disk::exchange`] on the real disk).
//!
//! On Linux the new file and the old trade names in one step (`renameat2(RENAME_EXCHANGE)`),
//! so what was at the path at that instant is kept at the temp name instead of being unlinked
//! unseen. Three rules keep that step from doing harm the plain rename never did:
//!
//! - Only a regular file is replaced. The exchange would trade places with a directory, a FIFO
//!   or a link as readily as with a file (a plain rename refuses a directory), so anything else
//!   found at the temp name afterwards is swapped straight back and the save fails as before.
//! - What was replaced is read back without following a link, without blocking on a FIFO, and
//!   only up to a limit. A file that cannot be read back so is left under its temp name
//!   ([`Displaced::Unread`]), never deleted unread.
//! - When nothing was at the path, the file is published with `RENAME_NOREPLACE`, so a file
//!   another program made in that instant is exchanged with, not replaced.
//!
//! Where the kernel or the filesystem has no such rename (NFS, 9p, FUSE, an old kernel, a
//! sandbox that forbids it), a plain rename is used and the answer is [`Displaced::Unknown`].

use std::io::Write as _;
use std::path::{Path, PathBuf};

use super::disk::{DiskError, Displaced, Exchanged, RealDisk, Step};
use crate::quiet::{self, Looked};

/// Read back at most this much of a replaced file; a larger one is kept under its temp name.
pub(super) const READ_BACK_LIMIT: u64 = 64 * 1024 * 1024;

/// How many times a file appearing in the instant before a no-replace publish is exchanged with.
const PUBLISH_TRIES: usize = 3;

pub(super) fn exchange(path: &Path, bytes: &[u8]) -> Result<Exchanged, DiskError> {
    let fail = |step: Step| move |e: std::io::Error| DiskError { step, error: e.to_string() };
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(DiskError { step: Step::Check, error: "the file is a link, and is not replaced".into() });
    }
    let (temp, mut file) = RealDisk::create_temp(path, true).map_err(fail(Step::CreateTemp))?;
    let written = file
        .write_all(bytes)
        .map_err(fail(Step::Write))
        .and_then(|()| file.sync_all().map_err(fail(Step::SyncFile)));
    drop(file);
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    let displaced = swap_in(&temp, path, READ_BACK_LIMIT)?;
    let parent = path.parent().unwrap_or(Path::new("."));
    // The new bytes are in place. A failed directory sync makes them unsure, and what they
    // replaced is still handed back: it exists nowhere else.
    let unsure = std::fs::File::open(parent).and_then(|d| d.sync_all()).map_err(fail(Step::SyncDir)).err();
    Ok(Exchanged { displaced, unsure })
}

/// Puts the written `temp` at `path` and says what it took the place of. On an error nothing at
/// `path` was replaced and `temp` is gone.
pub(super) fn swap_in(temp: &Path, path: &Path, limit: u64) -> Result<Displaced, DiskError> {
    let fail = |e: std::io::Error| DiskError { step: Step::Rename, error: e.to_string() };
    match place(temp, path) {
        Err(e) => {
            let _ = std::fs::remove_file(temp);
            Err(fail(e))
        }
        Ok(Placed::Created) => Ok(Displaced::Nothing),
        Ok(Placed::Renamed) => Ok(Displaced::Unknown),
        Ok(Placed::Exchanged) => {
            let regular = std::fs::symlink_metadata(temp).is_ok_and(|m| m.file_type().is_file());
            if regular {
                return Ok(read_back(temp, limit));
            }
            // A directory, a FIFO, a device or a link was at the path: put it back.
            match exchange_names(temp, path) {
                Ok(()) => {
                    let _ = std::fs::remove_file(temp);
                    Err(DiskError {
                        step: Step::Rename,
                        error: "what is at the path is not a regular file, and is not replaced".into(),
                    })
                }
                // Could not put it back: ours is in place, and theirs stays under the temp name.
                Err(_) => Ok(Displaced::Unread(temp.to_path_buf())),
            }
        }
    }
}

/// The replaced file's bytes, the temp removed; or the temp kept, unread, when it cannot be read
/// back safely ([`crate::quiet::read_within`]: a regular file within `limit`, never a link).
fn read_back(temp: &Path, limit: u64) -> Displaced {
    match quiet::read_within(temp, limit) {
        Ok(Looked::Read(bytes)) => {
            let _ = std::fs::remove_file(temp);
            Displaced::Bytes(bytes)
        }
        _ => Displaced::Unread(temp.to_path_buf()),
    }
}

/// What a replace that only lands over what it accepts came to ([`replace_if`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Guarded {
    /// The bytes are at the path; what they replaced, if anything, was accepted.
    Placed,
    /// What was at the path is not accepted: it is back at the path, and the bytes are not.
    Refused,
    /// What was at the path is not accepted and could not be put back: the bytes are at the
    /// path, and what they replaced is kept, whole, under this name beside it.
    KeptAside(PathBuf),
}

/// The suffix of [`replace_if`]'s temp, `.<file>.<pid>.<n>.exchange`. Not `.tmp`: after an
/// exchange the temp holds what was displaced, someone else's file with its own age, and the
/// stale-temp sweep for `.tmp` names would remove it unread. Only a sweep that recognises the
/// temp's content as the app's own may remove one of these (`settings::file::sweep_own_temps`).
pub(crate) const EXCHANGE_SUFFIX: &str = "exchange";

/// Replaces `path` with `bytes` only over nothing, or over a regular file of one name within
/// `limit` whose bytes `accept` takes ([`crate::quiet::read_own`]). The check is made on what
/// the exchange actually displaced, not on a look taken before it, so nothing that arrives in
/// between is replaced unseen. Nothing is removed unless it is known: the displaced file once
/// accepted, the app's own bytes once they are back under the temp name. Anything else is kept
/// aside under a name of its own. Where the filesystem has no exchange, a plain rename is used;
/// the caller's own look before is then the only check (the limit `Placed` cannot remove there).
pub(crate) fn replace_if(path: &Path, bytes: &[u8], limit: u64, accept: impl Fn(&[u8]) -> bool) -> std::io::Result<Guarded> {
    let (temp, mut file) = RealDisk::create_temp_as(path, false, EXCHANGE_SUFFIX)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    let placed = match place(&temp, path) {
        Ok(placed) => placed,
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            return Err(e);
        }
    };
    let outcome = match placed {
        Placed::Created | Placed::Renamed => Guarded::Placed,
        Placed::Exchanged => match quiet::read_own(&temp, limit) {
            Ok(Looked::Read(theirs)) if accept(&theirs) => {
                let _ = std::fs::remove_file(&temp);
                Guarded::Placed
            }
            // Not accepted, or not readable as a file of one name: theirs goes back.
            _ => match exchange_names(&temp, path) {
                Ok(()) => {
                    // What came back under the temp name is removed only if it is ours: a third
                    // writer may have put its own file at the path in that instant.
                    match quiet::read_own(&temp, limit) {
                        Ok(Looked::Read(back)) if back == bytes => {
                            let _ = std::fs::remove_file(&temp);
                        }
                        _ => {
                            let kept = keep_aside(&temp, path)?;
                            tracing::warn!(file = %path.display(), kept = %kept.display(), "a file that arrived at the path as it was put back is kept beside it, unread");
                        }
                    }
                    Guarded::Refused
                }
                Err(_) => Guarded::KeptAside(keep_aside(&temp, path)?),
            },
        },
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::File::open(parent).and_then(|d| d.sync_all());
    }
    Ok(outcome)
}

/// Moves what a replace displaced off its temp name to `<file>.set-aside-<pid>-<n>.json` beside
/// the path, never over a name that is there (`RENAME_NOREPLACE`; a look and a plain rename
/// where the filesystem has none). If no name can be had, it stays under the temp name, which
/// no sweep removes unless it holds the app's own bytes.
fn keep_aside(temp: &Path, path: &Path) -> std::io::Result<PathBuf> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    for n in 0..1000u32 {
        let kept = path.with_file_name(format!("{name}.set-aside-{}-{n}.json", std::process::id()));
        match publish_new(temp, &kept) {
            Ok(()) => return Ok(kept),
            Err(e) if is_present(&e) => continue,
            Err(e) if unsupported(&e) => {
                if std::fs::symlink_metadata(&kept).is_ok() {
                    continue;
                }
                std::fs::rename(temp, &kept)?;
                return Ok(kept);
            }
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free name to keep the displaced file under"))
}

/// How the new file took its place.
enum Placed {
    /// Exchanged with what was at the path, which is now at the temp name.
    Exchanged,
    /// Nothing was at the path, and nothing was replaced.
    Created,
    /// Renamed over whatever was there, unseen: no exchange on this filesystem.
    Renamed,
}

fn place(temp: &Path, path: &Path) -> std::io::Result<Placed> {
    for _ in 0..PUBLISH_TRIES {
        match exchange_names(temp, path) {
            Ok(()) => return Ok(Placed::Exchanged),
            Err(e) if is_absent(&e) => match publish_new(temp, path) {
                Ok(()) => return Ok(Placed::Created),
                // Made by someone else in that instant: exchange with it instead.
                Err(e) if is_present(&e) => continue,
                Err(e) if unsupported(&e) => break,
                Err(e) => return Err(e),
            },
            Err(e) if unsupported(&e) => break,
            Err(e) => return Err(e),
        }
    }
    std::fs::rename(temp, path)?;
    Ok(Placed::Renamed)
}

#[cfg(target_os = "linux")]
fn exchange_names(a: &Path, b: &Path) -> std::io::Result<()> {
    rename2(a, b, libc::RENAME_EXCHANGE)
}

#[cfg(target_os = "linux")]
fn publish_new(temp: &Path, path: &Path) -> std::io::Result<()> {
    rename2(temp, path, libc::RENAME_NOREPLACE)
}

#[cfg(target_os = "linux")]
fn rename2(a: &Path, b: &Path, flags: libc::c_uint) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;
    let a = CString::new(a.as_os_str().as_bytes())?;
    let b = CString::new(b.as_os_str().as_bytes())?;
    // SAFETY: both strings are valid and NUL-terminated for the call, which only reads them.
    // The raw syscall is used because not every libc this OS runs on wraps renameat2.
    let r = unsafe { libc::syscall(libc::SYS_renameat2, libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), flags) };
    if r == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn is_absent(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(libc::ENOENT)
}

#[cfg(target_os = "linux")]
fn is_present(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(libc::EEXIST)
}

/// No such rename here: a filesystem without it (EINVAL), a kernel without it (ENOSYS), or a
/// sandbox that forbids the call (EPERM; a real permission error fails the plain rename too).
#[cfg(target_os = "linux")]
fn unsupported(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(libc::EINVAL) | Some(libc::ENOSYS) | Some(libc::EPERM))
}

#[cfg(not(target_os = "linux"))]
fn exchange_names(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(not(target_os = "linux"))]
fn publish_new(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(not(target_os = "linux"))]
fn is_absent(_: &std::io::Error) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
fn is_present(_: &std::io::Error) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
fn unsupported(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::Unsupported
}

#[cfg(test)]
mod keep_aside_tests {
    use super::*;

    /// A displaced file is kept under a name of its own, never over one that is there.
    #[test]
    fn a_displaced_file_is_never_kept_over_a_name_that_is_there() {
        let d = std::env::temp_dir().join(format!("yantrik-keep-aside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join("settings.json");
        let first = d.join(format!("settings.json.set-aside-{}-0.json", std::process::id()));
        std::fs::write(&first, "kept before").unwrap();
        let temp = d.join(format!(".settings.json.{}.9.{EXCHANGE_SUFFIX}", std::process::id()));
        std::fs::write(&temp, "theirs").unwrap();
        let kept = keep_aside(&temp, &path).unwrap();
        assert_ne!(kept, first);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "kept before");
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "theirs");
        assert!(!temp.exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
