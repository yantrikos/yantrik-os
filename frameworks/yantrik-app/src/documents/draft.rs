//! Drafts per document, ordered by `seq`, and the rule that decides what to offer back at a
//! start: the newest of the file, an interrupted save's journal, and the draft — and only one
//! that differs from what the file already says.

use serde::{Deserialize, Serialize};

use super::{Document, Fingerprint};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftState {
    None,
    /// A draft of this `seq` is asked for (or on disk).
    Kept { seq: u64 },
    /// The draft could not be kept: recovery after a crash is not protected, and the person
    /// is told.
    Failed { sentence: String },
}

/// What a draft file holds: the app's content, with what it was a draft of and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope<C> {
    pub version: u32,
    /// The document's `Source::key()`.
    pub key: String,
    /// The file it is a draft of, if any, so a recovered draft can be shown beside it.
    pub path: Option<String>,
    pub seq: u64,
    /// Milliseconds since the epoch, when it was made.
    pub at: u64,
    pub content: C,
    /// The file as the document last read or wrote it when the draft was made (`None` for a
    /// document never saved). A start compares it with the file: equal, and the draft's edits
    /// can go back into the document as they were; different, and the file moved on since, so
    /// putting the draft back is a conflict the person resolves, never a silent replace. Absent
    /// in a draft written before it was kept, which reads as not known.
    #[serde(default)]
    pub baseline: Option<Fingerprint>,
}

/// The draft envelope's version.
pub const ENVELOPE_VERSION: u32 = 1;

impl<C: Clone + Serialize> Document<C> {
    /// The draft to keep for this document now, if one is owed (it is dirty and the draft is
    /// older than the edits). `now_ms` is milliseconds since the epoch. The app passes the
    /// bytes to `fx.save_draft(name, bytes)` and then calls [`Document::draft_asked`].
    pub fn draft_owed(&self, now_ms: u64) -> Option<(String, Vec<u8>)> {
        if !self.dirty() {
            return None;
        }
        if let DraftState::Kept { seq } = self.draft {
            if seq >= self.seq {
                return None;
            }
        }
        let envelope = Envelope {
            version: ENVELOPE_VERSION,
            key: self.source.key(),
            path: self.source.path().map(|p| p.display().to_string()),
            seq: self.seq,
            at: now_ms,
            content: self.content.clone(),
            baseline: self.baseline.clone(),
        };
        let bytes = serde_json::to_vec(&envelope).ok()?;
        Some((self.source.key(), bytes))
    }

    pub fn draft_asked(&mut self) {
        self.draft = DraftState::Kept { seq: self.seq };
    }

    pub fn draft_failed(&mut self, sentence: impl Into<String>) {
        self.draft = DraftState::Failed { sentence: sentence.into() };
    }

    /// Whether the draft can be discarded: a save of its `seq` or later has committed.
    pub fn draft_superseded(&self) -> bool {
        match self.draft {
            DraftState::Kept { seq } => self.saved_seq >= seq,
            DraftState::Failed { .. } => !self.dirty(),
            DraftState::None => false,
        }
    }

    pub fn draft_discarded(&mut self) {
        self.draft = DraftState::None;
    }
}

/// What a start should do with a document's file, its journal and its draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovered<C> {
    /// Nothing differs from the file: open it as it is.
    File,
    /// Offer this text back, from a draft.
    Draft(C),
    /// Offer this text back, from an interrupted save.
    Journal(C),
}

/// A text a start may offer back: from a draft or an interrupted save's journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate<C> {
    pub content: C,
    /// Milliseconds since the epoch, when it was written.
    pub at_ms: u64,
    /// The edit it holds (`Envelope::seq`), when it records one.
    pub seq: Option<u64>,
}

/// The precedence at a start. A candidate equal to the file's content is not offered. Of the
/// rest, the newer wins. At the same millisecond the later edit wins when both record their
/// `seq`, and otherwise the draft, which is the later of the two writes one edit makes.
///
/// A wall clock stepped backwards between the two writes (NTP) can make the older text look
/// newer. Both texts are kept until the person's choice, so the other is still recoverable.
pub fn recover<C: PartialEq>(
    file: Option<&C>,
    journal: Option<Candidate<C>>,
    draft: Option<Candidate<C>>,
) -> Recovered<C> {
    let differs = |c: &Candidate<C>| file.is_none_or(|f| *f != c.content);
    let journal = journal.filter(differs);
    let draft = draft.filter(differs);
    match (journal, draft) {
        (None, None) => Recovered::File,
        (Some(j), None) => Recovered::Journal(j.content),
        (None, Some(d)) => Recovered::Draft(d.content),
        (Some(j), Some(d)) => {
            let journal_later = match j.at_ms.cmp(&d.at_ms) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Less => false,
                std::cmp::Ordering::Equal => matches!((j.seq, d.seq), (Some(js), Some(ds)) if js > ds),
            };
            if journal_later {
                Recovered::Journal(j.content)
            } else {
                Recovered::Draft(d.content)
            }
        }
    }
}
