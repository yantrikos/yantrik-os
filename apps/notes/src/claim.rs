//! One copy of Notes per library.
//!
//! The framework's instance claim keeps one copy per app and profile. A vault needs more: two
//! copies on one vault share its journal (`.recovery-v2/pending.json`), so one's save removes the
//! other's, and one starting while the other saves replays that journal as a duplicate note. So
//! Notes also holds, for the life of its model:
//!
//! - the lock every build before the framework took, `$XDG_RUNTIME_DIR/yantrik-notes.lock`, so an
//!   old build and this one keep each other out, whichever started first; it is named per profile
//!   (`yantrik-notes.<profile>.lock`, the default profile keeping the old name), because two
//!   profiles are two instances with two vaults and one must not lock the other out or summon the
//!   other's window;
//! - a lock inside the vault itself, `<vault>/.notes.lock`, so two profiles pointed at one vault
//!   (`YANTRIK_NOTES_DIR`) keep each other out too. The library lists only `.md` files, so it is
//!   never a note.
//!
//! A lock that cannot be taken at all (no runtime directory, a vault that cannot be made) guards
//! nothing and stops nothing: the library's own errors say what is wrong with it.

use std::path::{Path, PathBuf};

use yantrik_app::lock::{self, FileLock, LockError};
use yantrik_app::paths::DEFAULT_PROFILE;

/// Said when another copy holds this library.
pub const ALREADY_RUNNING: &str = "Notes is already running on this library.";

/// The lock file's name in the runtime directory, as the builds before the framework named it.
pub const LEGACY_NAME: &str = "yantrik-notes.lock";
/// The lock file's name inside the vault.
pub const VAULT_NAME: &str = ".notes.lock";

/// Both locks, held while the model lives.
#[derive(Debug)]
pub struct Claim {
    _legacy: Option<FileLock>,
    _vault: Option<FileLock>,
}

/// Another copy holds one of the locks.
#[derive(Debug, PartialEq, Eq)]
pub struct Held;

/// Takes the legacy lock at `legacy` (when there is a runtime directory to hold it in) and the
/// vault's own.
pub fn hold(legacy: Option<&Path>, vault: &Path) -> Result<Claim, Held> {
    let legacy = legacy.map(take).transpose()?.flatten();
    // The vault may not exist yet on a first start; the load makes it anyway.
    let _ = std::fs::create_dir_all(vault);
    let vault = take(&vault.join(VAULT_NAME))?;
    Ok(Claim { _legacy: legacy, _vault: vault })
}

fn take(path: &Path) -> Result<Option<FileLock>, Held> {
    match lock::hold(path) {
        Ok(held) => Ok(Some(held)),
        Err(LockError::Held) => Err(Held),
        Err(LockError::Io(e)) => {
            tracing::warn!(path = %path.display(), error = %e, "a library lock could not be taken; it guards nothing");
            Ok(None)
        }
    }
}

/// The legacy lock's name for a profile. The default profile keeps [`LEGACY_NAME`], so an old
/// build and this one keep each other out whichever started first; any other profile takes
/// `yantrik-notes.<profile>.lock`, the `<id>.<profile>` shape the framework's instance name and
/// control socket already use, so it is not kept out by the default profile's lock.
pub fn legacy_name(profile: &str) -> String {
    if profile == DEFAULT_PROFILE {
        LEGACY_NAME.to_string()
    } else {
        format!("yantrik-notes.{profile}.lock")
    }
}

/// `$XDG_RUNTIME_DIR/` with this profile's legacy lock, the path the old build locked; `None`
/// without a runtime directory (where no old build could have started either).
pub fn legacy_path(profile: &str) -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()).map(|d| PathBuf::from(d).join(legacy_name(profile)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("yantrik-notes-claim-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn either_lock_held_keeps_a_second_copy_out_until_it_is_let_go() {
        let d = scratch("both");
        let (legacy, vault) = (d.join(LEGACY_NAME), d.join("vault"));
        let first = hold(Some(&legacy), &vault).expect("free");
        assert_eq!(hold(Some(&legacy), &d.join("another vault")).unwrap_err(), Held, "the old build's lock");
        assert_eq!(hold(Some(&d.join("other.lock")), &vault).unwrap_err(), Held, "the vault's own lock");
        drop(first);
        assert!(hold(Some(&legacy), &vault).is_ok(), "let go with the model");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_legacy_lock_is_named_per_profile() {
        assert_eq!(legacy_name("default"), LEGACY_NAME, "the name every build before the framework used");
        assert_eq!(legacy_name("work"), "yantrik-notes.work.lock");
    }

    #[test]
    fn one_profiles_legacy_lock_leaves_another_profile_free() {
        let d = scratch("profiles");
        let (by_default, by_work, other_vault) =
            (d.join(legacy_name("default")), d.join(legacy_name("work")), d.join("other vault"));
        let vault = d.join("vault");

        let default = hold(Some(&by_default), &vault).expect("the default profile starts");
        assert!(hold(Some(&by_work), &other_vault).is_ok(), "another profile has a legacy lock of its own");
        drop(default);

        let work = hold(Some(&by_work), &vault).expect("the other way round");
        assert!(hold(Some(&by_default), &other_vault).is_ok(), "and the default profile is free too");
        drop(work);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_lock_that_cannot_be_taken_stops_nothing() {
        let d = scratch("unusable");
        std::fs::write(d.join("plain file"), "").unwrap();
        // A legacy path under a file cannot be opened at all: it guards nothing.
        assert!(hold(Some(&d.join("plain file").join(LEGACY_NAME)), &d.join("vault")).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }
}
