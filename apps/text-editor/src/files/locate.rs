//! Where a file that was open went, once it is no longer where it was (#86).
//!
//! Files moves with `rename`, which keeps the bytes and the inode and changes only the name. So
//! the file is looked for near where it was, as `yantrik-file-follow` bounds the search: first by
//! where it is on the disk (the same device and inode) holding exactly the bytes the tab last
//! agreed with, which finds a rename to any name; then by the same name and the same bytes, which
//! finds a copy-and-delete across filesystems. Both halves of each test matter: an inode on its
//! own may have been given to a new file since, and the bytes on their own would point at a
//! backup copy. `None` is an honest answer, and the callers say so rather than guess.

use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use super::read::read_text;
use crate::tab::Inode;

/// The same bounds the shared search keeps: four levels under the folder, two thousand entries.
const DEPTH: usize = 4;
const ENTRIES: usize = 2000;

/// Where the file that was at `original`, holding `on_disk`, is now; `None` when it was not found.
pub fn locate(original: &Path, on_disk: &str, inode: Option<Inode>) -> Option<PathBuf> {
    if fs::symlink_metadata(original).is_ok() {
        return None;
    }
    inode
        .and_then(|inode| by_inode(original, on_disk, inode))
        .or_else(|| yantrik_file_follow::moved_to(original, on_disk, read_text))
}

/// A regular file with this inode and exactly these bytes, under the folder `original` was in
/// (or the one above, when that folder itself was moved). Hidden folders and links are passed by.
fn by_inode(original: &Path, on_disk: &str, inode: Inode) -> Option<PathBuf> {
    let start = original
        .parent()
        .filter(|p| p.is_dir())
        .or_else(|| original.parent().and_then(Path::parent).filter(|p| p.is_dir()))?;
    let mut queue = vec![(start.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    while let Some((folder, depth)) = queue.pop() {
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > ENTRIES {
                return None;
            }
            let path = entry.path();
            let Ok(meta) = fs::symlink_metadata(&path) else { continue };
            if meta.is_dir() {
                let hidden = entry.file_name().to_string_lossy().starts_with('.');
                if !hidden && depth < DEPTH {
                    queue.push((path, depth + 1));
                }
            } else if meta.is_file()
                && meta.dev() == inode.dev
                && meta.ino() == inode.ino
                && meta.len() == on_disk.len() as u64
                && read_text(&path).ok().as_deref() == Some(on_disk)
            {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::read::inode;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("yantrik-editor-locate-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        fs::canonicalize(&d).unwrap()
    }

    #[test]
    fn a_rename_to_another_name_is_found_by_where_it_is_on_the_disk() {
        let d = scratch("renamed");
        let from = d.join("notes.txt");
        fs::write(&from, "body\n").unwrap();
        let at = inode(&from);
        // A copy with the same bytes and name elsewhere is not it.
        fs::create_dir(d.join("backup")).unwrap();
        fs::write(d.join("backup/pricing.txt"), "body\n").unwrap();
        let to = d.join("pricing.txt");
        fs::rename(&from, &to).unwrap();
        assert_eq!(locate(&from, "body\n", at), Some(to));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_still_there_or_really_gone_is_not_guessed_at() {
        let d = scratch("gone");
        let path = d.join("draft.txt");
        fs::write(&path, "kept\n").unwrap();
        let at = inode(&path);
        assert_eq!(locate(&path, "kept\n", at), None, "it has not moved");
        fs::remove_file(&path).unwrap();
        fs::write(d.join("other.txt"), "different\n").unwrap();
        assert_eq!(locate(&path, "kept\n", at), None, "an inode given to another file is not it");
        let _ = fs::remove_dir_all(&d);
    }
}
