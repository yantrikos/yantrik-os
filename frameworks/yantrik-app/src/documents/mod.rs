//! Documents: what an editor has open, independent of the windows that show it
//! (design/notes-gate-2026-10-21.md §2).
//!
//! A **document** owns its content, the fingerprint of the file as this process last read or
//! wrote it (its baseline), and four small state machines: whether it is being saved, whether
//! the file changed outside, whether its draft is kept, and how a close asks about it. A
//! **view** shows one document in one window; two views on one file share one document, so their
//! content, dirty state, conflict and draft can never diverge. Tabs and windows are only ways to
//! hold views.
//!
//! Everything here is pure: no file is read and no thread is started. The app's jobs do the
//! I/O (through [`disk::Disk`], so a test can make any step fail), and report back with the
//! sequence number they were given, so a late outcome can never overwrite a newer state.

pub mod close;
pub mod disk;
pub mod draft;
pub mod files;
pub(crate) mod exchange;
pub mod outside;
pub mod save;
#[cfg(test)]
mod disk_tests;
#[cfg(test)]
mod tests;
pub mod watched;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::app::WindowId;
pub use crate::settings::Fingerprint;
pub use close::{Choice, CloseFlow, Closing};
pub use draft::{Candidate, DraftState, Envelope, Recovered};
pub use outside::{Observed, OutsideState};
pub use save::{BeginSave, Durability, Replaced, SaveError, SaveOutcome, SaveState, SaveTicket, Witnessed};
pub use watched::Watched;

/// A document, for the life of this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentId(pub u64);

/// One place a document is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewId(pub u64);

/// Where a document lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A file on disk.
    File(PathBuf),
    /// Not saved anywhere yet; the key names its draft across restarts.
    New(String),
}

impl Source {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Source::File(p) => Some(p),
            Source::New(_) => None,
        }
    }

    /// A name stable across restarts, for the document's draft and its watch: the file's path
    /// hashed (so no path ends up in a file name), or the new document's own key. SHA-256, cut
    /// to 128 bits: two open documents sharing a key would share a draft, and file names in a
    /// synced vault may be chosen by someone else, so the hash must resist a chosen collision.
    pub fn key(&self) -> String {
        match self {
            Source::File(p) => {
                use sha2::Digest as _;
                let digest = sha2::Sha256::digest(p.as_os_str().as_encoded_bytes());
                let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
                format!("doc-{hex}")
            }
            Source::New(key) => format!("new-{key}"),
        }
    }
}

/// One open document.
#[derive(Debug, Clone)]
pub struct Document<C> {
    pub id: DocumentId,
    pub source: Source,
    pub content: C,
    /// The file as this process last read or wrote it; `None` for a document never saved.
    pub baseline: Option<Fingerprint>,
    /// Bumped by every edit. A save, a draft and an outcome each carry the `seq` they were for.
    pub seq: u64,
    /// The newest `seq` known to be on disk.
    pub saved_seq: u64,
    pub save: SaveState,
    pub outside: OutsideState,
    pub draft: DraftState,
    /// A save was asked for while one was in flight: one follow-up is owed, never more.
    pub(crate) follow_up: bool,
    /// What the watch saw while a save was in flight, judged when the save's outcome lands.
    pub(crate) seen_while_saving: Option<Observed>,
    /// Our writes the watch may not have seen yet: a sighting of one is suppressed once
    /// (outside.rs). Cleared when the watch catches up with the baseline.
    pub(crate) ours: Vec<Fingerprint>,
    /// Bytes of ours a failed save may have left on disk (`Durability::ContentOnly`, `Unknown`).
    pub(crate) unsure: Option<Fingerprint>,
    /// An older write of ours was seen: the watch is to look again from the baseline.
    pub(crate) recheck: bool,
}

/// How many of our writes the watch may not have seen yet are remembered.
const REMEMBERED_WRITES: usize = 4;

impl<C> Document<C> {
    /// Edits not yet on disk.
    pub fn dirty(&self) -> bool {
        self.seq != self.saved_seq
    }

    /// Changes the content: the document is dirty until a save of this `seq` or a later one
    /// commits. Returns the new `seq`.
    pub fn edit(&mut self, change: impl FnOnce(&mut C)) -> u64 {
        change(&mut self.content);
        self.seq += 1;
        self.seq
    }

    /// What `fx.watch` takes for this document: its key, its file, and the baseline the watch
    /// starts from. `None` for a document with no file yet.
    pub fn watch_entry(&self) -> Option<(String, PathBuf, Option<Observed>)> {
        let path = self.source.path()?.to_path_buf();
        Some((self.source.key(), path, self.baseline.clone().map(Observed::File)))
    }

    pub(crate) fn remember_write(&mut self, written: Fingerprint) {
        self.ours.retain(|f| *f != written);
        self.ours.push(written);
        if self.ours.len() > REMEMBERED_WRITES {
            self.ours.remove(0);
        }
    }
}

/// One place a document is shown: its window, and nothing of the document's own state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    pub id: ViewId,
    pub window: WindowId,
    pub document: DocumentId,
}

/// Every document an app has open, and every view on them.
#[derive(Debug, Clone)]
pub struct Documents<C> {
    next_document: u64,
    next_view: u64,
    documents: BTreeMap<DocumentId, Document<C>>,
    views: BTreeMap<ViewId, View>,
}

impl<C> Default for Documents<C> {
    fn default() -> Self {
        Documents {
            next_document: 1,
            next_view: 1,
            documents: BTreeMap::new(),
            views: BTreeMap::new(),
        }
    }
}

impl<C> Documents<C> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a document, or returns the one already open from the same file, so a file shown
    /// twice is one document. `baseline` is the fingerprint of what was read (`None` when new).
    pub fn open(&mut self, source: Source, content: C, baseline: Option<Fingerprint>) -> DocumentId {
        if let Some(id) = self.find(&source) {
            return id;
        }
        let id = DocumentId(self.next_document);
        self.next_document += 1;
        self.documents.insert(
            id,
            Document {
                id,
                source,
                content,
                baseline,
                seq: 0,
                saved_seq: 0,
                save: SaveState::Idle,
                outside: OutsideState::Clean,
                draft: DraftState::None,
                follow_up: false,
                seen_while_saving: None,
                ours: Vec::new(),
                unsure: None,
                recheck: false,
            },
        );
        id
    }

    /// The open document from this source, if there is one.
    pub fn find(&self, source: &Source) -> Option<DocumentId> {
        self.documents.values().find(|d| d.source == *source).map(|d| d.id)
    }

    /// The open document whose watch or draft key this is.
    pub fn by_key(&self, key: &str) -> Option<DocumentId> {
        self.documents.values().find(|d| d.source.key() == key).map(|d| d.id)
    }

    pub fn get(&self, id: DocumentId) -> Option<&Document<C>> {
        self.documents.get(&id)
    }

    pub fn get_mut(&mut self, id: DocumentId) -> Option<&mut Document<C>> {
        self.documents.get_mut(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Document<C>> {
        self.documents.values()
    }

    /// Documents with edits not on disk.
    pub fn dirty(&self) -> impl Iterator<Item = &Document<C>> {
        self.documents.values().filter(|d| d.dirty())
    }

    /// Shows a document in a window.
    pub fn show(&mut self, document: DocumentId, window: WindowId) -> ViewId {
        let id = ViewId(self.next_view);
        self.next_view += 1;
        self.views.insert(id, View { id, window, document });
        id
    }

    pub fn view(&self, id: ViewId) -> Option<View> {
        self.views.get(&id).copied()
    }

    pub fn views_of(&self, document: DocumentId) -> impl Iterator<Item = View> + '_ {
        self.views.values().filter(move |v| v.document == document).copied()
    }

    pub fn views_in(&self, window: WindowId) -> impl Iterator<Item = View> + '_ {
        self.views.values().filter(move |v| v.window == window).copied()
    }

    /// Points a view at another document (the person picked another note in this window).
    pub fn retarget(&mut self, view: ViewId, document: DocumentId) {
        if let Some(v) = self.views.get_mut(&view) {
            v.document = document;
        }
    }

    /// Removes a view. Returns its document when no view shows it any more: the app then
    /// decides whether to close it (unwatch, drop it from the table).
    pub fn forget_view(&mut self, view: ViewId) -> Option<DocumentId> {
        let gone = self.views.remove(&view)?;
        let still_shown = self.views.values().any(|v| v.document == gone.document);
        (!still_shown).then_some(gone.document)
    }

    /// Removes a document and every view on it.
    pub fn close_document(&mut self, id: DocumentId) -> Option<Document<C>> {
        self.views.retain(|_, v| v.document != id);
        self.documents.remove(&id)
    }
}
