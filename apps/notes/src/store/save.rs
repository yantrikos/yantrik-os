//! Saving a note: the journal first, then the store's check that the files are still what the note
//! was read from, then the text, then the sidecar, then the journal removed. Each failure says
//! honestly how much reached the disk (`Durability`), so the document model never calls a note
//! saved that is not, and never mistakes our own bytes for an outside edit. The text replaces the
//! file by an atomic exchange, so what another program wrote after the check is kept (theirs.rs).

use std::path::Path;

use yantrik_app::documents::disk::Displaced;
use yantrik_app::documents::{Durability, Fingerprint, SaveError, SaveTicket, Witnessed};

use super::records::Record;
use super::theirs::{Alike, Theirs};
use super::{read, validate, Vault, CHANGED_ON_DISK, LIMIT, META_LIMIT};
use crate::note::{now, NoteText};

/// One save of an open note: the document's ticket, and what the note's files held when it was
/// read or last written.
#[derive(Clone)]
pub struct SaveJob {
    pub id: String,
    pub ticket: SaveTicket<NoteText>,
    /// The note as its files hold it; `None` for a note never saved.
    pub on_disk: Option<NoteText>,
    /// A sidecar of ours a failed save may have left on disk instead of `on_disk`'s.
    pub meta_unsure: Option<String>,
    pub modified: u64,
    /// The library's live notes, so another program's version a replace displaces is not kept
    /// again when one of them already holds it.
    pub alike: Vec<Alike>,
}

/// A save that landed.
#[derive(Debug, Clone)]
pub struct Saved {
    pub content: NoteText,
    /// The fingerprint of the `.md` as written: the document's new baseline.
    pub written: Fingerprint,
    pub modified: u64,
    /// What the replace is known to have taken the place of.
    pub witnessed: Witnessed,
    /// Another program's version it replaced, kept. Boxed: rare, and a note's worth of text,
    /// while every save's outcome is moved through the app's queue.
    pub theirs: Option<Box<Theirs>>,
}

/// A save that did not land, and the sidecar it may have left behind.
#[derive(Debug, Clone)]
pub struct SaveFailed {
    pub error: SaveError,
    pub meta_unsure: Option<String>,
    /// The library is open read-only and refused the save before writing anything. Its sentence
    /// is the library's own, which says what to do, so it is said as it is.
    pub read_only: bool,
    /// Another program's version the text's replace took the place of before a later step
    /// failed, kept. Boxed, as in [`Saved`].
    pub theirs: Option<Box<Theirs>>,
}

impl SaveFailed {
    pub(crate) fn wrote_nothing(sentence: impl Into<String>) -> SaveFailed {
        SaveFailed { error: SaveError::new(sentence), meta_unsure: None, read_only: false, theirs: None }
    }

    /// A failure after these bytes may have landed (`after_writing`).
    fn after(durability: Durability, written: &Fingerprint, sentence: impl Into<String>) -> SaveFailed {
        SaveFailed { error: SaveError::after_writing(durability, written.clone(), sentence), ..SaveFailed::wrote_nothing("") }
    }
}

/// What writing a note's files did: the save, and what its text's replace displaced.
type Wrote = (Saved, Option<Displaced>);

/// A failure, and what the text's replace displaced if it got that far. Boxed: a failure is
/// rare, and the success path should not carry its size.
type NotWrote = Box<(SaveFailed, Option<Displaced>)>;

impl Vault {
    /// Saves an open note. The store refuses to write over a file that is neither what the note
    /// was read from nor a write of ours, and over a sidecar that is neither, with
    /// [`CHANGED_ON_DISK`].
    ///
    /// What the note was read from is compared byte for byte, as the store always did: the
    /// fingerprint is a change detector, not a hash a synced vault's other writer cannot choose
    /// to match. Only our own unsure write, whose bytes this job does not hold, is known by its
    /// fingerprint (`SaveTicket::unsure`, the document's rule). A library open read-only refuses
    /// before anything, the journal included, in its own words (`SaveFailed::read_only`).
    pub fn save(&self, job: &SaveJob) -> Result<Saved, SaveFailed> {
        if let Some(why) = self.read_only() {
            return Err(SaveFailed { read_only: true, ..SaveFailed::wrote_nothing(why) });
        }
        let content = &job.ticket.content;
        let record = Record::of(&job.id, content, job.on_disk.as_ref(), job.modified);
        let baseline_text = job.on_disk.as_ref().map(|d| d.text.as_str());
        let baseline_meta = job.on_disk.as_ref().map(|d| d.meta.as_str()).unwrap_or_default();
        let ours = |text: &str| job.ticket.unsure.as_ref() == Some(&Fingerprint::of_bytes(text.as_bytes()));
        let wrote = self.write_files(
            &job.id,
            content,
            Some(&record),
            |text| text == baseline_text || text.is_some_and(ours),
            |meta| meta == baseline_meta || Some(meta) == job.meta_unsure.as_deref(),
        );
        // Whatever came after it, a replace that displaced another program's version keeps it.
        match wrote {
            Ok((saved, displaced)) => {
                let (witnessed, theirs) = self.keep_theirs(&job.ticket, displaced, &job.alike);
                Ok(Saved { witnessed, theirs: theirs.map(Box::new), ..saved })
            }
            Err(not) => {
                let (failed, displaced) = *not;
                let (_, theirs) = self.keep_theirs(&job.ticket, displaced, &job.alike);
                Err(SaveFailed { theirs: theirs.map(Box::new), ..failed })
            }
        }
    }

    /// Writes a note no file holds yet (a restore, an import): refused if anything is there.
    pub(super) fn save_new(&self, id: &str, content: &NoteText) -> Result<Saved, String> {
        let record = Record::of(id, content, None, 0);
        self.write_note(id, content, Some(&record), |text| text.is_none(), str::is_empty)
            .map_err(|f| f.error.sentence)
    }

    /// [`Vault::write_files`] where nothing is replaced (a create, a replay), so nothing can be
    /// displaced.
    pub(super) fn write_note(
        &self,
        id: &str,
        content: &NoteText,
        journal: Option<&Record>,
        text_ok: impl FnOnce(Option<&str>) -> bool,
        meta_ok: impl FnOnce(&str) -> bool,
    ) -> Result<Saved, SaveFailed> {
        self.write_files(id, content, journal, text_ok, meta_ok).map(|(saved, _)| saved).map_err(|not| not.0)
    }

    /// The text and the sidecar, under the journal when one is given (a recovery writes without:
    /// it is the journal being replayed).
    fn write_files(
        &self,
        id: &str,
        content: &NoteText,
        journal: Option<&Record>,
        text_ok: impl FnOnce(Option<&str>) -> bool,
        meta_ok: impl FnOnce(&str) -> bool,
    ) -> Result<Wrote, NotWrote> {
        let before = |failed: SaveFailed| Box::new((failed, None));
        validate(&content.text).map_err(|e| before(SaveFailed::wrote_nothing(e)))?;
        if content.meta.len() > META_LIMIT {
            return Err(before(SaveFailed::wrote_nothing("Metadata exceeds 8 KiB.")));
        }
        let (text_path, meta_path) = self.files(id).map_err(|e| before(SaveFailed::wrote_nothing(e)))?;
        // Write-ahead recovery survives a crash between the text and the sidecar.
        if let Some(record) = journal {
            let json = record.json().map_err(|e| before(SaveFailed::wrote_nothing(e)))?;
            self.put(&self.journal(), &json, true).map_err(|e| before(SaveFailed::wrote_nothing(e.error)))?;
        }
        let on_disk = read(&text_path, LIMIT).map_err(|e| before(SaveFailed::wrote_nothing(e)))?;
        let meta_on_disk = read(&meta_path, META_LIMIT).map_err(|e| before(SaveFailed::wrote_nothing(e)))?;
        if !text_ok(on_disk.as_deref()) || !meta_ok(meta_on_disk.as_deref().unwrap_or_default()) {
            let error = SaveError::conflict(CHANGED_ON_DISK);
            return Err(before(SaveFailed { error, ..SaveFailed::wrote_nothing("") }));
        }
        let written = Fingerprint::of_bytes(content.text.as_bytes());
        // A file there passed the check, so it is one this save may replace, by an exchange that
        // hands back what it took the place of; with none, a create that a file made meanwhile
        // by anyone else makes fail rather than be replaced, so it displaces nothing.
        let displaced = if on_disk.is_some() {
            let exchanged = self.exchange(&text_path, &content.text).map_err(|e| {
                before(SaveFailed::after(e.step.durability(), &written, e.error))
            })?;
            if let Some(e) = exchanged.unsure {
                // In place, its durability unknown: not called saved, and what it displaced kept.
                return Err(Box::new((SaveFailed::after(Durability::Unknown, &written, e.error), Some(exchanged.displaced))));
            }
            Some(exchanged.displaced)
        } else {
            self.put(&text_path, &content.text, false)
                .map_err(|e| before(SaveFailed::after(e.step.durability(), &written, e.error)))?;
            None
        };
        if let Err((durability, sentence)) = self.write_meta(&meta_path, &content.meta) {
            let failed = SaveFailed {
                meta_unsure: (durability == Durability::Unknown).then(|| content.meta.clone()),
                ..SaveFailed::after(durability, &written, sentence)
            };
            return Err(Box::new((failed, displaced)));
        }
        if journal.is_some() {
            // Everything landed; only the journal is left. Kept, it replays as an identical note
            // at the next start, which is dropped; but the save is not called durable.
            if let Err(sentence) = self.remove(&self.journal()) {
                let failed = SaveFailed {
                    meta_unsure: Some(content.meta.clone()),
                    ..SaveFailed::after(Durability::Unknown, &written, sentence)
                };
                return Err(Box::new((failed, displaced)));
            }
        }
        let saved = Saved { content: content.clone(), written, modified: now(), witnessed: Witnessed::Same, theirs: None };
        Ok((saved, displaced))
    }

    /// The sidecar, written when there is something to say or a sidecar to bring up to date.
    /// A failure says how much of the save landed: the text did, so it is at least content-only.
    fn write_meta(&self, path: &Path, meta: &str) -> Result<(), (Durability, String)> {
        let exists = path.exists();
        if meta.is_empty() && !exists {
            return Ok(());
        }
        self.put(path, meta, exists).map_err(|e| {
            let durability = match e.step.durability() {
                Durability::NotWritten => Durability::ContentOnly,
                other => other,
            };
            (durability, e.error)
        })
    }
}
