//! The person's live config, kept when "Use Yantrik models" is about to replace it and a backup
//! it did not make is already beside it (#1201).
//!
//! A `<name>.before-yantrik` that no marker names is adopted as "the original" (a crash after it was
//! written, or a by-hand rollback). If the live file then differs from it, the live file is the
//! person's newer work: it is copied to `<name>.yantrik-replaced-<unix time>`, created exclusively
//! (never over an earlier copy), with the original's mode and owner, because it can hold an API key.

use std::path::{Path, PathBuf};

use crate::private_file::{self, Publish};

const INFIX: &str = ".yantrik-replaced-";

/// Copy the live file at `path` to a new `.yantrik-replaced-*` file beside it and return its path.
/// Fails, rather than overwriting, if every name tried is taken.
pub(super) fn keep_live(path: &Path, live: &[u8]) -> Result<PathBuf, String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let base = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    for n in 0..1000u32 {
        let suffix = if n == 0 { String::new() } else { format!("-{n}") };
        let copy = path.with_file_name(format!("{base}{INFIX}{now}{suffix}"));
        if std::fs::symlink_metadata(&copy).is_ok() {
            continue;
        }
        // Created at 600 and hard-linked into place: fails if the name appeared meanwhile.
        match private_file::write(&copy, live, Publish::CreateOnly, || Ok(())) {
            Ok(()) => {
                match_original(path, &copy)?;
                tracing::info!(file = %path.display(), copy = %copy.display(), "kept the live config before replacing it");
                return Ok(copy);
            }
            Err(_) if std::fs::symlink_metadata(&copy).is_ok() => continue,
            Err(e) => return Err(e),
        }
    }
    Err(format!("could not find a free name to keep a copy of {}; nothing was changed", path.display()))
}

/// Give `copy` the mode and owner of `original`.
#[cfg(unix)]
fn match_original(original: &Path, copy: &Path) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let m = std::fs::metadata(original).map_err(|e| format!("could not read {}: {e}", original.display()))?;
    std::fs::set_permissions(copy, std::fs::Permissions::from_mode(m.mode() & 0o7777))
        .map_err(|e| format!("could not set the mode of {}: {e}", copy.display()))?;
    let own = std::fs::metadata(copy).map_err(|e| e.to_string())?;
    if (own.uid(), own.gid()) != (m.uid(), m.gid()) {
        std::os::unix::fs::chown(copy, Some(m.uid()), Some(m.gid()))
            .map_err(|e| format!("could not give {} the owner of {}: {e}", copy.display(), original.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn match_original(_original: &Path, _copy: &Path) -> Result<(), String> {
    Ok(())
}

/// Put a backup's content back over `path` and leave the backup where it is (it was not made by
/// this apply, so it is not this code's to delete). The file keeps the backup's mode.
pub(super) fn restore_keeping_backup(backup: &Path, path: &Path) -> Result<(), String> {
    let content = std::fs::read(backup).map_err(|e| format!("could not read {}: {e}", backup.display()))?;
    private_file::write(path, &content, Publish::Replace, || Ok(()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = std::fs::metadata(backup) {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(m.permissions().mode() & 0o7777));
        }
    }
    Ok(())
}

/// The sentence that names a kept copy, for Apply's and Revert's results.
pub(super) fn sentence(copy: &Path) -> String {
    format!("Your config from before this change is also kept at {}.", copy.display())
}
