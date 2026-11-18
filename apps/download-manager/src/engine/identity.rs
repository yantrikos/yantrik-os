//! Proof that a file is the one a download made, and nothing else.
//!
//! The rule: the app changes a file only when it has shown the file is its own, and the showing is
//! done on the same open file descriptor it then writes to. A path can be swapped between a check
//! and an open; a descriptor cannot.
//!
//! What is recorded when the app creates a file, and again each time it has written to it, is a
//! stamp. The file's identity is the first three parts: device, inode and birth time, and only
//! those must match. Device and inode alone are not enough, because a filesystem hands a freed
//! inode to the next file made; a new file has another birth time. Size and modification time
//! are in the stamp but are not identity, since a recreated file can be given the same values.
//! After a crash the file is longer than the last stamp says, because the app wrote more than it
//! had recorded; those bytes are its own, so a resume continues from the file's actual size,
//! read on the verified descriptor, provided that is not less than the recorded one (less means
//! the file was truncated, and the download restarts under a new name).
//!
//! Where the filesystem gives no birth time the stamp carries `-` in its place and is not a proof
//! at all: the app refuses to resume, to delete on cancel and to delete on retry, and leaves the
//! file. Mtime and size alone are never accepted, because a recreated file can match them.
//!
//! Taking a file away is done by quarantine, never by unlinking a name. `unlinkat` acts on a name
//! and nothing ties the name to the inode that was checked, so a file the person saves there
//! between the check and the unlink would go. Instead the file is first renamed, without replacing
//! anything, into a private folder on the same filesystem; the stamp is checked on what was moved;
//! only then is that held name unlinked. A mismatch moves the file back, and if its name has been
//! taken meanwhile it is kept under a recovery name and the person is told where. The worst case
//! is "the person's file moved for a moment and came back", never "deleted".
//!
//! This is Linux only. Elsewhere no stamp can be taken, so the app never deletes a partial file
//! and never resumes one: it refuses rather than guess.

use super::*;

/// The stamp format. A row saved with another format (an earlier build wrote `dev:ino`) has no
/// stamp this build can check, so it is treated as having none.
const TAG: &str = "v2:";

/// The stamp of the file open as `file`, as text; `None` if it is not a plain file, or the
/// platform cannot say.
pub(super) fn stamp_fd(file: &File) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        sys::stamp(file.as_raw_fd(), c"", libc::AT_EMPTY_PATH)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = file;
        None
    }
}

/// Is this text a stamp this build can prove ownership with? It needs the birth time: without
/// one the stamp is kept as a record but never acted on.
pub(super) fn has_stamp(text: &str) -> bool {
    text.strip_prefix(TAG).is_some_and(|rest| rest.split(':').nth(2).is_some_and(|birth| birth != "-"))
}

/// The identity inside a stamp: device, inode and birth time. Birth time is new for every file
/// created, so a recreated file never has the same one. Size and modification time are not part
/// of it, because a recreated file can be given the same values.
fn identity(stamp: &str) -> Option<&str> {
    let rest = stamp.strip_prefix(TAG)?;
    let end = rest.match_indices(':').nth(2).map(|(i, _)| i)?;
    Some(&rest[..end])
}

/// Is the file with stamp `now` the file that `recorded` was taken of? Needs a birth time.
pub(super) fn same_file(recorded: &str, now: &str) -> bool {
    has_stamp(recorded) && identity(recorded).is_some() && identity(recorded) == identity(now)
}

/// The size recorded in a stamp: how many bytes the app had written when it took it.
pub(super) fn recorded_size(stamp: &str) -> Option<u64> {
    stamp.strip_prefix(TAG)?.rsplit(':').next()?.parse().ok()
}

/// Open the download's partial file to append, and prove on that descriptor that it is the one
/// the download wrote. `Ok(None)` if nothing is at the path. Keep the descriptor for the whole
/// transfer; do not open the path again.
pub(super) fn open_owned(download: &Download) -> Result<Option<File>, String> {
    let mut options = OpenOptions::new();
    options.append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // `O_NONBLOCK` so a pipe put at the path cannot hold the thread; it is refused below.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    let path = download.path();
    let file = match options.open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if stamp_fd(&file).is_some_and(|now| same_file(&download.file_id, &now)) {
        Ok(Some(file))
    } else {
        Err(format!("the file at {} is not provably this download's own", path.display()))
    }
}

/// What taking a file away came to.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Removal {
    /// The file was the app's own, and is gone.
    Removed,
    /// Nothing was deleted, and whatever was at the name is where it was (or was never there).
    Kept,
    /// Something that is not the app's was moved aside and could not go back; it is here.
    Recovered(PathBuf),
}

/// Delete the file at `path` if, and only if, it still has the stamp `expected`, by quarantine
/// (see the top of this file). Never deletes when it cannot prove the file or has no same-folder
/// place to hold it.
pub(super) fn remove_if_ours(path: &Path, expected: &str) -> Removal {
    if !has_stamp(expected) {
        return Removal::Kept;
    }
    #[cfg(target_os = "linux")]
    {
        sys::quarantine_remove(path, expected)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Removal::Kept
    }
}

impl Engine {
    /// Carry out `remove_if_ours` and say aloud when a file of the person's had to be set aside.
    /// True if the app's own file was deleted.
    pub(super) fn remove_owned(&self, path: &Path, expected: &str) -> bool {
        match remove_if_ours(path, expected) {
            Removal::Removed => true,
            Removal::Kept => false,
            Removal::Recovered(at) => {
                tracing::warn!(from = %path.display(), kept = %at.display(), "a file that is not the download's was set aside and kept");
                self.set_notice(format!(
                    "{} is not the file this download made, so it was not deleted. It is kept at {}",
                    path.display(),
                    at.display()
                ));
                false
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod sys {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::sync::atomic::AtomicU64;
    use std::os::unix::ffi::OsStrExt;

    /// `statx` on `dirfd`/`name`: the stamp of a plain file, `None` for anything else.
    pub(super) fn stamp(dirfd: i32, name: &CStr, flags: i32) -> Option<String> {
        // SAFETY: `st` is a plain-data out parameter and `name` a NUL-terminated string.
        let mut st: libc::statx = unsafe { std::mem::zeroed() };
        let mask = libc::STATX_BASIC_STATS | libc::STATX_BTIME;
        let rc = unsafe { libc::statx(dirfd, name.as_ptr(), flags, mask, &mut st) };
        if rc != 0 || u32::from(st.stx_mode) & libc::S_IFMT != libc::S_IFREG {
            return None;
        }
        let dev = (u64::from(st.stx_dev_major) << 32) | u64::from(st.stx_dev_minor);
        let birth = if st.stx_mask & libc::STATX_BTIME != 0 {
            format!("{}.{}", st.stx_btime.tv_sec, st.stx_btime.tv_nsec)
        } else {
            "-".to_string()
        };
        Some(format!(
            "{TAG}{dev}:{}:{birth}:{}.{}:{}",
            st.stx_ino, st.stx_mtime.tv_sec, st.stx_mtime.tv_nsec, st.stx_size
        ))
    }

    /// The private folder, made inside the download's own folder so a rename into it never
    /// crosses a filesystem. Hidden, owner-only, and removed again when it is empty.
    const QUARANTINE: &CStr = c".yantrik-downloads-held";
    /// The flag of `renameat2` that refuses to replace anything.
    const NOREPLACE: u32 = 1;

    fn rename_noreplace(from_dir: i32, from: &CStr, to_dir: i32, to: &CStr) -> bool {
        // SAFETY: both names are NUL-terminated and both descriptors are open folders.
        unsafe { libc::syscall(libc::SYS_renameat2, from_dir, from.as_ptr(), to_dir, to.as_ptr(), NOREPLACE) == 0 }
    }

    fn open_dir_at(parent: i32, name: &CStr) -> Option<OwnedFd> {
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        // SAFETY: `name` is NUL-terminated; a failed call returns -1, which is checked.
        let fd = unsafe { libc::openat(parent, name.as_ptr(), flags) };
        // SAFETY: a non-negative result is a descriptor this call owns.
        (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// The quarantine folder, made if needed, and checked to be ours and closed to others.
    fn open_quarantine(dirfd: i32) -> Option<OwnedFd> {
        for _ in 0..3 {
            // SAFETY: NUL-terminated name; EEXIST is fine, any other failure shows at the open.
            unsafe { libc::mkdirat(dirfd, QUARANTINE.as_ptr(), 0o700) };
            let Some(fd) = open_dir_at(dirfd, QUARANTINE) else { continue };
            // SAFETY: `st` is a plain-data out parameter.
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } != 0 || st.st_uid != unsafe { libc::geteuid() } {
                return None;
            }
            if st.st_mode & 0o077 != 0 && unsafe { libc::fchmod(fd.as_raw_fd(), 0o700) } != 0 {
                return None;
            }
            return Some(fd);
        }
        None
    }

    /// Take the quarantine folder away if nothing is in it. Failure means something is, which is
    /// the right answer.
    fn tidy(dirfd: i32) {
        // SAFETY: NUL-terminated name.
        unsafe { libc::unlinkat(dirfd, QUARANTINE.as_ptr(), libc::AT_REMOVEDIR) };
    }

    pub(super) fn quarantine_remove(path: &Path, expected: &str) -> Removal {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { return Removal::Kept };
        let (Ok(name), Ok(dir_c)) = (CString::new(name.as_bytes()), CString::new(dir.as_os_str().as_bytes())) else {
            return Removal::Kept;
        };
        let Some(dirfd) = open_dir_at(libc::AT_FDCWD, &dir_c) else { return Removal::Kept };
        let dirfd = dirfd.as_raw_fd();
        let Some(qfd) = open_quarantine(dirfd) else { return Removal::Kept };
        let qfd_raw = qfd.as_raw_fd();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let held_text = format!("held-{}-{n}", std::process::id());
        let Ok(held) = CString::new(held_text.as_str()) else { return Removal::Kept };
        let held_at = || dir.join(".yantrik-downloads-held").join(&held_text);

        // Step one: the name goes first, and nothing is replaced. EXDEV, a missing file or any
        // other failure ends it here with nothing touched.
        if !rename_noreplace(dirfd, &name, qfd_raw, &held) {
            tidy(dirfd);
            return Removal::Kept;
        }
        // Step two: look at what was moved, through the private folder, never following a link.
        // SAFETY: NUL-terminated name; a failed call returns -1, which is checked.
        let opened = unsafe {
            libc::openat(qfd_raw, held.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        };
        let ours = opened >= 0 && {
            // SAFETY: a non-negative result is a descriptor this call owns.
            let file = unsafe { File::from_raw_fd(opened) };
            stamp_fd(&file).is_some_and(|now| same_file(expected, &now))
                && stamp(qfd_raw, &held, libc::AT_SYMLINK_NOFOLLOW).is_some_and(|now| same_file(expected, &now))
        };
        if ours {
            // Step three: nothing else renames in this folder, so the held name is that file.
            #[cfg(test)]
            super::seam::run("before_unlink", path);
            // SAFETY: NUL-terminated name.
            if unsafe { libc::unlinkat(qfd_raw, held.as_ptr(), 0) } == 0 {
                tidy(dirfd);
                return Removal::Removed;
            }
            return Removal::Recovered(held_at());
        }
        // Not the app's: put it back. Its name may have been taken meanwhile; then it is kept
        // under a recovery name beside it, and if even that fails, where it is.
        if rename_noreplace(qfd_raw, &held, dirfd, &name) {
            tidy(dirfd);
            return Removal::Kept;
        }
        let base = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        for k in 1..=50 {
            let recovered = if k == 1 { format!("{base}.recovered") } else { format!("{base}.recovered-{k}") };
            let Ok(recovered_c) = CString::new(recovered.as_str()) else { break };
            if rename_noreplace(qfd_raw, &held, dirfd, &recovered_c) {
                tidy(dirfd);
                return Removal::Recovered(dir.join(recovered));
            }
        }
        Removal::Recovered(held_at())
    }
}

/// A hook the tests use to swap a file at a chosen moment. Keyed by path, so tests that run side
/// by side in their own folders never see each other's.
#[cfg(test)]
pub(super) mod seam {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    type Hook = Box<dyn FnOnce() + Send>;
    type Hooks = Mutex<HashMap<(String, PathBuf), Hook>>;

    fn hooks() -> &'static Hooks {
        static HOOKS: OnceLock<Hooks> = OnceLock::new();
        HOOKS.get_or_init(Default::default)
    }

    /// Run `hook` once, the next time the app reaches `point` for `path`.
    pub fn on(point: &str, path: &Path, hook: impl FnOnce() + Send + 'static) {
        hooks().lock().unwrap().insert((point.to_string(), path.to_path_buf()), Box::new(hook));
    }

    pub fn run(point: &str, path: &Path) {
        let hook = hooks().lock().unwrap().remove(&(point.to_string(), path.to_path_buf()));
        if let Some(hook) = hook {
            hook();
        }
    }
}
