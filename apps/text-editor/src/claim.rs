//! One Text Editor at a time, with the builds before the framework too.
//!
//! The framework's instance claim keeps one copy per app and profile. The builds before it took
//! a lock of their own, `$XDG_RUNTIME_DIR/yantrik-editor.lock`, and both builds keep drafts in the
//! same state folder, so while an old build is still running (an update applied, not yet
//! restarted) a new one must not start beside it. This copy holds that lock for the life of its
//! model; a copy that finds it held asks the running one to show its window and stops.
//!
//! A lock that cannot be taken at all (no runtime directory) guards nothing and stops nothing.

use std::path::{Path, PathBuf};

use yantrik_app::lock::{self, FileLock, LockError};

/// Said when another copy holds the lock.
pub const ALREADY_RUNNING: &str = "Text Editor is already running.";

/// The lock file's name in the runtime directory, as the builds before the framework named it.
pub const LEGACY_NAME: &str = "yantrik-editor.lock";

/// The lock, held while the model lives.
#[derive(Debug)]
pub struct Claim {
    _legacy: Option<FileLock>,
}

/// Another copy holds the lock.
#[derive(Debug, PartialEq, Eq)]
pub struct Held;

/// Takes the lock at `legacy`, when there is a runtime directory to hold it in.
pub fn hold(legacy: Option<&Path>) -> Result<Claim, Held> {
    let held = match legacy.map(lock::hold) {
        None => None,
        Some(Ok(held)) => Some(held),
        Some(Err(LockError::Held)) => return Err(Held),
        Some(Err(LockError::Io(e))) => {
            tracing::warn!(error = %e, "the editor's lock could not be taken; it guards nothing");
            None
        }
    };
    Ok(Claim { _legacy: held })
}

/// `$XDG_RUNTIME_DIR/yantrik-editor.lock`; `None` without a runtime directory.
pub fn legacy_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join(LEGACY_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_builds_lock_keeps_a_second_copy_out_until_it_is_let_go() {
        let d = std::env::temp_dir().join(format!("yantrik-editor-claim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join(LEGACY_NAME);
        let first = hold(Some(&path)).expect("free");
        assert_eq!(hold(Some(&path)).unwrap_err(), Held);
        drop(first);
        assert!(hold(Some(&path)).is_ok(), "let go with the model");
        assert!(hold(None).is_ok(), "no runtime directory stops nothing");
        let _ = std::fs::remove_dir_all(&d);
    }
}
