//! Close as a document flow. Closing a view only asks about a document when it is the last view
//! on it and the document is dirty; quitting asks once over every dirty document; no window is
//! privileged. While a close waits on its saves, edits go on, and the close completes only when
//! every document it asked about is clean.

use std::collections::BTreeSet;

use super::{DocumentId, Documents, ViewId};
use crate::app::WindowId;

/// What closing something would take with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Closing {
    /// Nothing dirty would be lost: close now.
    Now,
    /// These dirty documents have no other view: ask *Save / Don't save / Cancel*.
    Ask(Vec<DocumentId>),
}

/// The person's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Save,
    DontSave,
    Cancel,
}

impl<C> Documents<C> {
    /// Closing one view.
    pub fn closing_view(&self, view: ViewId) -> Closing {
        let Some(v) = self.view(view) else { return Closing::Now };
        let others = self.views_of(v.document).filter(|o| o.id != view).count();
        match self.get(v.document) {
            Some(d) if d.dirty() && others == 0 => Closing::Ask(vec![v.document]),
            _ => Closing::Now,
        }
    }

    /// Closing a window: its views' documents that no other window shows.
    pub fn closing_window(&self, window: WindowId) -> Closing {
        let mut ask = BTreeSet::new();
        for v in self.views_in(window) {
            let elsewhere = self.views_of(v.document).any(|o| o.window != window);
            if !elsewhere && self.get(v.document).is_some_and(|d| d.dirty()) {
                ask.insert(v.document);
            }
        }
        if ask.is_empty() {
            Closing::Now
        } else {
            Closing::Ask(ask.into_iter().collect())
        }
    }

    /// Quitting: every dirty document, once.
    pub fn quitting(&self) -> Closing {
        let ask: Vec<DocumentId> = self.dirty().map(|d| d.id).collect();
        if ask.is_empty() {
            Closing::Now
        } else {
            Closing::Ask(ask)
        }
    }
}

/// A close waiting on the person and then on saves. The app keeps one per window (or one for
/// quitting) in its model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseFlow {
    pub window: Option<WindowId>,
    pub asked: Vec<DocumentId>,
    pub stage: Stage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// The prompt is showing.
    Asking,
    /// *Save* was chosen: close when every asked document is clean.
    SavingThenClose,
    /// A save failed while closing: the window stays, with this sentence.
    Blocked(String),
}

/// What the app does after an answer or a save outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Start these saves (the latest `seq` of each) with [`Document::retry_save`], not
    /// `begin_save`: a document whose last autosave failed would answer `Failed` and the close
    /// would wait for ever. A save that answers `Blocked`, or lands `Failed`, is fed back to
    /// [`CloseFlow::progress`] as `failed`, with its sentence.
    ///
    /// [`Document::retry_save`]: super::Document::retry_save
    Save(Vec<DocumentId>),
    /// Close now. With *Don't save*, discard these documents' edits and drafts.
    Close { discard: Vec<DocumentId> },
    /// Keep everything as it is; the prompt goes away.
    Stay,
    /// Still waiting.
    Wait,
}

impl CloseFlow {
    pub fn new(window: Option<WindowId>, asked: Vec<DocumentId>) -> CloseFlow {
        CloseFlow { window, asked, stage: Stage::Asking }
    }

    /// The person answered the prompt. A second close request while saving changes nothing.
    pub fn answer(&mut self, choice: Choice) -> Next {
        match (&self.stage, choice) {
            (Stage::SavingThenClose, _) => Next::Wait,
            (_, Choice::Cancel) => Next::Stay,
            (_, Choice::DontSave) => Next::Close { discard: self.asked.clone() },
            (_, Choice::Save) => {
                self.stage = Stage::SavingThenClose;
                Next::Save(self.asked.clone())
            }
        }
    }

    /// A save outcome landed (or an edit was made) while closing. `failed` carries a failed or
    /// blocked save's sentence; otherwise each asked document's dirtiness decides.
    pub fn progress<C>(&mut self, documents: &Documents<C>, failed: Option<String>) -> Next {
        if self.stage != Stage::SavingThenClose {
            return Next::Wait;
        }
        if let Some(why) = failed {
            self.stage = Stage::Blocked(why);
            return Next::Stay;
        }
        let still_dirty: Vec<DocumentId> = self
            .asked
            .iter()
            .copied()
            .filter(|id| documents.get(*id).is_some_and(|d| d.dirty()))
            .collect();
        if still_dirty.is_empty() {
            Next::Close { discard: Vec::new() }
        } else {
            // Edits made while closing: save the latest of them too.
            Next::Save(still_dirty)
        }
    }
}
