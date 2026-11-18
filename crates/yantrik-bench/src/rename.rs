//! The one rename that never replaces what is at the destination.
//!
//! The archive, Undo and Files' own move and paste (`yantrik-ui`'s `fileops`) all need "move this
//! there, or fail if something is there already", and a check followed by a rename is two steps
//! with a window between them. This is `renameat2(RENAME_NOREPLACE)`, in one place.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// A rename that fails with `AlreadyExists` instead of replacing what is at `to`.
pub fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    let (a, b) = (c_path(from)?, c_path(to)?);
    // SAFETY: both are NUL-terminated strings that outlive the call.
    let rc = unsafe { libc::renameat2(libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), libc::RENAME_NOREPLACE) };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        // The file system cannot do an atomic no-replace rename (some network and FUSE ones):
        // check first, and say so, since the check and the move are two steps there.
        Some(libc::EINVAL) | Some(libc::ENOSYS) | Some(libc::EOPNOTSUPP) => {
            tracing::debug!("renameat2 is not supported here; checking before the move");
            if std::fs::symlink_metadata(to).is_ok() {
                return Err(std::io::ErrorKind::AlreadyExists.into());
            }
            std::fs::rename(from, to)
        }
        _ => Err(err),
    }
}

fn c_path(p: &Path) -> std::io::Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| std::io::ErrorKind::InvalidInput.into())
}

