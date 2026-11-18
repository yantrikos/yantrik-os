//! How the library puts bytes on disk: whole or not at all, readable by the person alone.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

/// Writes `bytes` to `path`, whole or not at all, readable by the person alone, making the folder
/// if it is missing. For a spec or a build, which make their own game's folder.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    yantrik_foundation::json_store::write_atomically(path, bytes)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// [`write_private`] into a folder that must already be there. A verdict or a screenshot lands
/// beside a game's spec some minutes after it was asked for; if the game was deleted meanwhile,
/// the folder is gone, and making it again would leave a folder with no spec in it that the list
/// cannot show and `delete` cannot reach, but that still keeps the title from being used again.
pub fn write_in_place(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let folder = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(folder) = folder {
        if !folder.is_dir() {
            return Err(format!(
                "{} is no longer in the library (deleted or moved while this ran), so nothing was written",
                folder.file_name().and_then(|n| n.to_str()).unwrap_or("the game")
            ));
        }
    }
    write_private(path, bytes)
}

/// Makes `folder` (and what is missing above it) the person's alone: 0700 on Unix.
pub fn private_folder(folder: &Path) -> std::io::Result<()> {
    fs::create_dir_all(folder)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(folder, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Which build of a game a file is: its modification time and size, or nothing when it is absent.
/// A verdict carries the stamp of the build it judged, and is dropped if the build has changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp(Option<(SystemTime, u64)>);

pub fn stamp_of(path: &Path) -> Stamp {
    Stamp(fs::metadata(path).ok().and_then(|m| m.modified().ok().map(|t| (t, m.len()))))
}
