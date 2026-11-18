//! Changes made outside this process, classified. Never last-writer-wins: an outside edit to a
//! clean document is offered, one to a dirty document is a conflict the person resolves, and a
//! deletion is said. Our own writes are recognised by fingerprint and are not outside edits.
//!
//! An older write of ours is ambiguous. It may be a late look by the watch, taken before our
//! newer write landed, or another program putting our older text back (a sync client, a
//! `git checkout`). The first sighting of it is not trusted either way: it is suppressed once,
//! and the document asks for its watch to be re-armed at the baseline ([`Document::rewatch`]).
//! A late look then goes quiet, because the file is the baseline. A revert is reported again,
//! and the second time it is an outside edit.

use std::path::PathBuf;

use super::{Document, Fingerprint, SaveState};

/// What the watch saw at a document's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    File(Fingerprint),
    Gone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutsideState {
    Clean,
    /// The file changed and nothing here was unsaved: the app may load the new text.
    Changed { now: Fingerprint },
    /// The file changed while edits here were unsaved (or a save found it changed): the person
    /// chooses: keep mine as a copy, take theirs, or not now. `now` is `None` when a save's
    /// check found the change and the watch has not seen it yet.
    Conflict { now: Option<Fingerprint> },
    /// The file was moved or deleted. Polling cannot tell which. A save becomes *save as new*.
    Deleted,
}

impl<C: Clone> Document<C> {
    /// A sighting from the watch. Returns the state it leaves the document in. Afterwards the
    /// app calls [`Document::rewatch`], and re-arms the watch when it says to.
    pub fn observe(&mut self, seen: Observed) -> &OutsideState {
        if matches!(self.save, SaveState::Saving { .. }) {
            // Judged when the save lands: against our write, not the old baseline.
            self.seen_while_saving = Some(seen);
            return &self.outside;
        }
        match seen {
            Observed::File(now) => {
                if self.baseline.as_ref() == Some(&now) || self.unsure.as_ref() == Some(&now) {
                    // The watch has caught up with what we know is there: no older write of
                    // ours can still be on its way.
                    self.ours.clear();
                    if !matches!(self.outside, OutsideState::Conflict { .. }) {
                        self.outside = OutsideState::Clean;
                    }
                } else if self.ours.contains(&now) {
                    // An older write of ours: a late look, or a revert. Suppressed once.
                    self.ours.retain(|f| *f != now);
                    self.recheck = true;
                } else {
                    self.ours.clear();
                    if self.dirty() || matches!(self.outside, OutsideState::Conflict { .. }) {
                        self.outside = OutsideState::Conflict { now: Some(now) };
                    } else {
                        self.outside = OutsideState::Changed { now };
                    }
                }
            }
            Observed::Gone => {
                if self.source.path().is_some() && self.baseline.is_some() {
                    self.outside = OutsideState::Deleted;
                }
            }
        }
        &self.outside
    }

    /// After a sighting of an older write of ours: the watch entry to arm again, from the
    /// baseline, so the file is looked at once more. `None` when nothing is owed.
    pub fn rewatch(&mut self) -> Option<(String, PathBuf, Option<Observed>)> {
        if !std::mem::take(&mut self.recheck) {
            return None;
        }
        self.watch_entry()
    }

    /// Take theirs: the file's content replaces this document's. The app keeps the text being
    /// replaced somewhere recoverable first (a recovery note) and clears every view's undo.
    pub fn take_theirs(&mut self, theirs: C, fingerprint: Fingerprint) {
        self.content = theirs;
        self.baseline = Some(fingerprint);
        self.seq += 1;
        self.saved_seq = self.seq;
        self.outside = OutsideState::Clean;
        self.save = SaveState::Idle;
        self.forget_disk_history();
    }

    /// Keep mine as a copy: returns this document's text for the app to save as a new
    /// document, and resets this one to the file's content as `take_theirs` does.
    pub fn keep_mine_as_copy(&mut self, theirs: C, fingerprint: Fingerprint) -> C {
        let mine = self.content.clone();
        self.take_theirs(theirs, fingerprint);
        mine
    }

    /// The file is gone and the person chose to keep the text: the document becomes new, so
    /// the next save creates a file instead of replacing one.
    pub fn save_as_new(&mut self, key: impl Into<String>) {
        self.source = super::Source::New(key.into());
        self.baseline = None;
        self.outside = OutsideState::Clean;
        self.save = SaveState::Idle;
        self.forget_disk_history();
    }

    /// What this document knew of its file's past: none of it applies to the file it has now.
    fn forget_disk_history(&mut self) {
        self.follow_up = false;
        self.seen_while_saving = None;
        self.ours.clear();
        self.unsure = None;
        self.recheck = false;
    }
}
