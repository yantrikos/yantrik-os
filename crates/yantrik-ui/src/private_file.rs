//! Writing a file only its owner can read, whole or not at all, and never through a link.
//!
//! The one place this is done. Preferences (config_store) and a harness's settings given a
//! provider's key (provider_handoff) both write a file that holds something private in a
//! directory another process running as the person could also write, so both need the same
//! things:
//!
//! - **A new, uniquely named temporary file**, created exclusively at mode 600 — so a file or a
//!   link planted at a predictable name is never opened, truncated or followed, and nothing can
//!   read the content while it is being written.
//! - **The target checked first**: if it exists it must be a regular file with one link, so a
//!   link planted in its place is refused rather than written through.
//! - **Published in one step**: renamed over the file it replaces, or hard-linked into place when
//!   there should be no file yet — which fails, rather than clobbers, if one appeared meanwhile.
//! - **A last check** the caller supplies, run after the bytes are written and before they are
//!   published (config_store's "changed on disk while saving").

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// Whether the file may be written: absent, or a regular file with one link. A link, a
/// directory, a device or a hard-linked file is refused.
pub(crate) fn check_target(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(_) => Ok(()),
        Ok(m) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if !m.file_type().is_file() || m.nlink() != 1 {
                    return Err(format!("{} is linked or not a plain file, so it is left alone", path.display()));
                }
            }
            #[cfg(not(unix))]
            if !m.file_type().is_file() {
                return Err(format!("{} is not a plain file, so it is left alone", path.display()));
            }
            Ok(())
        }
    }
}

/// How the new file takes the old one's place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Publish {
    /// Rename over whatever is at the path (checked by [`check_target`] first).
    Replace,
    /// There must be no file at the path; fail if one appeared.
    CreateOnly,
}

/// Write `bytes` to `path` at mode 600. `before_publish` runs after the bytes are on disk and
/// before they replace anything; an `Err` from it abandons the write.
pub(crate) fn write(
    path: &Path,
    bytes: &[u8],
    publish: Publish,
    before_publish: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    check_target(path)?;
    let parent = path.parent().ok_or("no directory to write into")?;
    fs::create_dir_all(parent).map_err(|e| format!("could not make {}: {e}", parent.display()))?;
    let temp = temp_path(parent, path);
    let mut created = false;
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut f = options.open(&temp).map_err(|e| format!("could not write {}: {e}", path.display()))?;
        created = true;
        f.write_all(bytes).and_then(|_| f.sync_all()).map_err(|e| format!("could not write {}: {e}", path.display()))?;
        drop(f);
        before_publish()?;
        match publish {
            Publish::Replace => {
                check_target(path)?;
                fs::rename(&temp, path).map_err(|e| format!("could not write {}: {e}", path.display()))?;
            }
            Publish::CreateOnly => {
                fs::hard_link(&temp, path).map_err(|e| format!("could not write {}: {e}", path.display()))?;
            }
        }
        fs::File::open(parent).and_then(|d| d.sync_all()).map_err(|e| format!("written, but the directory did not sync: {e}"))
    })();
    if created {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn temp_path(parent: &Path, path: &Path) -> PathBuf {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    parent.join(format!(".{name}-{}-{}.tmp", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("private-file-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_new_file_is_600_and_whole() {
        let d = dir("new");
        let p = d.join("a.json");
        write(&p, b"{}", Publish::Replace, || Ok(())).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"{}");
        assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1, "no temporary file left behind");
    }

    #[test]
    fn a_link_in_the_files_place_is_refused_and_its_target_untouched() {
        let d = dir("link");
        let target = d.join("elsewhere");
        fs::write(&target, b"theirs").unwrap();
        let p = d.join("a.json");
        std::os::unix::fs::symlink(&target, &p).unwrap();
        assert!(write(&p, b"secret", Publish::Replace, || Ok(())).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"theirs");
    }

    #[test]
    fn create_only_never_clobbers_a_file_that_appeared() {
        let d = dir("create-only");
        let p = d.join("a.before");
        fs::write(&p, b"original").unwrap();
        assert!(write(&p, b"second", Publish::CreateOnly, || Ok(())).is_err());
        assert_eq!(fs::read(&p).unwrap(), b"original");
    }

    #[test]
    fn a_failed_last_check_publishes_nothing() {
        let d = dir("check");
        let p = d.join("a.json");
        fs::write(&p, b"before").unwrap();
        assert!(write(&p, b"after", Publish::Replace, || Err("changed".into())).is_err());
        assert_eq!(fs::read(&p).unwrap(), b"before");
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1);
    }
}
