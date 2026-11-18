//! Deleting a game: it moves to the freedesktop Trash, where the desktop's Files app can bring it
//! back. That is why `delete` is graded recoverable on the control surface.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Local;

use super::Library;

impl Library {
    /// Move a game to the freedesktop Trash. Recoverable from Files, which is
    /// the whole reason this is not `fs::remove_dir_all`.
    pub fn delete_game(&self, title_or_slug: &str) -> Result<String, String> {
        let (slug, spec) = self.load_game_spec(title_or_slug)?;
        let dir = self.game_dir(&slug);
        let trash = self.trash_dirs();
        fs::create_dir_all(&trash.files).map_err(|e| format!("cannot reach the Trash: {e}"))?;
        fs::create_dir_all(&trash.info).map_err(|e| format!("cannot reach the Trash: {e}"))?;

        let target = unique_path(&trash.files.join(format!("yantrik-arcade-{slug}")));
        // rename is atomic on one filesystem; the copy fallback covers a library
        // that lives somewhere else (an override on /mnt/c, say).
        if fs::rename(&dir, &target).is_err() {
            copy_dir(&dir, &target)?;
            fs::remove_dir_all(&dir).map_err(|e| format!("cannot remove the original: {e}"))?;
        }
        let info = format!(
            "[Trash Info]\nPath={}\nDeletionDate={}\n",
            percent_encode(&dir.display().to_string()),
            Local::now().format("%Y-%m-%dT%H:%M:%S")
        );
        let info_path = trash
            .info
            .join(target.file_name().and_then(|n| n.to_str()).unwrap_or("game"));
        let _ = fs::write(info_path.with_extension("trashinfo"), info);
        Ok(format!("{:?} moved to the Trash", spec.title))
    }
}

struct TrashDirs {
    files: PathBuf,
    info: PathBuf,
}

impl Library {
    fn trash_dirs(&self) -> TrashDirs {
        // $XDG_DATA_HOME respected, per the freedesktop spec, with its default.
        let data = match &self.trash_override {
            Some(dir) => dir.clone(),
            None => std::env::var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
                    PathBuf::from(home).join(".local/share")
                }),
        };
        TrashDirs {
            files: data.join("Trash/files"),
            info: data.join("Trash/info"),
        }
    }
}

fn unique_path(base: &Path) -> PathBuf {
    let mut candidate = base.to_path_buf();
    let mut n = 2;
    while candidate.exists() {
        candidate = base.with_file_name(format!(
            "{}.{}",
            base.file_name().and_then(|f| f.to_str()).unwrap_or("game"),
            n
        ));
        n += 1;
    }
    candidate
}

/// The `Path=` value in a .trashinfo is percent-encoded, per the spec.
pub(super) fn percent_encode(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub(super) fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("cannot create {}: {e}", to.display()))?;
    for entry in fs::read_dir(from).map_err(|e| format!("cannot read {}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        // A link is copied as a link, never followed: one planted in a game's folder must not
        // pull whatever it points at into the Trash.
        let kind = fs::symlink_metadata(&src).map_err(|e| format!("cannot read {}: {e}", src.display()))?.file_type();
        if kind.is_symlink() {
            copy_link(&src, &dst)?;
        } else if kind.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            fs::copy(&src, &dst)
                .map_err(|e| format!("cannot copy {} to {}: {e}", src.display(), dst.display()))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_link(from: &Path, to: &Path) -> Result<(), String> {
    let target = fs::read_link(from).map_err(|e| format!("cannot read the link {}: {e}", from.display()))?;
    std::os::unix::fs::symlink(&target, to).map_err(|e| format!("cannot copy the link {}: {e}", from.display()))
}

#[cfg(not(unix))]
fn copy_link(from: &Path, _: &Path) -> Result<(), String> {
    Err(format!("cannot copy the link {} on this system", from.display()))
}
