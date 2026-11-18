//! Text a run of Notes kept and did not save, offered back at the next start (design/notes-gate
//! §2 "Drafts", §3 "Recovery after a forced kill").
//!
//! While a note has edits not on disk, its document's draft is kept through the framework's
//! drafts lane (update/drafts.rs), named by the note's file, in the framework's [`Envelope`]. A
//! run that ends without saving them (a crash, `kill -9`, a power cut) leaves the draft behind.
//! The next start reads every draft that is a draft of a note in this library, and once the
//! library is read, weighs each against the note's file and the interrupted save the store's
//! journal held, if it was of the same note: the newer wins, and only text that differs from the
//! file is offered (update/recovery.rs). Nothing is put back until the person chooses.

use std::path::{Path, PathBuf};

use yantrik_app::documents::draft::ENVELOPE_VERSION;
use yantrik_app::documents::{Envelope, Fingerprint, Source};
use yantrik_app::drafts::Drafts;

use crate::note::NoteText;

/// What a start found, and what is offered back.
#[derive(Debug, Default)]
pub struct Recovery {
    /// The drafts read at start, until the library is first read and they can be weighed.
    pub found: Option<Vec<Found>>,
    /// The texts offered back, newest first.
    pub offers: Vec<Offer>,
    /// Drafts found at start that could not be read (cut short, or another build's), kept where
    /// they are and said once.
    pub unreadable: Vec<PathBuf>,
    /// Drafts of a note in another library, kept where they are and said once.
    pub elsewhere: Vec<PathBuf>,
}

/// A draft read at start, of a note in this library.
#[derive(Debug, Clone)]
pub struct Found {
    pub offer: Offer,
    /// When it was written, in milliseconds since the epoch.
    pub at_ms: u64,
    /// The edit it holds.
    pub seq: u64,
}

/// A text offered back for a note.
#[derive(Debug, Clone)]
pub struct Offer {
    /// The note's file name in the vault.
    pub id: String,
    /// The draft's name, removed once the person chooses.
    pub draft: String,
    pub content: NoteText,
    /// The note's file as the draft was written over it; `None` for a note never saved, or a
    /// draft that does not say.
    pub base: Option<Fingerprint>,
}

/// A note's files, text and sidecar, as the one fingerprint a Notes draft records as its
/// baseline (`Envelope::baseline`, "the file it was written over"). Both, because a draft holds
/// both: a start then tells another program's change to either from the person's own unsaved
/// change, which leaves the files as they were. A text never holds a NUL (`store::validate`),
/// so the two cannot run together. `None` for a note with no file.
pub fn base_of(on_disk: Option<&NoteText>) -> Option<Fingerprint> {
    on_disk.map(|d| Fingerprint::of_bytes(format!("{}\0{}", d.text, d.meta).as_bytes()))
}

impl Recovery {
    /// The drafts in `drafts` (under `state_dir`) that are drafts of notes in the vault at
    /// `vault`. A draft of another library (the vault was moved, `YANTRIK_NOTES_DIR` changed), or
    /// one this build cannot read, is left on disk as it is, and its place is said: it is not
    /// this library's to offer or to remove, and it may be someone's only copy.
    pub fn read(drafts: &Drafts, vault: &Path, state_dir: &Path) -> Recovery {
        let mut recovery = Recovery::default();
        let mut found = Vec::new();
        for name in drafts.names() {
            let at = || yantrik_app::drafts::path_in(state_dir, name);
            match drafts.json::<Envelope<NoteText>>(name) {
                None => recovery.unreadable.push(at()),
                Some(envelope) => match of_this_library(envelope, name, vault) {
                    Some(f) => found.push(f),
                    None => recovery.elsewhere.push(at()),
                },
            }
        }
        found.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
        recovery.found = Some(found);
        recovery
    }

    /// The offer for note `id`, if there is one.
    pub fn offer_for(&self, id: &str) -> Option<&Offer> {
        self.offers.iter().find(|o| o.id == id)
    }
}

fn of_this_library(envelope: Envelope<NoteText>, name: &str, vault: &Path) -> Option<Found> {
    let path = Path::new(envelope.path.as_deref()?);
    let id = path.file_name()?.to_str()?.to_string();
    let ours = envelope.version == ENVELOPE_VERSION
        && path.parent() == Some(vault)
        && id.ends_with(".md")
        && envelope.key == name
        && Source::File(vault.join(&id)).key() == name;
    if !ours {
        return None;
    }
    Some(Found {
        offer: Offer { id, draft: name.into(), content: envelope.content, base: envelope.baseline },
        at_ms: envelope.at,
        seq: envelope.seq,
    })
}
