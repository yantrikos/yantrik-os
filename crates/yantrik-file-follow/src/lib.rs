//! The folder an open file lives in, watched, so that moving the file does not strand it.
//!
//! Seen on 22 Sep 2026: with a document open in yDoc, Files moved it into a new folder. The disk
//! agreed and the window did not — it went on saying `saved · <the old path>`, and everything
//! downstream of that line was then wrong. Save had no file to write into, Save As refused the
//! new path because something was already there, and Open silently dropped the unsaved edit. The
//! path in the window has to be able to change without a person retyping it, and inotify on the
//! folder is how the window finds out that it has. The Text Editor was found (#86) to strand a
//! moved file exactly the same way, so what #84 built for yDoc lives here and both apps use it.
//!
//! The wire and the decisions are together here because the decisions are pure enough to test
//! with a temporary directory and no filesystem event in sight: [`follow_rename`] answers where
//! a file is after a rename the folder reported, and [`moved_to`] goes looking on disk when the
//! event said only that the file left. What stays in each app is what a save does about it — the
//! refusal that names where the bytes went, and the write itself.

use notify::{
    event::{EventKind, ModifyKind, RenameMode},
    Event, RecursiveMode, Watcher,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{mpsc::Sender, Arc, Mutex},
};

/// What the folder said about the open document.
#[derive(Debug)]
pub enum Move {
    /// The rename named both of its ends, so this is where the document is now. An in-place
    /// rename, or a move into a folder that happens to be watched too.
    To(PathBuf),
    /// The document left the folder and the event did not say where to — which is the ordinary
    /// case, because the folder it went into is not the folder being watched. Somebody has to go
    /// and look: [`moved_to`].
    Away,
}

/// The watch on one folder, re-pointed as the document moves between them.
pub struct Watch {
    /// `None` when the kernel would not give us a watcher. Not fatal: without it the document
    /// simply does not follow a move, which is where this app was before.
    watcher: Option<notify::RecommendedWatcher>,
    /// The folder being watched, so re-pointing at the same one is a no-op.
    folder: Option<PathBuf>,
    /// Where the document is, as the watcher's own thread sees it. The UI thread owns the
    /// document; this is the single field the watcher needs and the only one it is given.
    file: Arc<Mutex<Option<PathBuf>>>,
}

impl Watch {
    /// Start watching nothing.
    ///
    /// `wake` runs on the watcher's thread every time something is put on `events`, and its whole
    /// job is to get the UI thread to come and read them — the document cannot be touched from
    /// here. Same shape as the Files workbench's worker events in `yantrik-ui`.
    pub fn new(events: Sender<Move>, wake: impl Fn() + Send + 'static) -> Self {
        let file: Arc<Mutex<Option<PathBuf>>> = Arc::default();
        let mirror = file.clone();
        let handler = move |result: Result<Event, notify::Error>| {
            let Ok(event) = result else { return };
            let Some(current) = mirror.lock().ok().and_then(|held| (*held).clone()) else { return };
            let Some(moved) = read_event(&event, &current) else { return };
            if events.send(moved).is_ok() {
                wake();
            }
        };
        match notify::recommended_watcher(handler) {
            Ok(watcher) => Self { watcher: Some(watcher), folder: None, file },
            Err(e) => {
                tracing::warn!(error = %e, "cannot watch the folder the open file is in");
                Self { watcher: None, folder: None, file }
            }
        }
    }

    /// Watch the folder `file` lives in, and stop watching whatever was being watched before.
    ///
    /// Called from `paint`, which runs after every change to the document, so the watch cannot
    /// drift away from the file it is supposed to be about.
    pub fn point_at(&mut self, file: Option<&Path>) {
        if let Ok(mut held) = self.file.lock() {
            *held = file.map(|f| f.to_path_buf());
        }
        let folder = file.and_then(|f| f.parent()).map(|f| f.to_path_buf());
        if folder == self.folder {
            return;
        }
        let Some(watcher) = self.watcher.as_mut() else { return };
        if let Some(old) = self.folder.take() {
            let _ = watcher.unwatch(&old);
        }
        let Some(new) = folder else { return };
        // Not recursive. The folder a document is in is the folder that reports it being moved
        // out, and a recursive watch rooted at somebody's Documents is a watch on everything
        // they own.
        match watcher.watch(&new, RecursiveMode::NonRecursive) {
            Ok(()) => self.folder = Some(new),
            Err(e) => tracing::warn!(folder = %new.display(), error = %e, "folder is not watchable"),
        }
    }
}

/// What one directory event means for the file at `current`.
///
/// `None` for the events that are about something else, which is nearly all of them — including
/// the editor's own temporary file being renamed over the document on every single save.
pub fn read_event(event: &Event, current: &Path) -> Option<Move> {
    match &event.kind {
        // Both halves of the rename, paired by inotify on their cookie: the folder told us where
        // the file went.
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
            let from = event.paths.first()?;
            let to = event.paths.get(1)?;
            follow_rename(current, from, to).map(Move::To)
        }
        // Only the leaving half, or a removal. The file went somewhere this watch cannot see, or
        // it is really gone; which of those it is takes a look at the disk, and that is the
        // caller's job. Checked against the disk first, because the path named may be a file
        // that merely shares a prefix with ours.
        EventKind::Modify(ModifyKind::Name(RenameMode::From | RenameMode::Any))
        | EventKind::Remove(_) => {
            let ours = event
                .paths
                .iter()
                .any(|p| p.as_path() == current || current.starts_with(p));
            (ours && fs::symlink_metadata(current).is_err()).then_some(Move::Away)
        }
        _ => None,
    }
}

/// Everything the folder has said so far, as one decision.
///
/// inotify reports the leaving half of a rename before it reports the pair, so a rename inside the
/// watched folder arrives as "it is gone" and then as "here is where it went". Both are true and
/// only the second is worth acting on. Taking the batch together rather than one event at a time
/// is what stops an ordinary rename from first spending a directory walk hunting for a file that
/// is one event away from naming itself.
pub fn latest(moves: Vec<Move>) -> Option<Move> {
    let mut away = false;
    let mut to = None;
    for moved in moves {
        match moved {
            Move::To(path) => to = Some(path),
            Move::Away => away = true,
        }
    }
    to.map(Move::To).or(away.then_some(Move::Away))
}

// ── The decisions ───────────────────────────────────────────────────────────
//
// A file open in an editor is a file somebody else can move while it is open — Files does
// exactly that, with `rename`, which keeps the bytes and the inode and changes only the name.
// Nothing below watches anything; these are the two decisions the watcher makes, separated from
// the events that trigger them so they can be tested with a temporary directory and no inotify.

/// How far the search for a moved file is allowed to go.
///
/// A person moving a file in Files moves it near where it was: into a folder beside it, usually
/// one they have just made. That is what this looks for. The bounds are what stop "where did my
/// document go" from walking a whole home directory — four levels under the folder it used to
/// be in, and two thousand entries, whichever runs out first.
const SEARCH_DEPTH: usize = 4;
const SEARCH_ENTRIES: usize = 2000;

/// Where the file that used to be at `original` probably is now, or `None`.
///
/// The same name and exactly the bytes the document last agreed with, somewhere under the folder
/// it used to live in. Both halves matter: the name on its own would point at any file called
/// `notes.md`, and the bytes on their own would point at a backup copy under a different name.
/// `None` is an honest answer and the callers say so rather than guessing.
///
/// `read` is the calling app's own file reader, so what counts as "the same bytes" is bounded
/// the way that app bounds a document — yDoc and the Text Editor refuse different things, and
/// each gets its own refusal here rather than a shared lowest common denominator.
pub fn moved_to(
    original: &Path,
    baseline: &str,
    read: fn(&Path) -> Result<String, String>,
) -> Option<PathBuf> {
    let name = original.file_name()?;
    // The folder it lived in — or, when that folder is itself the thing that was moved, the one
    // above it. No further up than that: a search that climbs is a search with no bound.
    let start = original.parent().filter(|p| p.is_dir()).or_else(|| {
        original
            .parent()
            .and_then(|p| p.parent())
            .filter(|p| p.is_dir())
    })?;

    let mut queue = vec![(start.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    while let Some((folder, depth)) = queue.pop() {
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > SEARCH_ENTRIES {
                return None;
            }
            let path = entry.path();
            // symlink_metadata, so a link pointing back up the tree cannot turn this walk into
            // a loop: a symlinked folder is neither descended into nor read as a document.
            let Ok(meta) = fs::symlink_metadata(&path) else { continue };
            if meta.is_dir() {
                // Hidden folders are skipped. A `.git` or a `.cache` beside the document would
                // spend the whole entry budget on somewhere nobody moved a document to.
                let hidden = entry.file_name().to_string_lossy().starts_with('.');
                if !hidden && depth < SEARCH_DEPTH {
                    queue.push((path, depth + 1));
                }
            } else if meta.is_file()
                && entry.file_name() == name
                && meta.len() == baseline.len() as u64
                && path.as_path() != original
                && read(&path).ok().as_deref() == Some(baseline)
            {
                return Some(path);
            }
        }
    }
    None
}

/// Where an open file lives after a rename the folder around it reported.
///
/// Two cases, and the second is the one that catches people out: the file itself was renamed, or
/// a folder it sits inside was. Moving a folder in Files strands every document in it exactly as
/// thoroughly as moving one document does, and it is the same fix.
pub fn follow_rename(current: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    if current == from {
        return Some(to.to_path_buf());
    }
    current.strip_prefix(from).ok().map(|rest| to.join(rest))
}

#[cfg(test)]
mod tests {
    //! Real inotify, real renames, a real temporary directory.
    //!
    //! What an editor does about a move is decided by its own save path, on top of
    //! `follow_rename` and `moved_to` here; the decisions themselves are tested without a
    //! filesystem event in sight in `tests/document-core` and the Text Editor's own tests. What
    //! is left over is the wire, and the wire is the part that cannot be reasoned about: that
    //! the watch is pointed at the right folder, that the kernel's event reaches the channel,
    //! and that somebody else's file moving in the same folder wakes nobody.

    use super::*;
    use std::{sync::mpsc::Receiver, time::Duration};

    /// A private directory per test, removed however the test ends. On the real filesystem and
    /// not under a mount that fakes inotify, which is the whole point of these three.
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("file-follow-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("a scratch directory");
            Self(fs::canonicalize(&path).expect("a real path"))
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A watch pointed at `file`, and the channel its folder reports to.
    fn watching(file: &Path) -> (Watch, Receiver<Move>) {
        let (sender, inbox) = std::sync::mpsc::channel();
        let mut watch = Watch::new(sender, || {});
        watch.point_at(Some(file));
        (watch, inbox)
    }

    /// Everything the folder has to say about the change that just happened, as one decision.
    ///
    /// The quiet period is what the app gets for free by draining the channel in one go: the
    /// events of a single rename arrive together, and the app reads them together.
    fn settled(inbox: &Receiver<Move>) -> Option<Move> {
        let mut seen = Vec::new();
        if let Ok(first) = inbox.recv_timeout(Duration::from_secs(5)) {
            seen.push(first);
            while let Ok(rest) = inbox.recv_timeout(Duration::from_millis(250)) {
                seen.push(rest);
            }
        }
        latest(seen)
    }

    #[test]
    fn a_rename_inside_the_folder_arrives_as_the_new_path() {
        let dir = Dir::new("renamed");
        let file = dir.0.join("notes.md");
        fs::write(&file, "body
").unwrap();
        let (_watch, inbox) = watching(&file);

        let to = dir.0.join("pricing.md");
        fs::rename(&file, &to).unwrap();

        // inotify says "it left" before it says "and here it is"; the second is the answer.
        match settled(&inbox) {
            Some(Move::To(now)) => assert_eq!(now, to),
            other => panic!("expected the new path; got {other:?}"),
        }
    }

    #[test]
    fn a_move_into_a_folder_this_watch_cannot_see_says_only_that_it_went() {
        let dir = Dir::new("away");
        let file = dir.0.join("notes.md");
        fs::write(&file, "body
").unwrap();
        let archive = dir.0.join("archive");
        fs::create_dir(&archive).unwrap();
        let (_watch, inbox) = watching(&file);

        // What Files does with cut and paste, and the case the issue was filed about: the folder
        // it lands in is not the folder being watched, so only the leaving half is reported and
        // nothing in the event says where it went.
        fs::rename(&file, archive.join("notes.md")).unwrap();

        match settled(&inbox) {
            Some(Move::Away) => {}
            other => panic!("expected the file to be reported as gone; got {other:?}"),
        }
    }

    #[test]
    fn somebody_elses_file_moving_in_the_same_folder_is_not_this_document() {
        let dir = Dir::new("other");
        let file = dir.0.join("notes.md");
        fs::write(&file, "body
").unwrap();
        let other = dir.0.join("other.md");
        fs::write(&other, "not ours
").unwrap();
        let (_watch, inbox) = watching(&file);

        fs::rename(&other, dir.0.join("renamed.md")).unwrap();
        fs::remove_file(dir.0.join("renamed.md")).unwrap();

        let heard = inbox.recv_timeout(Duration::from_millis(1500));
        assert!(heard.is_err(), "nothing happened to our document; got {heard:?}");
    }

    #[test]
    fn a_batch_that_names_where_the_file_went_beats_the_half_that_only_says_it_left() {
        let to = PathBuf::from("/home/p/Documents/pricing.md");
        let reduced = latest(vec![Move::Away, Move::To(to.clone())]);
        assert!(matches!(reduced, Some(Move::To(p)) if p == to));
        assert!(matches!(latest(vec![Move::Away]), Some(Move::Away)));
        assert!(latest(vec![]).is_none());
    }
}
