//! Saving a note through its document: one save in flight, of any note, the edits made meanwhile
//! in one follow-up, a failed save not retried by the autosave until something changes and said
//! with how much of it reached the disk, and a file changed outside never written over: the person
//! chooses what happens to the two versions first (outside.rs). Another program's version a save
//! replaced is kept by the store and said here.
//!
//! A save that lands shows the disk takes writes again, so a note whose save failed before is then
//! saved again, one at a time, with no key pressed in it ([`retry_failed`]); its failure stays said
//! until it is on disk.

use yantrik_app::documents::{
    BeginSave, DocumentId, Durability, Fingerprint, SaveError, SaveOutcome, SaveState, SaveTicket,
};
use yantrik_app::effects::Effects;

use super::{
    after_store, begin, drafts, failed, refreshed, relist, said, save_state, set_status, AUTOSAVE, AUTOSAVE_AFTER,
};
use crate::command::Command;
use crate::model::{Model, Work};
use crate::note::{Note, NoteText};
use crate::store::{SaveFailed, SaveJob, Saved, Theirs};
use crate::view::render::{OUTSIDE, STATUS};
use crate::Notes;

/// Who asked for a save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// The debounce: a save that failed is not tried again until the note changes.
    Autosave,
    /// The person, a close, or an action waiting on it: a save that failed is tried again.
    Asked,
}

/// The autosave's debounce ran out: every note with edits not on disk is saved, each in a window
/// that shows it, the focused window's first. The store does one thing at a time, so one save
/// starts now; its landing arms the debounce again (`refreshed`), and the next is saved then.
pub fn autosave(m: &mut Model, fx: &mut Effects<Notes>) {
    let focused = m.current_id();
    let others = m.documents.iter().map(|d| d.id).filter(|id| Some(*id) != focused);
    let order: Vec<DocumentId> = focused.into_iter().chain(others).collect();
    for doc in order {
        if m.unsaved_doc(doc) {
            save_document(m, fx, doc, How::Autosave);
        }
        if m.busy.is_some() {
            return;
        }
    }
}

/// Saves `doc` in a window that shows it, if it has edits not on disk.
pub fn save_document(m: &mut Model, fx: &mut Effects<Notes>, doc: DocumentId, how: How) {
    if let Some(window) = m.windows_showing(doc).first().copied() {
        m.within(window, |m| save(m, fx, how));
    }
}

/// Saves the focused window's note, if it has edits not on disk.
pub fn save(m: &mut Model, fx: &mut Effects<Notes>, how: How) {
    let Some(doc) = m.current_id() else { return };
    // The store does one thing at a time. A save of this note already in flight takes the edits
    // as its follow-up below; a save of another note is followed by this one's once it lands.
    if m.busy.is_some_and(|w| w != Work::Save(doc)) {
        return;
    }
    fx.stop_timer(AUTOSAVE);
    if !m.ready || !m.current().is_some_and(|c| c.unsaved() && !c.trash()) {
        return;
    }
    m.keep_unsaved_dirty();
    let Some((d, _)) = m.current_mut() else { return };
    let begun = match how {
        How::Autosave => d.begin_save(),
        How::Asked => d.retry_save(),
    };
    match begun {
        BeginSave::Ticket(ticket) => spawn(m, fx, doc, ticket),
        // In flight: one follow-up is owed. Clean or failed: nothing to do until a change.
        BeginSave::InFlight | BeginSave::Clean | BeginSave::Failed => {}
        BeginSave::Blocked => blocked(m, fx, how),
    }
}

/// The file changed or went outside, and the person has not chosen yet. The autosave says so in
/// the footer only: the bar already asks, or the person put it away with *Not now*. A save the
/// person (or a close, or an action) asked for is refused in words and brings the bar back.
fn blocked(m: &mut Model, fx: &mut Effects<Notes>, how: How) {
    let Some((doc, sentence)) = m.current().map(|c| (c.doc.id, said::blocked(c.outside()))) else { return };
    match how {
        How::Autosave => {
            set_status(m, save_state(m));
            fx.changed(STATUS);
        }
        How::Asked => {
            m.pane.outside_later = false;
            fx.changed(OUTSIDE);
            failed(m, fx, Some(doc), sentence.into());
        }
    }
}

fn spawn(m: &mut Model, fx: &mut Effects<Notes>, doc: DocumentId, ticket: SaveTicket<NoteText>) {
    let Some(file) = m.files.get(&doc) else { return };
    let seq = ticket.seq;
    let alike = m.library.iter().filter(|n| !n.trash).map(|n| (n.id.clone(), n.content.text.len())).collect();
    let job = SaveJob {
        id: file.id.clone(),
        on_disk: file.on_disk.clone(),
        meta_unsure: file.meta_unsure.clone(),
        modified: file.modified,
        ticket,
        alike,
    };
    // A save that ended without an answer may have written anything: not called saved.
    let written = Fingerprint::of_bytes(job.ticket.content.text.as_bytes());
    let lost = Command::SaveLanded {
        doc,
        seq,
        result: Err(SaveFailed {
            error: SaveError::after_writing(Durability::Unknown, written, said::JOB_LOST),
            ..SaveFailed::wrote_nothing("")
        }),
    };
    begin(m, fx, Work::Save(doc), lost, move |vault| Command::SaveLanded { doc, seq, result: vault.save(&job) });
}

/// A save of `doc` at `seq` landed.
pub fn landed(m: &mut Model, fx: &mut Effects<Notes>, doc: DocumentId, seq: u64, result: Result<Saved, SaveFailed>) {
    if m.busy == Some(Work::Save(doc)) {
        m.busy = None;
    }
    // Another program's version the replace took the place of is in the library, or beside it,
    // and said, whatever else this landing turns out to be: never kept and left unsaid.
    let theirs = match &result {
        Ok(saved) => saved.theirs.clone(),
        Err(failure) => failure.theirs.clone(),
    };
    let theirs = theirs.map(|theirs| kept(m, fx, *theirs));
    if let Some(sentence) = &theirs {
        m.notice = sentence.clone();
        m.lasting_notice = true;
        fx.changed(STATUS);
    }
    let Some(d) = m.documents.get_mut(doc) else { return };
    match result {
        Ok(saved) => {
            let outcome = d.save_landed_witnessed(seq, Ok(saved.written.clone()), saved.witnessed);
            let SaveOutcome::Saved { follow_up, replaced } = outcome else { return };
            // From the new baseline: a look the watch took before the save is dropped, and our
            // own write is never reported as an outside edit.
            if let Some((key, path, seen)) = d.watch_entry() {
                fx.watch(&key, path, seen);
            }
            let key = d.source.key();
            let Some(file) = m.files.get_mut(&doc) else { return };
            file.on_disk = Some(saved.content.clone());
            file.meta_unsure = None;
            file.modified = saved.modified;
            m.written.insert(file.id.clone(), saved.written.clone());
            let entry = Note { id: file.id.clone(), content: saved.content, modified: saved.modified, trash: false, stored: true };
            match m.library.iter_mut().find(|n| n.id == entry.id && !n.trash) {
                Some(n) => *n = entry,
                None => m.library.push(entry),
            }
            // Another program's write that this save replaced is said, and stays said while the
            // person types on (one kept by the store was said above, with where it is kept); a
            // failure's notice is answered by this landing, unless another note's save still
            // stands failed.
            match replaced {
                _ if theirs.is_some() => {}
                Some(what) => {
                    m.notice = said::replaced(what).into();
                    m.lasting_notice = true;
                }
                None if !m.lasting_notice && still_failed(m, doc).is_empty() => m.notice.clear(),
                None => {}
            }
            // On disk: a draft it took over from another note is not needed any more.
            drafts::released(m, fx, &key);
            refreshed(m, fx);
            relist(m, fx);
            if let Some(ticket) = follow_up {
                spawn(m, fx, doc, ticket);
            }
            after_store(m, fx);
            retry_failed(m, fx, doc);
        }
        Err(failure) => {
            let conflict = failure.error.conflict;
            let mut sentence = if failure.read_only {
                failure.error.sentence.clone()
            } else {
                said::save_failed(&failure.error)
            };
            // The replace went in before a later step failed, and took the place of another
            // program's version: where that is kept comes first.
            if let Some(kept) = &theirs {
                sentence = format!("{kept} {sentence}");
            }
            let Some(d) = m.documents.get_mut(doc) else { return };
            if matches!(d.save_landed(seq, Err(failure.error)), SaveOutcome::Stale) {
                return;
            }
            if conflict {
                // The store's check found the file changed: the person chooses, as for the watch.
                sentence = said::blocked(&d.outside).into();
                m.pane.outside_later = false;
            }
            if let (Some(file), Some(meta)) = (m.files.get_mut(&doc), failure.meta_unsure) {
                file.meta_unsure = Some(meta);
            }
            // A sighting the save was holding is judged now, and may be the bar's news.
            fx.changed(OUTSIDE);
            failed(m, fx, Some(doc), sentence);
            // Another note's edits that waited behind this save are still saved at the next tick;
            // this one is not tried again by the autosave until it changes.
            if m.documents.iter().any(|d| d.id != doc && m.unsaved_doc(d.id)) {
                fx.after(AUTOSAVE, AUTOSAVE_AFTER, Command::Autosave);
            }
        }
    }
}

/// The notes other than `except` whose last save failed and whose edits are still not on disk.
fn still_failed(m: &Model, except: DocumentId) -> Vec<DocumentId> {
    m.documents
        .iter()
        .filter(|d| d.id != except && matches!(d.save, SaveState::Failed { .. }))
        .map(|d| d.id)
        .filter(|id| m.unsaved_doc(*id))
        .collect()
}

/// A save of `saved` just landed, so the disk takes writes again: the next note whose save failed
/// is saved again, in a window that shows it, as the person's own Save would be, rather than left
/// off disk until it is edited. One at a time, and only on a success: a retry that fails ends the
/// retries, so none is tried over and over. Not while a close or an action waits on the store
/// (each saves what it needs first), nor for a note whose file changed outside (the person
/// chooses first).
fn retry_failed(m: &mut Model, fx: &mut Effects<Notes>, saved: DocumentId) {
    if m.busy.is_some() || m.closing.is_some() || m.pending.is_some() {
        return;
    }
    let next = still_failed(m, saved).into_iter().find(|id| m.documents.get(*id).is_some_and(|d| !d.blocked()));
    if let Some(next) = next {
        save_document(m, fx, next, How::Asked);
    }
}

/// Another program's version a save replaced, now in the library or beside it, and the sentence
/// that says where. One that could not be written is kept as a draft under its own note's name,
/// so a start offers it back even if Notes stops before it is saved, and the close prompt counts
/// it while it is not on disk.
fn kept(m: &mut Model, fx: &mut Effects<Notes>, theirs: Theirs) -> String {
    match theirs {
        Theirs::Kept(note) => {
            let title = note.title();
            m.library.push(note);
            said::theirs_kept(&title)
        }
        Theirs::Same(id) => {
            let title = m.library.iter().find(|n| n.id == id).map(|n| n.title()).unwrap_or_default();
            said::theirs_held(&title)
        }
        Theirs::Aside(path) => said::theirs_aside(&path),
        Theirs::Unwritten { note, why } => {
            let title = note.title();
            if let Some((name, bytes)) = drafts::envelope(&m.vault.dir().join(&note.id), &note.content, 1, None) {
                fx.checkpoint_draft(&name, bytes);
            }
            m.library.push(note);
            said::theirs_unwritten(&title, &why)
        }
        Theirs::Lost(why) => said::theirs_lost(&why),
    }
}
