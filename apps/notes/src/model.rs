//! What Notes is: the model every command changes and every render reads.
//!
//! The library is every note as the vault holds it. Each open note is a document in the
//! framework's `Documents` table, with a [`NoteFile`] beside it saying what its files hold; each
//! window shows one through its [`Pane`], which owns everything that belongs to the view and not
//! to the note (its undo history, its search, its dialog). The focused window's pane is
//! [`Model::pane`] (pane.rs).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;
use yantrik_app::app::WindowId;
use yantrik_app::documents::{CloseFlow, Document, DocumentId, Documents, Fingerprint, OutsideState};
use yantrik_app::intents::InvocationId;

use crate::claim::Claim;
use crate::command::Step;
use crate::note::{Note, NoteText};
use crate::notice::ScopedNotice;
use crate::pane::Pane;
use crate::recovery::Recovery;
use crate::store::Vault;

pub struct Model {
    pub vault: Vault,
    pub library: Vec<Note>,
    pub documents: Documents<NoteText>,
    /// What each open document's files hold, by document.
    pub files: BTreeMap<DocumentId, NoteFile>,
    /// The focused window's pane, where every command acts.
    pub pane: Pane,
    /// The other windows' panes, by window, until one is focused.
    pub parked: BTreeMap<WindowId, Pane>,
    /// The windows open, in the order the person was last in them, the one they are in first
    /// (pane.rs). Work done in another window for a moment does not move it.
    pub visited: Vec<WindowId>,
    /// The library has been read; until then nothing but `reload` touches it.
    pub ready: bool,
    /// The one store job in flight. The store does one thing at a time, as it always has.
    pub busy: Option<Work>,
    /// The window the store's job was started from: its landing is applied there.
    pub job_window: WindowId,
    /// What could not be done, in one line; also `describe.notice`.
    pub notice: String,
    /// The notice says something a person may not see while typing on (a save replaced another
    /// program's write): the next save that lands leaves it, and the next action takes it back.
    pub lasting_notice: bool,
    /// The window the notice is about, when it is about one window only (its close was refused
    /// while another window's prompt waits): drawn there and nowhere else. Kept with the sentence
    /// it was said with, so any later notice, which replaces the sentence, is every window's again.
    pub notice_in: Option<ScopedNotice>,
    /// An action waiting for the open note to be saved first.
    pub pending: Option<Pending>,
    /// A close or a quit asking about unsaved notes, or waiting on their saves.
    pub closing: Option<CloseFlow>,
    /// The window the close prompt asks in, while `closing` asks: the window asked to close, or
    /// the one a quit was asked from. It is drawn and answered there and nowhere else.
    pub close_asked_in: WindowId,
    /// An action that would leave a note whose file is gone, waiting on the person's answer
    /// (update/leaving.rs).
    pub leaving: Option<Pending>,
    /// Text kept from a run that ended before it was saved, offered back note by note.
    pub recovery: Recovery,
    /// This run's last landed write to each note, by file name: the fingerprint of its text. A
    /// note whose file still holds it is ours to keep if another program replaces it
    /// (update/follow.rs), however often it was closed, reopened or reloaded since.
    pub written: BTreeMap<String, Fingerprint>,
    /// Drafts whose text moved to another note, by the draft that took it over: each old draft is
    /// removed once the new one is on disk, or the note it moved to is saved (update/drafts.rs).
    pub handed_over: Vec<(String, String)>,
    /// The document, and its edit, whose draft could not be written: tried again at its next edit.
    pub draft_failed_at: Option<(DocumentId, u64)>,
    /// A call from the surface waiting for the work it started.
    pub asked: Option<Asked>,
    /// What the last call from the surface is answered with.
    pub answer: Value,
    /// This copy's hold on the library, let go with the model.
    _claim: Claim,
}

/// What an open note's files hold, beside its document.
#[derive(Debug, Clone)]
pub struct NoteFile {
    /// The note's file name in the vault.
    pub id: String,
    pub trash: bool,
    /// The note as its files hold it; `None` for a note not saved yet. Whether the note is unsaved
    /// is a comparison with this, as it always was.
    pub on_disk: Option<NoteText>,
    /// A sidecar of ours a failed save may have left on disk.
    pub meta_unsure: Option<String>,
    pub modified: u64,
    /// The edit `Model::keep_unsaved_dirty` made when the note's file was found gone, while no
    /// other edit has followed it: the file coming back undoes it (update/outside.rs).
    pub gone_at: Option<u64>,
}

/// One store job, by what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Load,
    /// A save of this document. Only one runs at a time, of any document.
    Save(DocumentId),
    Trash,
    Restore,
    Import,
    Export,
    /// An outside change resolved: the window's text kept as a note, the other version read.
    Resolve,
    /// The other version read, for a comparison.
    Compare,
    /// The open note read again: another program changed it, and nothing here was unsaved.
    Reread,
}

/// An action held until the open note is saved.
#[derive(Clone)]
pub struct Pending {
    pub step: Step,
    /// The window it was asked in, where it runs.
    pub window: WindowId,
    /// From the surface, where a save that fails refuses the call ("nothing else was done").
    pub refuse_if_unsaved: bool,
}

/// A call from the surface whose answer waits for its work.
#[derive(Debug, Clone)]
pub struct Asked {
    pub invocation: Option<InvocationId>,
    /// The window that was focused when it was asked: the answer is about that window's note.
    pub window: WindowId,
    pub expect: Expect,
    /// The save it waited on failed: the refusal.
    pub refused: Option<String>,
    /// The import, export or comparison it started landed.
    pub landed: bool,
    /// The note the window's text was kept as, when the call resolved an outside change.
    pub kept: Option<String>,
}

/// What the answer to a call from the surface is, once its work is done.
#[derive(Debug, Clone)]
pub enum Expect {
    /// The open note, read back off the disk.
    Written,
    /// The open note, refused unless the disk holds exactly it.
    Saved,
    Trashed { id: String, title: String, path: PathBuf },
    Restored { id: String },
    /// Unsaved text from before Notes closed, put back into its note.
    Recovered { id: String },
    /// Unsaved text from before Notes closed, dropped.
    Discarded { id: String },
    Copied { from: String },
    Reloaded,
    Imported { from: PathBuf },
    Exported { to: PathBuf, text: String },
    /// The file's version, taken into the window in place of the text that was there.
    TookTheirs { id: String },
    /// The window's text beside the file's.
    Compared,
}

/// The note the pane shows: its document and what its files hold.
pub struct Current<'a> {
    pub doc: &'a Document<NoteText>,
    pub file: &'a NoteFile,
}

impl Current<'_> {
    pub fn content(&self) -> &NoteText {
        &self.doc.content
    }

    pub fn text(&self) -> &str {
        &self.doc.content.text
    }

    pub fn title(&self) -> String {
        self.doc.content.title()
    }

    pub fn id(&self) -> &str {
        &self.file.id
    }

    pub fn trash(&self) -> bool {
        self.file.trash
    }

    /// What is known of changes made to the note's file outside Notes.
    pub fn outside(&self) -> &OutsideState {
        &self.doc.outside
    }

    /// Edits not on disk: the content is not what the files hold, or the files are gone (moved or
    /// deleted outside Notes), so the text is on no disk at all until the person keeps it.
    pub fn unsaved(&self) -> bool {
        if self.doc.outside == OutsideState::Deleted {
            return true;
        }
        let disk = self.file.on_disk.as_ref();
        disk.map(|d| d.text.as_str()) != Some(self.text())
            || disk.map(|d| d.meta.as_str()).unwrap_or("") != self.content().meta
    }

    /// Whether the person changed the note since its file was found gone, or before (its edits were
    /// unsaved then): what letting it go would drop is their changes, not only its text.
    pub fn edited_since_gone(&self) -> bool {
        self.file.gone_at != Some(self.doc.seq)
    }

    /// The note as the library lists it, with this view's unsaved content.
    pub fn as_note(&self) -> Note {
        Note {
            id: self.file.id.clone(),
            content: self.doc.content.clone(),
            modified: self.file.modified,
            trash: self.file.trash,
            stored: self.file.on_disk.is_some(),
        }
    }
}

impl Model {
    pub fn new(vault: Vault, claim: Claim, recovery: Recovery) -> Model {
        Model {
            vault,
            library: Vec::new(),
            documents: Documents::new(),
            files: BTreeMap::new(),
            pane: Pane {
                // The window's own first words, until the library is asked for.
                status: "Opening your library…".into(),
                ..Pane::new(WindowId::PRIMARY)
            },
            parked: BTreeMap::new(),
            visited: vec![WindowId::PRIMARY],
            ready: false,
            busy: None,
            job_window: WindowId::PRIMARY,
            notice: String::new(),
            lasting_notice: false,
            notice_in: None,
            pending: None,
            closing: None,
            close_asked_in: WindowId::PRIMARY,
            leaving: None,
            recovery,
            written: BTreeMap::new(),
            handed_over: Vec::new(),
            draft_failed_at: None,
            asked: None,
            answer: Value::Null,
            _claim: claim,
        }
    }

    /// The document the focused window shows.
    pub fn current_id(&self) -> Option<DocumentId> {
        self.document_in(&self.pane)
    }

    /// An open note by its document: the document and what its files hold.
    pub fn note_of(&self, id: DocumentId) -> Option<Current<'_>> {
        Some(Current { doc: self.documents.get(id)?, file: self.files.get(&id)? })
    }

    /// The note the focused window shows.
    pub fn current(&self) -> Option<Current<'_>> {
        self.current_in(&self.pane)
    }

    pub fn current_mut(&mut self) -> Option<(&mut Document<NoteText>, &mut NoteFile)> {
        let id = self.current_id()?;
        Some((self.documents.get_mut(id)?, self.files.get_mut(&id)?))
    }

    /// The focused window's note has edits not on disk.
    pub fn unsaved(&self) -> bool {
        self.current().is_some_and(|c| c.unsaved())
    }

    /// Whether a document has edits not on disk: its text or sidecar is not what its files hold.
    /// The one rule for "unsaved", wherever it is asked (the save path, the close flow, the
    /// footer, `describe`), so no window can call a note saved while another holds its edits.
    pub fn unsaved_doc(&self, id: DocumentId) -> bool {
        self.note_of(id).is_some_and(|c| c.unsaved())
    }

    /// Makes every document with edits not on disk (`unsaved_doc`) dirty to the framework as well:
    /// a save starts only for a dirty document, a draft is kept only for one, and a close asks only
    /// about one.
    ///
    /// A note whose file was found gone is the one way the two part on their own: nothing was
    /// typed, and yet its text is on no disk. It is made dirty here, so its draft is kept, a close
    /// asks about it, and nothing calls it saved (`gone_at` remembers the edit, undone if the file
    /// comes back). Every other change is an edit of its document, so the two never part; if they
    /// ever did, a test fails here, and a running Notes says so in its log and makes the document
    /// dirty, so the text is saved, kept and asked about rather than closed away as if on disk.
    pub fn keep_unsaved_dirty(&mut self) {
        let parted: Vec<DocumentId> =
            self.documents.iter().filter(|d| !d.dirty() && self.unsaved_doc(d.id)).map(|d| d.id).collect();
        for id in parted {
            let Some(d) = self.documents.get_mut(id) else { continue };
            let gone = d.outside == OutsideState::Deleted;
            if !gone {
                if cfg!(test) {
                    panic!("Notes' unsaved note must be a dirty document");
                }
                tracing::error!("an unsaved note was not a dirty document; it is made one, so it is saved and asked about");
            }
            let seq = d.edit(|_| ());
            if let Some(file) = self.files.get_mut(&id).filter(|_| gone) {
                file.gone_at = Some(seq);
            }
        }
    }

    /// The focused window's note's offered text, while its offer waits (`offered_in`).
    pub fn offered(&self) -> Option<&NoteText> {
        self.offered_in(&self.pane)
    }

    /// What the focused window shows of its note (`shown_in`).
    pub fn shown(&self) -> Option<&NoteText> {
        self.shown_in(&self.pane)
    }

    /// The focused window's note as the library lists it, as the window shows it.
    pub fn shown_note(&self) -> Option<Note> {
        self.shown_note_in(&self.pane)
    }

    /// Whether what a note's files hold is this run's last landed write to it.
    pub fn ours(&self, file: &NoteFile) -> bool {
        let held = file.on_disk.as_ref().map(|d| Fingerprint::of_bytes(d.text.as_bytes()));
        held.is_some() && self.written.get(&file.id) == held.as_ref()
    }

    /// The live notes, other than `except`, that held exactly `content` when last read: the ones
    /// whose files a job reads to learn whether one still holds it, as file name and length.
    pub fn alike(&self, content: &NoteText, except: &str) -> Vec<(String, usize)> {
        self.library
            .iter()
            .filter(|n| !n.trash && n.id != except && n.content == *content)
            .map(|n| (n.id.clone(), n.content.text.len()))
            .collect()
    }

    /// The library's text, leaving out the focused window's note's own entry.
    pub fn text_beside_current(&self) -> usize {
        let key = self.current().map(|c| (c.id().to_string(), c.trash()));
        self.library
            .iter()
            .filter(|n| key.as_ref().is_none_or(|(id, trash)| *id != n.id || *trash != n.trash))
            .map(|n| n.content.text.len())
            .sum()
    }

    pub fn library_text(&self) -> usize {
        self.library.iter().map(|n| n.content.text.len()).sum()
    }
}
