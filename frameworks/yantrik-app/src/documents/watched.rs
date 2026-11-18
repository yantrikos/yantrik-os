//! The files an app's open documents live in, as the housekeeping thread watches them.
//!
//! The core adds and removes entries (`fx.watch`, `fx.unwatch`) on the UI thread; the watch
//! thread looks at every entry each tick and reports what changed. Each entry remembers what was
//! last seen, so only a change is reported, and an entry added with the document's baseline does
//! not report the file it was just read from.
//!
//! A look is a `stat` first, and the bytes are read and hashed only when the stamp moved since
//! the last look (length, modification time, and on Unix the inode and change time), so an
//! unchanged file costs a `stat` a tick at any size. The trade: an edit that keeps every part of
//! the stamp is missed, as it is by every editor that watches this way. A newly added (or
//! re-armed) entry is always read once.
//!
//! Each sighting carries the arming it was taken under. The core drops one whose arming is no
//! longer current: a look queued behind a save's outcome, taken before the app re-armed. The
//! re-armed look reads the file afresh, so an outside edit still on disk is reported then.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use super::{Fingerprint, Observed};

/// What says a file may have changed since the last look: its length and modification time, and
/// on Unix its inode and change time too, so an atomic replace (a new inode) and a write whose
/// mtime was set back (`touch -r`, `cp -p`, `rsync -t`, caught by the change time) are still
/// seen. A same-length write in place within one clock tick is not: the kernel sets the mtime
/// and the change time from the same clock reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
    inode: u64,
    changed: (i64, i64),
}

impl Stamp {
    fn of(meta: &std::fs::Metadata) -> Stamp {
        #[cfg(unix)]
        let (inode, changed) = {
            use std::os::unix::fs::MetadataExt as _;
            (meta.ino(), (meta.ctime(), meta.ctime_nsec()))
        };
        #[cfg(not(unix))]
        let (inode, changed) = (0, (0, 0));
        Stamp { len: meta.len(), modified: meta.modified().ok(), inode, changed }
    }
}

/// What a look found.
enum Look {
    /// Not looked at this time: an error that is not "not found", so not a change.
    Skipped,
    /// Unchanged since the last look, by its stamp: not read.
    Unchanged,
    Seen(Observed, Option<Stamp>),
}

fn look(path: &Path, last: Option<Stamp>) -> Look {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Look::Seen(Observed::Gone, None),
        Err(_) => return Look::Skipped,
    };
    if !meta.is_file() {
        return Look::Seen(Observed::Gone, None);
    }
    let stamp = Stamp::of(&meta);
    if last == Some(stamp) {
        return Look::Unchanged;
    }
    match std::fs::read(path) {
        Ok(bytes) => Look::Seen(Observed::File(Fingerprint::of_bytes(&bytes)), Some(stamp)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Look::Seen(Observed::Gone, None),
        Err(_) => Look::Skipped,
    }
}

#[derive(Debug, Clone)]
struct Entry {
    path: PathBuf,
    seen: Option<Observed>,
    stamp: Option<Stamp>,
    /// Which arming this is: a look taken under an older arming is dropped.
    arming: u64,
}

/// Armings, numbered in the process.
static NEXT_ARMING: AtomicU64 = AtomicU64::new(1);

/// The table, shared by the core and the watch thread.
#[derive(Debug, Clone, Default)]
pub struct Watched(Arc<Mutex<BTreeMap<String, Entry>>>);

impl Watched {
    pub fn new() -> Watched {
        Watched::default()
    }

    fn table(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Entry>> {
        // A poisoned table is still a table: a panic elsewhere cannot stop the watch.
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Watches `path` under `key`. `seen` is what the app knows is there (the document's
    /// baseline), so only a change from it is reported; `None` reports the first look. Adding
    /// a key again re-arms it: the file is read at the next look whatever its stamp.
    pub fn add(&self, key: &str, path: PathBuf, seen: Option<Observed>) {
        let arming = NEXT_ARMING.fetch_add(1, Ordering::Relaxed);
        self.table().insert(key.to_string(), Entry { path, seen, stamp: None, arming });
    }

    pub fn remove(&self, key: &str) {
        self.table().remove(key);
    }

    /// The keys watched, with their paths.
    pub fn entries(&self) -> Vec<(String, PathBuf)> {
        self.table().iter().map(|(k, e)| (k.clone(), e.path.clone())).collect()
    }

    /// Whether `arming` is the one `key` is watched under now.
    pub fn is_current(&self, key: &str, arming: u64) -> bool {
        self.table().get(key).is_some_and(|e| e.arming == arming)
    }

    /// Looks at every file once and returns what changed, with the arming each look was taken
    /// under. The files are looked at outside the
    /// lock, so the UI thread never waits on a disk; an entry removed or re-armed meanwhile is
    /// left as the core set it.
    pub fn poll(&self) -> Vec<(String, Observed, u64)> {
        let snapshot: Vec<(String, PathBuf, Option<Stamp>, u64)> =
            self.table().iter().map(|(k, e)| (k.clone(), e.path.clone(), e.stamp, e.arming)).collect();
        let looks: Vec<(String, u64, Look)> = snapshot
            .into_iter()
            .map(|(key, path, last, arming)| {
                let found = look(&path, last);
                (key, arming, found)
            })
            .collect();
        let mut table = self.table();
        let mut changed = Vec::new();
        for (key, arming, found) in looks {
            let Look::Seen(seen, stamp) = found else { continue };
            let Some(entry) = table.get_mut(&key) else { continue };
            // Re-armed since the snapshot: this look is older than the entry.
            if entry.arming != arming {
                continue;
            }
            entry.stamp = stamp;
            if entry.seen.as_ref() == Some(&seen) {
                continue;
            }
            entry.seen = Some(seen.clone());
            changed.push((key, seen, arming));
        }
        changed
    }
}
