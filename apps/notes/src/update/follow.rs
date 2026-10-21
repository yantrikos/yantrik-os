//! A note with nothing unsaved follows its file (decided in review of #915): another program's
//! change is read again and shown, with a quiet line saying so, the way *Take theirs* shows a
//! version (the undo history starts again).
//!
//! Three guards keep this from costing the person anything:
//! - A note with unsaved edits is never replaced this way. A key typed before the read lands
//!   makes it a conflict, judged by the document, and the person chooses (outside.rs).
//! - When what the window holds is our own last write from this run (`Model::written`, kept
//!   across a Refresh and a reopening), it is kept as a recovered note before it is replaced.
//!   Another program writing from a stale copy would otherwise leave no copy of the person's last
//!   version anywhere, since the undo history is cleared too. Not when a live note's file holds
//!   exactly that text now (read by the job, not taken from the library as last loaded): a
//!   second copy would add nothing.
//! - When that version cannot be kept, it is not replaced, and it is held as a conflict against
//!   the file (#915 review finding 5): the bar offers the choices, and every way out of the note
//!   saves first, which the conflict refuses, so a Refresh or an Open cannot drop it unsaid.

use yantrik_app::documents::{DocumentId, Fingerprint, Observed, OutsideState};
use yantrik_app::effects::Effects;

use super::outside::{say, take_theirs};
use super::{after_store, begin, relist, said, save_state, set_status};
use crate::command::{Command, Resolution};
use crate::model::{Model, Work};
use crate::note::NoteText;
use crate::store::Reread;
use crate::view::render::{OUTSIDE, STATUS};
use crate::Notes;

/// The open note's file changed outside and the note has nothing unsaved: it is read again, and
/// [`reread`] shows it. Edits made since the change was seen make it the conflict it now is. While
/// the store is busy, the read waits for it (`reread_owed`).
pub fn follow(m: &mut Model, fx: &mut Effects<Notes>) {
    let Some(c) = m.current() else { return };
    let OutsideState::Changed { now } = c.outside().clone() else { return };
    let (doc, id, unsaved) = (c.doc.id, c.id().to_string(), c.unsaved());
    // Notes that held the text when last read; the job reads their files to know if one still does.
    let alike = m.alike(c.content(), c.id());
    let ours = m.ours(c.file).then(|| {
        let kept_id = Resolution::TakeTheirs.kept().new_id();
        (kept_id, c.content().clone())
    });
    if unsaved {
        if let Some(d) = m.documents.get_mut(doc) {
            d.observe(Observed::File(now));
        }
        set_status(m, save_state(m));
        fx.changed(STATUS);
        fx.changed(OUTSIDE);
        return;
    }
    if m.busy.is_some() {
        m.pane.reread_owed = true;
        return;
    }
    let lost = Command::Reread { doc, result: Err(said::JOB_LOST.into()) };
    begin(m, fx, Work::Reread, lost, move |vault| {
        let ours = ours.as_ref().map(|(kept_id, text)| (kept_id.as_str(), text));
        Command::Reread { doc, result: Ok(Box::new(vault.reread(&id, ours, &alike))) }
    });
}

/// The changed note, read again (and our last write kept first, when it was ours). Shown in place
/// of what the window held when nothing there is unsaved; a key typed while the read ran makes it
/// a conflict instead, so nothing typed is replaced. A file found gone is a deletion. When our
/// write could not be kept, nothing is replaced, and the window's text is held as a conflict.
pub fn reread(m: &mut Model, fx: &mut Effects<Notes>, doc: DocumentId, result: Result<Box<Reread>, String>) {
    m.busy = None;
    let still_changed =
        m.current().is_some_and(|c| c.doc.id == doc && matches!(c.outside(), OutsideState::Changed { .. }));
    // The job ended without an answer: whether ours was kept is not known, nor what the file holds.
    let Reread { kept, theirs } = result.map(|r| *r).unwrap_or_else(|e| Reread { kept: Err(e.clone()), theirs: Err(e) });
    let kept = match kept {
        Ok(kept) => kept,
        Err(e) => {
            match theirs {
                // Typed on while the job ran: a conflict like any other, judged by the document.
                Ok(theirs) if still_changed && m.unsaved() => observe(m, fx, doc, theirs.as_ref()),
                theirs if still_changed => hold(m, fx, doc, theirs),
                _ => {}
            }
            say(m, fx, &said::ours_not_kept(&e));
            return settle(m, fx);
        }
    };
    let sentence = if kept.is_some() { said::UPDATED_KEEPING_OURS } else { said::UPDATED_FROM_DISK };
    if let Some(kept) = kept {
        m.library.push(kept);
        relist(m, fx);
    }
    match theirs {
        Err(e) => say(m, fx, &said::reread_failed(&e)),
        Ok(_) if !still_changed => {}
        Ok(Some(theirs)) if !m.unsaved() => take_theirs(m, fx, doc, theirs, sentence),
        Ok(theirs) => observe(m, fx, doc, theirs.as_ref()),
    }
    settle(m, fx);
}

/// What the read found, judged by the document: a conflict under edits typed meanwhile, or a
/// deletion.
fn observe(m: &mut Model, fx: &mut Effects<Notes>, doc: DocumentId, theirs: Option<&NoteText>) {
    let seen = match theirs {
        Some(t) => Observed::File(Fingerprint::of_bytes(t.text.as_bytes())),
        None => Observed::Gone,
    };
    if let Some(d) = m.documents.get_mut(doc) {
        d.observe(seen);
    }
    fx.changed(OUTSIDE);
}

/// Our last version could not be kept, so it is not replaced: it stays in the window, unsaved
/// against what the file now holds, as a conflict the person resolves. `theirs` is the file as
/// the job read it; one it could not read leaves what the file holds unknown.
fn hold(m: &mut Model, fx: &mut Effects<Notes>, doc: DocumentId, theirs: Result<Option<NoteText>, String>) {
    let (Some(d), Some(file)) = (m.documents.get_mut(doc), m.files.get_mut(&doc)) else { return };
    match theirs {
        Ok(Some(theirs)) => {
            d.outside = OutsideState::Conflict { now: Some(Fingerprint::of_bytes(theirs.text.as_bytes())) };
            file.on_disk = Some(theirs);
        }
        Ok(None) => {
            d.outside = OutsideState::Deleted;
            file.on_disk = None;
        }
        Err(_) => {
            d.outside = OutsideState::Conflict { now: None };
            file.on_disk = None;
        }
    }
    // The text is now not what the file holds: an edit of its document, so it is dirty as well
    // as unsaved, and its draft is kept.
    d.edit(|_| ());
    // The bar comes back in every window on the note.
    for window in m.windows_showing(doc) {
        if let Some(pane) = m.pane_of_mut(window) {
            pane.outside_later = false;
        }
    }
    fx.changed(OUTSIDE);
}

fn settle(m: &mut Model, fx: &mut Effects<Notes>) {
    set_status(m, save_state(m));
    fx.changed(STATUS);
    after_store(m, fx);
}
