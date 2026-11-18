//! A window's pane: everything about one Notes window that belongs to the view and not to the note
//! (design/notes-gate-2026-10-21.md §2, "Views and windows"). A note shown in two windows is one
//! document with two views: its text, unsaved state, conflict and draft are the document's, and
//! each window keeps its own pane, with its undo history, search, dialog, scroll and footer line.
//!
//! The focused window's pane is `Model::pane`, and every command acts there: a command from a
//! window focuses that window first, and a job's landing, or a close, is applied in the window it
//! is about, with the focus put back afterwards (update/windows.rs). The other windows' panes wait
//! in `Model::parked`. So the code that acts on "the open note" has one pane to act on, and it is
//! always the right one.
//!
//! The focus moves for a moment wherever work is done in another window ([`Model::within`]); the
//! person moves only by acting in a window or opening one ([`Model::enter`]). `Model::visited`
//! keeps the windows in the order the person was last in them, so when the focused window closes,
//! the focus goes back to the window the person was in before it.

use yantrik_app::app::WindowId;
use yantrik_app::documents::{DocumentId, ViewId};

use crate::model::{Current, Model};
use crate::note::{Note, NoteText};
use crate::undo::History;
use crate::view::listing::Listing;
use crate::view::preview::Block;

/// One window's view of the library and of the note it shows.
pub struct Pane {
    pub window: WindowId,
    /// The view on the note this window shows, when it shows one.
    pub view: Option<ViewId>,
    pub history: History,
    /// The footer's save state, as the last thing that set it said it.
    pub status: String,
    pub folder: String,
    pub query: String,
    /// Whether the query is matched against the notes' text as well as their titles. The window
    /// of a Mind's display matches titles only; a search by a call matches as it always did.
    pub text_search: bool,
    pub preview: bool,
    pub focus_mode: bool,
    /// 0 none, 1 import, 2 export, 3 compare, 4 the close prompt.
    pub dialog: i32,
    /// The file's version beside the window's text in the comparison; `None` when its file is gone.
    pub theirs: Option<NoteText>,
    /// *Not now*: the outside bar is put away until a save is refused or the file changes again.
    /// Saving stays refused all the while.
    pub outside_later: bool,
    /// The open note's file changed while the store was busy: it is read again once it is not.
    pub reread_owed: bool,
    pub path: String,
    /// The notebook and tags fields as typed (the sidecar keeps them cut to length).
    pub notebook: String,
    pub tags: String,
    /// The library list as last drawn: it is redrawn where it always was, not on every key.
    pub listing: Listing,
    pub blocks: Vec<Block>,
    /// What the window is to do once drawn: focus, caret, scroll. Cleared before every command.
    pub cues: Vec<Cue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// A note was shown: scroll to its top and focus what can take keys.
    Shown,
    CaretAt(i32),
    FocusEditor,
    FocusShortcuts,
    FocusContent,
}

impl Pane {
    /// A window showing nothing yet, on the whole library.
    pub fn new(window: WindowId) -> Pane {
        Pane {
            window,
            view: None,
            history: History::default(),
            status: String::new(),
            folder: "all".into(),
            query: String::new(),
            text_search: true,
            preview: false,
            focus_mode: false,
            dialog: 0,
            theirs: None,
            outside_later: false,
            reread_owed: false,
            path: String::new(),
            notebook: String::new(),
            tags: String::new(),
            listing: Listing::default(),
            blocks: Vec::new(),
            cues: Vec::new(),
        }
    }
}

impl Model {
    /// Every window's pane, the focused one first.
    pub fn panes(&self) -> impl Iterator<Item = &Pane> {
        std::iter::once(&self.pane).chain(self.parked.values())
    }

    pub fn panes_mut(&mut self) -> impl Iterator<Item = &mut Pane> {
        std::iter::once(&mut self.pane).chain(self.parked.values_mut())
    }

    /// The windows open, the focused one first.
    pub fn windows(&self) -> Vec<WindowId> {
        self.panes().map(|p| p.window).collect()
    }

    pub fn pane_of(&self, window: WindowId) -> Option<&Pane> {
        if self.pane.window == window {
            return Some(&self.pane);
        }
        self.parked.get(&window)
    }

    pub fn pane_of_mut(&mut self, window: WindowId) -> Option<&mut Pane> {
        if self.pane.window == window {
            return Some(&mut self.pane);
        }
        self.parked.get_mut(&window)
    }

    /// Makes `window` the focused one, where commands act. `false` when it is not open.
    pub fn focus(&mut self, window: WindowId) -> bool {
        if self.pane.window == window {
            return true;
        }
        let Some(pane) = self.parked.remove(&window) else { return false };
        let was = std::mem::replace(&mut self.pane, pane);
        self.parked.insert(was.window, was);
        true
    }

    /// The person is in `window` now: a command came from it, or it was just opened. It is focused,
    /// and first in the order the person was in the windows. `false` when it is not open.
    pub fn enter(&mut self, window: WindowId) -> bool {
        if !self.focus(window) {
            return false;
        }
        self.visited.retain(|w| *w != window);
        self.visited.insert(0, window);
        true
    }

    /// Runs `f` with `window` focused, then puts the focus back. If the window `f` started from
    /// closed meanwhile, the focus stays where `f` left it. `None`, and `f` not run, when `window`
    /// is not open.
    pub fn within<R>(&mut self, window: WindowId, f: impl FnOnce(&mut Model) -> R) -> Option<R> {
        let back = self.pane.window;
        if !self.focus(window) {
            return None;
        }
        let r = f(self);
        self.focus(back);
        Some(r)
    }

    /// A new window's pane, focused: the window just opened is the one the person is in.
    pub fn add_pane(&mut self, pane: Pane) {
        let window = pane.window;
        self.parked.insert(window, pane);
        self.enter(window);
    }

    /// A window's pane is gone with its window; when it was the focused one, the focus goes back
    /// to the window the person was in before it, or, if that closed too, the one before that.
    /// The last window's pane is never removed: the app ends with it.
    pub fn remove_pane(&mut self, window: WindowId) {
        if self.pane.window == window {
            let before = self.visited.iter().copied().find(|w| *w != window && self.parked.contains_key(w));
            let Some(next) = before.or_else(|| self.parked.keys().next().copied()) else { return };
            self.enter(next);
        }
        self.visited.retain(|w| *w != window);
        self.parked.remove(&window);
    }

    /// The document a window's pane shows.
    pub fn document_in(&self, pane: &Pane) -> Option<DocumentId> {
        self.documents.view(pane.view?).map(|v| v.document)
    }

    /// The note a window's pane shows: its document and what its files hold.
    pub fn current_in(&self, pane: &Pane) -> Option<Current<'_>> {
        self.note_of(self.document_in(pane)?)
    }

    /// Whether the window the person is in shows `doc`. While work is done in another window for a
    /// moment, that window is focused; the person is still where they were.
    pub fn in_view(&self, doc: DocumentId) -> bool {
        let window = self.visited.first().copied().unwrap_or(self.pane.window);
        self.pane_of(window).is_some_and(|p| self.document_in(p) == Some(doc))
    }

    /// The windows showing `doc`, the focused one first.
    pub fn windows_showing(&self, doc: DocumentId) -> Vec<WindowId> {
        self.panes().filter(|p| self.document_in(p) == Some(doc)).map(|p| p.window).collect()
    }

    /// The other windows showing the focused window's note.
    pub fn others_showing(&self) -> Vec<WindowId> {
        let Some(doc) = self.current_id() else { return Vec::new() };
        self.windows_showing(doc).into_iter().filter(|w| *w != self.pane.window).collect()
    }

    /// A pane's note's unsaved text from before Notes closed, while its offer waits: what the
    /// window shows of the note (text, title, fields, list card, preview), and what Copy and
    /// Export take, until the person chooses.
    pub fn offered_in(&self, pane: &Pane) -> Option<&NoteText> {
        let c = self.current_in(pane)?;
        self.recovery.offer_for(c.id()).map(|o| &o.content)
    }

    /// What a pane shows of its note: its offered text while that waits, its content otherwise.
    pub fn shown_in(&self, pane: &Pane) -> Option<&NoteText> {
        self.offered_in(pane).or_else(|| self.current_in(pane).map(|c| &c.doc.content))
    }

    /// A pane's note as the library lists it, as the window shows it.
    pub fn shown_note_in(&self, pane: &Pane) -> Option<Note> {
        let mut note = self.current_in(pane)?.as_note();
        if let Some(offered) = self.offered_in(pane) {
            note.content = offered.clone();
        }
        Some(note)
    }
}
