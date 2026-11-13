//! The folder Files has open, watched for changes made from outside it.
//!
//! The listing was read when the folder was opened and again only after the window's own
//! operations, so a file a Mind moved into or out of it, or one made by `touch` in a terminal, stayed
//! missing (or kept being listed after it was gone) until the person pressed refresh: a display of
//! files that are not there (VM 520, #1069).
//!
//! This watches the one folder Files is showing by its own modification time and inode, once a
//! second, from a thread of its own. A folder's time moves when an entry is added, removed or
//! renamed in it, which is every change to what the listing shows except a file being written in
//! place (its size changes; the folder's time does not): that one is seen at the next refresh, as
//! before. One `stat` a second needs no descriptor per folder opened, as inotify would.
//!
//! - **The baseline is the listing's.** The listing thread takes the folder's stamp *before* it reads
//!   the folder and hands it over with the listing (`watch`), so a change that lands while the
//!   listing runs, or before the watcher's first look, is a difference from that stamp and is
//!   reported: nothing is baked into the baseline.
//! - **The watch follows what is shown.** It is pointed at a folder when that folder's listing
//!   lands, and back at the folder on screen when a listing fails (a failed open, a missing
//!   folder), never at one that was only asked for.
//! - **A change that cannot be acted on yet is kept** (`wire::files`' `dirty` flag): during a
//!   listing or a job, or while a quiet refresh is already running, the window notes it and
//!   refreshes once more when that ends, rather than restarting a listing that a busy folder
//!   would then never finish.
//! - **A hung mount costs the watch one folder, not the session.** The `stat` runs on a short-lived
//!   thread with a timeout, and no new one starts while one is still stuck.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const POLL: Duration = Duration::from_secs(1);
/// How long a `stat` may take before the watch gives up on this look.
const STAT_LIMIT: Duration = Duration::from_millis(1500);

/// What the folder looked like: modification time in nanoseconds, and its inode (a folder swapped
/// for another of the same name is a change).
pub(crate) type Stamp = (i64, u64);

/// The folder to watch and the stamp to compare its first look against.
struct Watched {
    path: PathBuf,
    baseline: Option<Stamp>,
}

static WATCHED: Mutex<Option<Watched>> = Mutex::new(None);

/// Watch `path` (or nothing), comparing against `baseline`: the stamp taken before the listing of
/// that folder was read. `None` for the baseline means the first look is the baseline.
pub fn watch(path: Option<PathBuf>, baseline: Option<Stamp>) {
    *WATCHED.lock().unwrap_or_else(|e| e.into_inner()) = path.map(|path| Watched { path, baseline });
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

/// What to do with "the folder changed", given what Files is doing.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Not the folder on screen, or Trash: nothing to do.
    Ignore,
    /// The folder on screen, but a listing, a job or a quiet refresh is running: do it afterwards.
    Defer,
    /// Refresh quietly now.
    Now,
}

pub(crate) fn decide(
    open: &Path,
    changed: &Path,
    trash: bool,
    job_running: bool,
    loading: bool,
    quiet_in_flight: bool,
) -> Decision {
    if trash || open != changed {
        Decision::Ignore
    } else if job_running || loading || quiet_in_flight {
        Decision::Defer
    } else {
        Decision::Now
    }
}

/// `stamp(path)`, given up on after [`STAT_LIMIT`]. `None` for a folder that cannot be read or did
/// not answer in time; `busy` is set while a look that timed out is still stuck, so there is never
/// more than one.
fn stamp_within(path: &Path, busy: &Arc<AtomicBool>) -> Option<Stamp> {
    if busy.swap(true, Ordering::AcqRel) {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    let (path, flag) = (path.to_path_buf(), busy.clone());
    let spawned = std::thread::Builder::new().name("files-stat".into()).spawn(move || {
        let _ = tx.send(stamp(&path));
        flag.store(false, Ordering::Release);
    });
    if spawned.is_err() {
        busy.store(false, Ordering::Release);
        return None;
    }
    rx.recv_timeout(STAT_LIMIT).ok().flatten()
}

/// Start the watch once; `on_change` is told the folder's path when it changes.
pub fn start(on_change: impl Fn(String) + Send + 'static) {
    let _ = std::thread::Builder::new().name("files-watch".into()).spawn(move || {
        let busy = Arc::new(AtomicBool::new(false));
        // What was installed last, and the stamp the next look is compared against.
        let mut given: Option<(PathBuf, Option<Stamp>)> = None;
        let mut known: Option<Stamp> = None;
        loop {
            std::thread::sleep(POLL);
            let current = WATCHED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|w| (w.path.clone(), w.baseline));
            let Some((path, baseline)) = current else {
                given = None;
                continue;
            };
            // A new watch (a listing landed, or the screen's folder was put back) starts from the
            // listing's stamp, or from the first look when there is none.
            if given.as_ref() != Some(&(path.clone(), baseline)) {
                given = Some((path.clone(), baseline));
                known = baseline;
            }
            let now = stamp_within(&path, &busy);
            if changed(known, now) {
                on_change(path.to_string_lossy().into_owned());
            }
            // Only a look that answered moves the stamp: a stuck mount keeps the last known one.
            if now.is_some() {
                known = now;
            }
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

    /// The decision the window makes when the watcher reports a change.
    #[test]
    fn a_change_is_acted_on_only_for_the_open_folder_and_kept_when_busy() {
        let (a, b) = (Path::new("/home/p/a"), Path::new("/home/p/b"));
        use Decision::*;
        assert_eq!(decide(a, a, false, false, false, false), Now);
        assert_eq!(decide(a, b, false, false, false, false), Ignore, "another folder");
        assert_eq!(decide(a, a, true, false, false, false), Ignore, "Trash");
        assert_eq!(decide(a, a, false, true, false, false), Defer, "a job is running");
        assert_eq!(decide(a, a, false, false, true, false), Defer, "a listing is running");
        assert_eq!(decide(a, a, false, false, false, true), Defer, "a quiet refresh is already running");
        assert_eq!(decide(a, b, false, true, true, true), Ignore, "busy or not, not this folder");
    }

    /// M1: the listing's stamp, taken before it read the folder, is the baseline, so a change that
    /// arrives while it runs is a difference from that stamp, not from a later first look.
    #[test]
    fn a_change_during_the_listing_is_a_difference_from_the_stamp_taken_before_it() {
        let dir = std::env::temp_dir().join(format!("yantrik-files-watch-base-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let before_listing = stamp(&dir);
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("arrived-during-the-listing"), "x").unwrap();
        let first_look = stamp(&dir);
        assert!(changed(before_listing, first_look), "reported, not baked into the baseline");
        assert!(!changed(first_look, first_look));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The window's side of it, read from the source: a change goes through `decide`, a deferred one
    /// sets `dirty` and is run when the listing or job ends, and the watch is pointed from the
    /// listing that landed (not from the request), back at the shown folder when one fails.
    #[test]
    fn the_window_wires_the_watch_as_described() {
        let src = include_str!("files.rs");
        let src = &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
        assert!(src.contains("Decision::Defer => s.dirty = true"));
        assert!(src.contains("Decision::Now => s.quiet_refresh(&u)"));
        assert!(src.contains("std::mem::take(&mut s.dirty)"), "a deferred change is run afterwards");
        assert!(src.contains("super::files_watch::watch((!trash).then(|| fsview::expand_home(&path)), stamp)"));
        assert!(!src.contains("files_watch::watch((!trash).then(|| fsview::expand_home(&path)));"), "load() does not point the watch at a folder that was only asked for");
        assert!(src.contains("stamp = super::files_watch::stamp(&dir)"), "the baseline is taken before the folder is read");
    }

    #[test]
    fn a_stat_that_does_not_answer_is_given_up_on_and_only_one_is_stuck_at_a_time() {
        let busy = Arc::new(AtomicBool::new(false));
        let dir = std::env::temp_dir();
        assert!(stamp_within(&dir, &busy).is_some());
        assert!(!busy.load(Ordering::Acquire), "free again after an answer");
        busy.store(true, Ordering::Release);
        assert!(stamp_within(&dir, &busy).is_none(), "one is already stuck: no second look");
    }
}
