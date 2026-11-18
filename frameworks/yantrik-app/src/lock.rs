//! An exclusive lock on a file, held for as long as its guard lives and released by the kernel
//! when the process dies.
//!
//! This is what makes "one copy" atomic. A pid file alone is check-then-create: two launches in
//! the same instant both find no live pid, and the second unlinks the first's fresh file before
//! creating its own, so both run. `flock` has no gap. It is also released on any death, a crash
//! or a `kill -9` included, so a stale lock never keeps a person out of an app.
//!
//! The file is opened without following a link and created private (0600), because it sits in a
//! directory whose contents decide whether a process runs.
//!
//! On a local filesystem a second `hold` of the same file is refused even within one process. On
//! NFS (and CIFS with some mount options) Linux turns `flock` into a POSIX lock, which never
//! conflicts within a process and is released when any descriptor of the file is closed: there it
//! excludes other processes only, so a caller must not rely on an in-process refusal.

use std::fs::File;
use std::path::{Path, PathBuf};

/// Held while it lives; the lock goes with it (and with the process).
#[derive(Debug)]
#[must_use = "the lock is released when this is dropped; bind it for as long as it must be held"]
pub struct FileLock {
    _file: File,
    path: PathBuf,
}

impl FileLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Why a lock was not taken.
#[derive(Debug)]
pub enum LockError {
    /// Another open of the file holds it: another process, or another guard in this one.
    Held,
    /// The file could not be opened or locked at all (a link at the name, a permission).
    Io(std::io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Held => write!(f, "held by another copy"),
            LockError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LockError::Held => None,
            LockError::Io(e) => Some(e),
        }
    }
}

/// Takes the lock at `path` without waiting, creating the file if it is absent.
#[cfg(unix)]
pub fn hold(path: &Path) -> Result<FileLock, LockError> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(LockError::Io)?;
    // SAFETY: the descriptor is open and owned by `file` for the whole call.
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if locked != 0 {
        let error = std::io::Error::last_os_error();
        return Err(if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
            LockError::Held
        } else {
            LockError::Io(error)
        });
    }
    Ok(FileLock { _file: file, path: path.to_path_buf() })
}

/// Windows development builds: no lock at all (the file is opened shared, so a second `hold`
/// succeeds and `Held` is never returned). The instance's pid file is what guards there.
#[cfg(not(unix))]
pub fn hold(path: &Path) -> Result<FileLock, LockError> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(LockError::Io)?;
    Ok(FileLock { _file: file, path: path.to_path_buf() })
}

/// For tests: whether `f` holds within a second. A lock dropped in a test can stay held for a
/// moment by a child another test forked just then: the child shares the open file until it
/// starts its program, when `O_CLOEXEC` closes it. A real app is not affected; a test must wait.
#[cfg(all(test, unix))]
pub(crate) fn soon(mut f: impl FnMut() -> bool) -> bool {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        if f() {
            return true;
        }
        if std::time::Instant::now() > until {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("yantrik-lock-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_second_hold_is_refused_until_the_first_is_dropped() {
        let d = dir("twice");
        let path = d.join("app.lock");
        let first = hold(&path).expect("free");
        assert!(matches!(hold(&path), Err(LockError::Held)), "atomic: no gap between check and take");
        drop(first);
        assert!(soon(|| hold(&path).is_ok()), "released with its guard");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_lock_file_is_private_and_a_link_at_its_name_is_refused() {
        let d = dir("private");
        let path = d.join("app.lock");
        let held = hold(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        drop(held);
        let elsewhere = d.join("elsewhere");
        std::fs::write(&elsewhere, "keep").unwrap();
        let link = d.join("linked.lock");
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
        assert!(matches!(hold(&link), Err(LockError::Io(_))));
        assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "keep");
        let _ = std::fs::remove_dir_all(&d);
    }
}
