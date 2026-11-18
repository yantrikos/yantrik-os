//! The save state machine: one save in flight per document, at most one follow-up owed, no
//! retry by itself after a failure, and the durability of a failure said honestly.

use super::{Document, DocumentId, Fingerprint, Observed, OutsideState};

/// How much of a failed save may have reached the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// Nothing changed on disk: the file is as it was before the save began.
    NotWritten,
    /// The content landed and its companion data (metadata) did not; the app's journal holds
    /// the rest. Treated as not saved.
    ContentOnly,
    /// A step after the rename failed (a directory sync): the new content may or may not
    /// survive a power loss. Treated as not saved.
    Unknown,
}

/// Why a save did not commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveError {
    pub durability: Durability,
    /// The sentence for the person, in the app's words.
    pub sentence: String,
    /// The file on disk no longer matches the baseline: an outside edit got there first.
    pub conflict: bool,
    /// The fingerprint of the bytes this save wrote, when they may be on disk (`ContentOnly`,
    /// `Unknown`): seeing them is our own write, not an outside edit, and a retry may replace
    /// them.
    pub landed: Option<Fingerprint>,
}

impl SaveError {
    /// A failure that wrote nothing. Any other durability goes through
    /// [`SaveError::after_writing`], so the bytes that may have landed are never forgotten.
    pub fn new(sentence: impl Into<String>) -> SaveError {
        SaveError { durability: Durability::NotWritten, sentence: sentence.into(), conflict: false, landed: None }
    }

    /// A failure after which these bytes may be on disk (nothing, for `NotWritten`).
    pub fn after_writing(durability: Durability, written: Fingerprint, sentence: impl Into<String>) -> SaveError {
        let landed = (durability != Durability::NotWritten).then_some(written);
        SaveError { durability, sentence: sentence.into(), conflict: false, landed }
    }

    pub fn conflict(sentence: impl Into<String>) -> SaveError {
        SaveError { durability: Durability::NotWritten, sentence: sentence.into(), conflict: true, landed: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveState {
    Idle,
    Saving { seq: u64 },
    Failed { seq: u64, error: SaveError },
}

/// What a save job is given: the content as it was at `seq`, and what it may find on disk.
#[derive(Debug, Clone)]
pub struct SaveTicket<C> {
    pub document: DocumentId,
    pub seq: u64,
    pub content: C,
    /// The file as this document last read or wrote it; `None` when it creates a new file.
    pub baseline: Option<Fingerprint>,
    /// Bytes of ours from a save whose durability was not known, which may be on disk instead.
    pub unsure: Option<Fingerprint>,
}

impl<C> SaveTicket<C> {
    /// The store's check before it replaces the file: what is on disk now (`None`: no file) is
    /// what this document knows is there, its baseline or its own unsure write. Anything else is
    /// an outside edit, and the save must fail as a conflict.
    pub fn may_replace(&self, on_disk: Option<&Fingerprint>) -> bool {
        match on_disk {
            None => self.baseline.is_none(),
            Some(now) => Some(now) == self.baseline.as_ref() || Some(now) == self.unsure.as_ref(),
        }
    }

    /// What the save's replace took the place of ([`super::disk::Disk::exchange`]), judged
    /// against what this document knew was there: the same file, or another program's write (or
    /// a deletion) that came between the store's check and the replace. Certain, unlike a
    /// sighting by the watch, which sees such a write only if it happened to look.
    pub fn judge(&self, displaced: &super::disk::Displaced) -> Witnessed {
        use super::disk::Displaced;
        match displaced {
            // Kept unread: it may have been theirs, and the app keeps the file it is left in.
            Displaced::Unknown | Displaced::Unread(_) => Witnessed::Unknown,
            Displaced::Nothing if self.baseline.is_some() => Witnessed::Replaced(Replaced::Deletion),
            Displaced::Nothing => Witnessed::Same,
            Displaced::Bytes(bytes) => {
                if self.may_replace(Some(&Fingerprint::of_bytes(bytes))) {
                    Witnessed::Same
                } else {
                    Witnessed::Replaced(Replaced::OutsideEdit)
                }
            }
        }
    }
}

/// What a save's replace is known to have taken the place of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Witnessed {
    /// The file this document knew: nothing to say.
    Same,
    /// Another program's write, or a deletion, between the check and the replace.
    Replaced(Replaced),
    /// The disk could not tell; the watch's sightings are all there is.
    Unknown,
}

/// What `begin_save` decided.
#[derive(Debug, Clone)]
pub enum BeginSave<C> {
    /// Start this save.
    Ticket(SaveTicket<C>),
    /// Nothing to save.
    Clean,
    /// A save is in flight; one follow-up is now owed.
    InFlight,
    /// The last save of this very content failed; an autosave does not retry it on its own.
    Failed,
    /// The file changed or went outside; the person must choose first.
    Blocked,
}

/// What a save that landed replaced, which the person must hear about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replaced {
    /// Another program wrote the file while ours was in flight, and ours replaced it.
    OutsideEdit,
    /// The file was deleted or moved while ours was in flight, and ours made it again.
    Deletion,
}

/// What a save's outcome did.
#[derive(Debug, Clone)]
pub enum SaveOutcome<C> {
    /// Saved. `follow_up` is the save the edits made meanwhile need; `replaced`, when set, is
    /// said to the person.
    Saved {
        follow_up: Option<SaveTicket<C>>,
        replaced: Option<Replaced>,
    },
    Failed,
    /// For a save this document no longer expects (an older one, after a newer outcome).
    Stale,
}

impl<C: Clone> Document<C> {
    /// An autosave or a save the app asks for on its own. A failed save of unchanged content is
    /// not retried here, so a read-only disk does not raise a notice every tick.
    pub fn begin_save(&mut self) -> BeginSave<C> {
        // Blocked says more than Failed: the person has a choice to make, not a disk to fix.
        if !self.blocked() {
            if let SaveState::Failed { seq, .. } = self.save {
                if seq == self.seq {
                    return BeginSave::Failed;
                }
            }
        }
        self.start()
    }

    /// The person asked to save, or a close flow did: a failure is retried.
    pub fn retry_save(&mut self) -> BeginSave<C> {
        self.start()
    }

    /// The file changed or went outside: no save until the person chooses.
    pub fn blocked(&self) -> bool {
        matches!(self.outside, OutsideState::Conflict { .. } | OutsideState::Deleted)
    }

    fn start(&mut self) -> BeginSave<C> {
        if self.blocked() {
            return BeginSave::Blocked;
        }
        if matches!(self.save, SaveState::Saving { .. }) {
            if self.dirty() {
                self.follow_up = true;
            }
            return BeginSave::InFlight;
        }
        if !self.dirty() && self.baseline.is_some() {
            return BeginSave::Clean;
        }
        self.save = SaveState::Saving { seq: self.seq };
        BeginSave::Ticket(SaveTicket {
            document: self.id,
            seq: self.seq,
            content: self.content.clone(),
            baseline: self.baseline.clone(),
            unsure: self.unsure.clone(),
        })
    }

    /// The save of `seq` ended. `Ok` carries the fingerprint of what is now on disk. After a
    /// save lands, the app re-arms the document's watch from its new baseline (`fx.watch` with
    /// [`Document::watch_entry`]), so a look the watch took before the save is dropped.
    pub fn save_landed(&mut self, seq: u64, result: Result<Fingerprint, SaveError>) -> SaveOutcome<C> {
        self.save_landed_witnessed(seq, result, Witnessed::Unknown)
    }

    /// [`Document::save_landed`], with what the replace is known to have displaced
    /// ([`SaveTicket::judge`]). A known answer replaces the guess from the watch's sightings:
    /// no "replaced" said when theirs was not, and none missed because the watch did not look.
    pub fn save_landed_witnessed(
        &mut self,
        seq: u64,
        result: Result<Fingerprint, SaveError>,
        witnessed: Witnessed,
    ) -> SaveOutcome<C> {
        match self.save {
            SaveState::Saving { seq: in_flight } if in_flight == seq => {}
            _ => return SaveOutcome::Stale,
        }
        let seen = self.seen_while_saving.take();
        match result {
            Ok(written) => {
                let before = self.baseline.replace(written.clone());
                let unsure = self.unsure.take();
                self.saved_seq = self.saved_seq.max(seq);
                self.remember_write(written.clone());
                self.save = SaveState::Idle;
                if matches!(self.outside, OutsideState::Changed { .. }) {
                    self.outside = OutsideState::Clean;
                }
                // Seen during the save: neither the old file, nor ours, nor an unsure write of
                // ours, so it existed between our check and our rename, and ours replaced it.
                let guessed = match &seen {
                    Some(Observed::File(f))
                        if *f != written && Some(f) != before.as_ref() && Some(f) != unsure.as_ref() =>
                    {
                        Some(Replaced::OutsideEdit)
                    }
                    Some(Observed::Gone) if before.is_some() => Some(Replaced::Deletion),
                    _ => None,
                };
                let replaced = match witnessed {
                    Witnessed::Same => None,
                    Witnessed::Replaced(r) => Some(r),
                    Witnessed::Unknown => guessed,
                };
                let follow_up = if std::mem::take(&mut self.follow_up) && self.dirty() {
                    match self.start() {
                        BeginSave::Ticket(t) => Some(t),
                        _ => None,
                    }
                } else {
                    None
                };
                SaveOutcome::Saved { follow_up, replaced }
            }
            Err(error) => {
                self.follow_up = false;
                if let Some(landed) = &error.landed {
                    // Our bytes may be on disk: seeing them is not an outside edit, and the
                    // retry may replace them. Nothing is called saved.
                    self.unsure = Some(landed.clone());
                }
                if error.conflict {
                    self.outside = OutsideState::Conflict { now: None };
                }
                self.save = SaveState::Failed { seq, error };
                // A sighting during a save that failed is judged now, against what we know.
                if let Some(seen) = seen {
                    self.observe(seen);
                }
                SaveOutcome::Failed
            }
        }
    }
}
