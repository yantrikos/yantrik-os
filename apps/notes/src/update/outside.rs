//! A change made to the open note's file outside Notes, and the person's choice about it (design
//! §3, "Conflicting outside edit"). The document classifies what the watch and the store's check
//! find; this module says what that asks of the person and carries out the choice:
//!
//! - *Keep mine as a copy* (*Save as new* when the file is gone): the window's text becomes a new
//!   note and the window shows it. The file another program wrote is left as it is.
//! - *Take theirs*: the window's text is kept as a recovery note first, then the file's text
//!   replaces it and the view's undo history starts again.
//! - *Compare*: the two side by side, read-only.
//! - *Not now*: the bar is put away and editing goes on; saving stays refused until a choice.
//!
//! Both resolutions write the window's text as a note of its own before anything else, so no path
//! here can lose it. They pass the library's limits: a one-off recovery is not growth, and
//! refusing it would leave a person whose library is full no way out of the conflict but to lose
//! their text. A change to a note with nothing unsaved asks nothing: it is followed (follow.rs).
//!
//! A note two windows show has one conflict: both show the bar, and a choice made in either
//! resolves it for both. *Keep mine* moves every window on it to the kept note; *Take theirs*
//! starts every window's undo history again.

use yantrik_app::documents::{DocumentId, Fingerprint, Observed, OutsideState};
use yantrik_app::effects::Effects;

use super::navigate::show_note;
use super::{after_store, begin, drafts, failed, follow, leaving, refreshed, relist, said, save_state, set_status, show, windows};
use crate::command::{Command, Resolution};
use crate::model::{Current, Model, Work};
use crate::note::{now, Note, NoteText};
use crate::store::Resolved;
use crate::view::render::{CHROME, OUTSIDE, STATUS};
use crate::Notes;

/// The dialog number of the comparison.
pub const COMPARE_DIALOG: i32 = 3;

/// The watch saw an open note's file change. The document classifies it (an outside edit, a
/// conflict, a deletion, or our own write). A conflict or a deletion is offered at once, and a
/// change after *Not now* brings the bar back: it is news the person has not seen.
pub fn seen(m: &mut Model, fx: &mut Effects<Notes>, key: &str, seen: Observed) {
    let Some(d) = m.documents.by_key(key).and_then(|id| m.documents.get_mut(id)) else { return };
    // A file back after it was found gone, with nothing typed since: the note is as clean as it
    // was, so what came back is read again (or is the same), not a conflict over no edits.
    let back = d.outside == OutsideState::Deleted && matches!(seen, Observed::File(_));
    if let Some(file) = m.files.get_mut(&d.id).filter(|f| back && f.gone_at == Some(d.seq)) {
        d.saved_seq = d.seq;
        file.gone_at = None;
    }
    let before = d.outside.clone();
    let after = d.observe(seen).clone();
    if let Some((key, path, seen)) = d.rewatch() {
        fx.watch(&key, path, seen);
    }
    if after != before {
        // News for every window on the note, *Not now* or not.
        let doc = d.id;
        if before == OutsideState::Deleted {
            leaving::no_longer_gone(m, fx, doc);
        }
        for window in m.windows_showing(doc) {
            if let Some(pane) = m.pane_of_mut(window) {
                pane.outside_later = false;
            }
        }
        set_status(m, save_state(m));
        fx.changed(STATUS);
    }
    fx.changed(OUTSIDE);
    if matches!(after, OutsideState::Changed { .. }) {
        follow::follow(m, fx);
    }
}

/// Why the open note has nothing for a resolution to do, or `None` when it may go ahead. `theirs`
/// is for the choices that need the file's version (take it, compare with it).
pub fn unresolvable(current: Option<Current<'_>>, theirs: bool) -> Option<&'static str> {
    let Some(c) = current else { return Some("No note is open, so there is nothing to resolve.") };
    match c.outside() {
        OutsideState::Conflict { .. } => None,
        OutsideState::Deleted if !theirs => None,
        OutsideState::Deleted => Some(
            "The open note's file was moved or deleted, so there is no version on disk to take or compare. \
             Save as new (`keep_mine_as_copy`) keeps this text as a new note.",
        ),
        OutsideState::Clean | OutsideState::Changed { .. } => Some(
            "Nothing to resolve: the open note's file has not been changed outside Notes under unsaved \
             edits, and it is still there.",
        ),
    }
}

/// Keeps the window's text as a note of its own, then reads the file's version. What happens to
/// the window is [`resolved`]'s, once both are known.
pub fn resolve(m: &mut Model, fx: &mut Effects<Notes>, resolution: Resolution) {
    if let Some(sentence) = unresolvable(m.current(), resolution == Resolution::TakeTheirs) {
        return say(m, fx, sentence);
    }
    let Some(c) = m.current() else { return };
    let (doc, id, mine, seq) = (c.doc.id, c.id().to_string(), c.content().clone(), c.doc.seq);
    let kept_id = resolution.kept().new_id();
    let lost = Command::Resolved { doc, seq, resolution, result: Err(said::JOB_LOST.into()) };
    begin(m, fx, Work::Resolve, lost, move |vault| Command::Resolved {
        doc,
        seq,
        resolution,
        result: vault.keep_beside(&id, &kept_id, &mine).map(Box::new),
    });
}

/// The window's text is kept as a note (or could not be), and the file's version is known.
pub fn resolved(
    m: &mut Model,
    fx: &mut Effects<Notes>,
    doc: DocumentId,
    seq: u64,
    resolution: Resolution,
    result: Result<Box<Resolved>, String>,
) {
    m.busy = None;
    let resolved = match result {
        Ok(resolved) => *resolved,
        Err(e) => return failed(m, fx, Some(doc), said::not_kept(&e)),
    };
    m.library.push(resolved.kept.clone());
    if let Some(asked) = m.asked.as_mut() {
        asked.kept = Some(resolved.kept.id.clone());
    }
    // The note as the window holds it now: the person may have typed while the job ran.
    let latest = m.current().filter(|c| c.doc.id == doc).map(|c| (c.content().clone(), c.doc.seq));
    match (resolution, resolved.theirs, latest) {
        // Nothing else can be shown while the store works; the kept note is listed either way.
        (_, _, None) => relist(m, fx),
        (Resolution::TakeTheirs, Ok(Some(theirs)), Some((_, at))) if at == seq => {
            take_theirs(m, fx, doc, theirs, said::TOOK_THEIRS)
        }
        (Resolution::TakeTheirs, Ok(Some(_)), Some(_)) => stay(m, fx, said::TYPED_MEANWHILE.into()),
        (Resolution::TakeTheirs, Err(e), Some(_)) => stay(m, fx, said::theirs_unread(&e)),
        (Resolution::TakeTheirs, Ok(None), Some((latest, _))) => {
            switch_to_kept(m, fx, doc, resolved.kept, Ok(None), latest, said::NOTHING_TO_TAKE)
        }
        (Resolution::KeepMine, theirs, Some((latest, _))) => {
            let sentence = if matches!(theirs, Ok(None)) { said::SAVED_AS_NEW } else { said::KEPT_MINE };
            switch_to_kept(m, fx, doc, resolved.kept, theirs, latest, sentence)
        }
    }
    after_store(m, fx);
}

/// The file's text replaces the window's (already kept as a recovery note), from a baseline the
/// watch starts again at, with the view's undo history cleared.
pub(super) fn take_theirs(m: &mut Model, fx: &mut Effects<Notes>, doc: DocumentId, theirs: NoteText, sentence: &str) {
    let Some(d) = m.documents.get_mut(doc) else { return };
    d.take_theirs(theirs.clone(), Fingerprint::of_bytes(theirs.text.as_bytes()));
    if let Some((key, path, seen)) = d.watch_entry() {
        fx.watch(&key, path, seen);
    }
    let Some(file) = m.files.get_mut(&doc) else { return };
    file.on_disk = Some(theirs.clone());
    file.meta_unsure = None;
    // The other program's write is recent; Refresh reads its exact time from the file.
    file.modified = now();
    let (id, modified) = (file.id.clone(), file.modified);
    if let Some(n) = m.library.iter_mut().find(|n| n.id == id && !n.trash) {
        *n = Note { id, content: theirs, modified, trash: false, stored: true };
    }
    shown_resolved(m, fx, sentence);
    // Every other window on the note shows the file's text too, its undo history started again.
    windows::follow(m, fx, true);
}

/// The window moves to the kept note. The original is listed as its files hold it now, or not at
/// all when its file is gone; anything typed while the job ran goes on as an unsaved edit of the
/// kept note, saved by the autosave like any other. The original's draft goes: its text is the kept
/// note's now, and a start must not offer it again (`drafts::moved`).
fn switch_to_kept(
    m: &mut Model,
    fx: &mut Effects<Notes>,
    doc: DocumentId,
    kept: Note,
    theirs: Result<Option<NoteText>, String>,
    latest: NoteText,
    sentence: &str,
) {
    if let Some(id) = m.files.get(&doc).map(|f| f.id.clone()) {
        match theirs {
            Ok(Some(text)) => {
                if let Some(n) = m.library.iter_mut().find(|n| n.id == id && !n.trash) {
                    n.content = text;
                }
            }
            Ok(None) => m.library.retain(|n| n.id != id || n.trash),
            // Listed as it was; Refresh reads it again.
            Err(_) => {}
        }
    }
    let typed_meanwhile = latest != kept.content;
    let from = m.documents.get(doc).and_then(drafts::kept_name);
    // The kept note is a write of ours: another program replacing it keeps it again (follow.rs).
    m.written.insert(kept.id.clone(), Fingerprint::of_bytes(kept.content.text.as_bytes()));
    show_note(m, fx, Some(kept.clone()));
    if typed_meanwhile {
        if let Some((d, _)) = m.current_mut() {
            d.edit(|c| *c = latest);
        }
    }
    // The text moved to the kept note in every window that showed it.
    windows::follow_to(m, fx, doc, Some(&kept));
    drafts::moved(m, fx, from);
    shown_resolved(m, fx, sentence);
    if m.unsaved() {
        refreshed(m, fx);
    }
}

/// A resolution landed: the comparison closes, the bar goes, and the window shows its note afresh.
fn shown_resolved(m: &mut Model, fx: &mut Effects<Notes>, sentence: &str) {
    if m.pane.dialog == COMPARE_DIALOG {
        m.pane.dialog = 0;
    }
    m.pane.theirs = None;
    m.pane.outside_later = false;
    show(m, fx);
    fx.changed(CHROME);
    fx.changed(OUTSIDE);
    say(m, fx, sentence);
}

/// Nothing was replaced; the window keeps its note, in conflict, and says why.
fn stay(m: &mut Model, fx: &mut Effects<Notes>, sentence: String) {
    set_status(m, save_state(m));
    relist(m, fx);
    say(m, fx, &sentence);
}

/// Reads the file's version for a side-by-side comparison.
pub fn compare(m: &mut Model, fx: &mut Effects<Notes>) {
    if let Some(sentence) = unresolvable(m.current(), true) {
        return say(m, fx, sentence);
    }
    let Some(c) = m.current() else { return };
    let (doc, id) = (c.doc.id, c.id().to_string());
    let lost = Command::Compared { doc, result: Err(said::JOB_LOST.into()) };
    begin(m, fx, Work::Compare, lost, move |vault| Command::Compared { doc, result: vault.read_note(&id) });
}

/// The file's version is read: the comparison opens, read-only. A file found gone is a deletion
/// the document learns of now, without waiting for the watch.
pub fn compared(m: &mut Model, fx: &mut Effects<Notes>, doc: DocumentId, result: Result<Option<NoteText>, String>) {
    m.busy = None;
    let theirs = match result {
        Ok(theirs) => theirs,
        Err(e) => return failed(m, fx, None, said::compare_unread(&e)),
    };
    if theirs.is_none() {
        if let Some(d) = m.documents.get_mut(doc) {
            d.observe(Observed::Gone);
        }
    }
    if m.current_id() == Some(doc) {
        m.pane.theirs = theirs;
        m.pane.dialog = COMPARE_DIALOG;
        fx.changed(CHROME);
    }
    if let Some(asked) = m.asked.as_mut() {
        asked.landed = true;
    }
    set_status(m, save_state(m));
    fx.changed(STATUS);
    fx.changed(OUTSIDE);
    after_store(m, fx);
}

/// *Not now*: the bar is put away in this window; the next save the person asks for is refused and
/// brings it back.
pub fn not_now(m: &mut Model, fx: &mut Effects<Notes>) {
    m.pane.outside_later = true;
    fx.changed(OUTSIDE);
}

/// A sentence of this module's in the notice, in place of whatever was there, a lasting one too.
pub(super) fn say(m: &mut Model, fx: &mut Effects<Notes>, sentence: &str) {
    m.notice = sentence.into();
    m.lasting_notice = false;
    fx.changed(STATUS);
}
