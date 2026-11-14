//! The person's live config, kept when "Use Yantrik models" is about to replace it and a backup
//! it did not make is already beside it (#1201).
//!
//! A `<name>.before-yantrik` that no marker names is adopted as "the original" (a crash after it was
//! written, or a by-hand rollback). If the live file then differs from it, the live file is the
//! person's newer work: it is copied to `<name>.yantrik-replaced-<unix time>`, created exclusively
//! (never over an earlier copy), with the original's mode and owner, because it can hold an API key.

use std::path::{Path, PathBuf};

use super::Touched;
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
                if let Err(e) = match_original(path, &copy) {
                    // A copy that cannot get the original's mode is not left at 600 for every retry to add to.
                    let _ = std::fs::remove_file(&copy);
                    return Err(e);
                }
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

/// Put a backup's content back over `path` and leave the backup where it is for the caller to set
/// aside (it was not made by this apply, so it is not this code's to delete). The file keeps the
/// backup's mode.
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

/// Move an adopted backup to `<backup>.found-<unix time>` (exclusive, never over another), so the next
/// "Use Yantrik models" does not adopt it again and Revert does not keep going back to an older file.
/// Hard-linked, then the old name removed: nothing is deleted, and a name taken meanwhile is not used.
pub(super) fn set_aside(backup: &Path) -> Result<PathBuf, String> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let base = backup.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    for n in 0..1000u32 {
        let suffix = if n == 0 { String::new() } else { format!("-{n}") };
        let to = backup.with_file_name(format!("{base}.found-{now}{suffix}"));
        match std::fs::hard_link(backup, &to) {
            Ok(()) => {
                std::fs::remove_file(backup).map_err(|e| format!("could not set {} aside: {e}", backup.display()))?;
                return Ok(to);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("could not set {} aside: {e}", backup.display())),
        }
    }
    Err(format!("could not find a free name to set {} aside", backup.display()))
}

fn list(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
}

/// Apply's words about one file whose live version was kept. Paths only: never a key.
pub(super) fn apply_sentence(t: &Touched) -> String {
    let older = if t.adopted { " An older backup was already beside it, and Revert will put that one back." } else { "" };
    format!("Your version of {} that this replaced is kept at {}.{older}", t.path.display(), list(&t.replaced))
}

/// Revert's words about one file: an older backup that was put back (and set aside), and the copies
/// of the person's own version that are kept.
pub(super) fn revert_sentence(file: &Path, found: Option<&Path>, copies: &[PathBuf]) -> Option<String> {
    let mut said = Vec::new();
    if let Some(f) = found {
        said.push(format!("Put back an older copy of {} that was already beside it (set aside, not deleted, at {}).", file.display(), f.display()));
    }
    if !copies.is_empty() {
        said.push(format!("Your own version of {} that this replaced is kept at {}.", file.display(), list(copies)));
    }
    (!said.is_empty()).then(|| said.join(" "))
}
