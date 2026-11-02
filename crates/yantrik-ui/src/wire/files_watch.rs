//! The folder Files has open, watched for changes made from outside it.
//!
//! The listing was read when the folder was opened and again only after the window's own
//! operations, so a file a Mind moved into or out of it, or one made by `touch` in a terminal, stayed
//! missing (or kept being listed after it was gone) until the person pressed refresh: a display of
//! files that are not there (VM 520, #1069).
//!
//! This watches the one open folder by its own modification time and inode, once a second, from a
//! thread of its own so a slow or hung mount can stall nothing but the watch. A folder's time moves
//! when an entry is added, removed or renamed in it, which is every change to what the listing shows
//! except a file being written in place (its size changes; the folder's time does not): that one is
//! seen at the next refresh, as before. One `stat` a second is not what inotify spares, and a watch
//! per folder would need a descriptor for each one opened; this needs none. The refresh it asks for
//! keeps the person's selection (`wire::files`).

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(1);

/// The folder to watch: the open one, or none (Trash, or nothing open).
static WATCHED: Mutex<Option<PathBuf>> = Mutex::new(None);

/// What the folder looked like: modification time in nanoseconds, and its inode (a folder swapped
/// for another of the same name is a change).
pub(crate) type Stamp = (i64, u64);

pub fn watch(path: Option<PathBuf>) {
    *WATCHED.lock().unwrap_or_else(|e| e.into_inner()) = path;
}

pub(crate) fn stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.mtime() * 1_000_000_000 + m.mtime_nsec(), m.ino()))
}

/// Did the folder change between two looks at it? A first look, or a folder that could not be read
/// now, is not a change: nothing is known to compare.
pub(crate) fn changed(before: Option<Stamp>, now: Option<Stamp>) -> bool {
    matches!((before, now), (Some(a), Some(b)) if a != b)
}

/// Start the watch once; `on_change` is told the folder's path when it changes.
pub fn start(on_change: impl Fn(String) + Send + 'static) {
    let _ = std::thread::Builder::new().name("files-watch".into()).spawn(move || {
        let mut seen: Option<(PathBuf, Option<Stamp>)> = None;
        loop {
            std::thread::sleep(POLL);
            let Some(path) = WATCHED.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
                seen = None;
                continue;
            };
            let now = stamp(&path);
            match &seen {
                Some((was, before)) if *was == path => {
                    if changed(*before, now) {
                        on_change(path.to_string_lossy().into_owned());
                    }
                }
                _ => {}
            }
            seen = Some((path, now));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_that_gained_or_lost_an_entry_has_changed_and_one_left_alone_has_not() {
        let dir = std::env::temp_dir().join(format!("yantrik-files-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = stamp(&dir);
        assert!(!changed(first, stamp(&dir)), "nothing happened");
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("touched.txt"), "x").unwrap();
        let after_add = stamp(&dir);
        assert!(changed(first, after_add), "a file made in it, as `touch` does");
        std::thread::sleep(Duration::from_millis(20));
        std::fs::rename(dir.join("touched.txt"), dir.join("renamed.txt")).unwrap();
        let after_rename = stamp(&dir);
        assert!(changed(after_add, after_rename), "a rename, as a move out of or into it is");
        std::thread::sleep(Duration::from_millis(20));
        std::fs::remove_file(dir.join("renamed.txt")).unwrap();
        assert!(changed(after_rename, stamp(&dir)), "a file removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_known_is_not_a_change() {
        assert!(!changed(None, Some((1, 1))), "the first look");
        assert!(!changed(Some((1, 1)), None), "a folder that cannot be read now");
        assert!(!changed(None, None));
        assert!(changed(Some((1, 1)), Some((1, 2))), "a different folder under the same name");
    }
}
