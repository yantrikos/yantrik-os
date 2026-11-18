//! What a start offers back (crate::recovery), and the person's choice about it: *Restore* puts
//! the text back into its note, *Discard* keeps the note as its file holds it.
//!
//! The text goes back into its note as edits, saved by the autosave like any other, only when the
//! note's file is still the one the draft was written over. When the file moved on since (another
//! program wrote it), or is gone, putting it back is the outside-change flow's: the note opens
//! with the text as unsaved edits and the choices the bar offers (*Keep mine as a copy*, *Take
//! theirs*, *Compare*; *Save as new* for a file gone), so neither version is lost.
//!
//! Until the person chooses, the note an offer is for takes no edits: its draft is the only copy
//! of the text offered, and a draft of new edits would be written over it.

use yantrik_app::documents::draft::recover;
use yantrik_app::documents::{Candidate, Fingerprint, OutsideState, Recovered};
use yantrik_app::effects::Effects;

use super::navigate::{self, show_note};
use super::saving::{self, How};
use super::{refreshed, show};
use crate::command::{Act, Command, WindowAction};
use crate::model::Model;
use crate::note::{Kept, Note};
use crate::pane::Pane;
use crate::recovery::{base_of, Found, Offer};
use crate::store::Interrupted;
use crate::update::said;
use crate::view::render::{CONTENT, FIELDS, OUTSIDE, STATUS};
use crate::Notes;

/// The library has been read for the first time: each draft found at start is weighed against
/// its note's file and the interrupted save the journal held, if it was the same note's. The
/// newer of draft and journal wins; one equal to the file is not offered. A draft that loses is
/// removed: the file already holds it, or the interrupted save, newer, came back as a note of
/// its own (`Vault::load`). A draft whose text moved to another note that holds it safely is
/// removed too (`held_elsewhere`): Notes stopped before its old draft was. Returns the sentence
/// that says how many are offered, when the bar's one is not all of them.
pub fn weigh(m: &mut Model, fx: &mut Effects<Notes>, interrupted: Option<&Interrupted>) -> Option<String> {
    let found = m.recovery.found.take()?;
    let with_drafts: Vec<String> = found.iter().map(|f| f.offer.id.clone()).collect();
    for Found { offer, at_ms, seq } in found {
        if held_elsewhere(m, &offer, &with_drafts) {
            fx.discard_draft(&offer.draft);
            continue;
        }
        let file = m.library.iter().find(|n| !n.trash && n.id == offer.id).map(|n| &n.content);
        let journal = interrupted
            .filter(|j| j.id == offer.id)
            .map(|j| Candidate { content: j.content.clone(), at_ms: j.at_ms, seq: None });
        let draft = Candidate { content: offer.content.clone(), at_ms, seq: Some(seq) };
        match recover(file, journal, Some(draft)) {
            Recovered::Draft(_) => m.recovery.offers.push(offer),
            Recovered::File | Recovered::Journal(_) => fx.discard_draft(&offer.draft),
        }
    }
    fx.changed(OUTSIDE);
    // The note the bar offers is opened, so the person sees the text before choosing; one whose
    // file is gone is described on the bar instead.
    if m.current().is_none() {
        show_first(m, fx);
    }
    let kept_aside = [
        said::drafts_unreadable(&std::mem::take(&mut m.recovery.unreadable)),
        said::drafts_elsewhere(&std::mem::take(&mut m.recovery.elsewhere)),
    ];
    // The bar names one note; the notice says when there are more.
    let more = (m.recovery.offers.len() > 1).then(|| said::offered(m.recovery.offers.len()));
    let parts: Vec<String> = kept_aside.into_iter().flatten().chain(more).collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Whether a draft's text is safe without it, so offering it would only let *Restore* make it twice.
/// A draft is never dropped when its text would then exist nowhere that survives:
///
/// - Another note's file holds exactly this text, and that note has no draft of its own in this
///   start. One that has could be restored over its file, and the text would be on no disk at all.
/// - A newer offer holds exactly this text, and the two are one moved text (`drafts::moved`): one
///   of them is a copy or recovered note (`note::Kept`) never saved, so the text was handed over
///   from the other's note, and a crash came before the old draft was removed. The hand-over is
///   kept only in memory (the framework's draft envelope has no room for it), so this is how a
///   start knows it. Two notes' drafts that merely match are both offered: discarding one must
///   never discard the other's edit.
fn held_elsewhere(m: &Model, offer: &Offer, with_drafts: &[String]) -> bool {
    let on_disk = m.alike(&offer.content, &offer.id).iter().any(|(id, _)| !with_drafts.contains(id));
    let never_saved_kept = |id: &str| Kept::of(id).is_some() && !m.library.iter().any(|n| n.id == id);
    let moved_with = |other: &Offer| never_saved_kept(&other.id) || never_saved_kept(&offer.id);
    on_disk || m.recovery.offers.iter().any(|o| o.content == offer.content && moved_with(o))
}

/// Opens the note of the offer the bar shows, if it is in the library: its text is drawn
/// read-only in the editor until the person chooses.
fn show_first(m: &mut Model, fx: &mut Effects<Notes>) {
    let Some(id) = shown(m).map(|o| o.id.clone()) else { return };
    let listed = m.library.iter().find(|n| n.id == id).map(|n| n.trash);
    if let Some(trash) = listed {
        navigate::open(m, fx, &id, trash);
    }
}

/// *Show*: the note of the offer the bar shows, opened so the unsaved text can be seen. Reached
/// through the guards, so the note that was open is saved first.
pub fn show_offer(m: &mut Model, fx: &mut Effects<Notes>) {
    if shown(m).is_none() {
        return say(m, fx, said::NOTHING_OFFERED);
    }
    show_first(m, fx);
}

/// Where the offer the bar shows stands: its note open (the text is drawn), its note in the
/// library and not open (it can be shown), or its note's file gone (it is described).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Showing {
    Open = 1,
    Listed = 2,
    Gone = 3,
}

/// Where the offer a window's bar shows stands, for that window.
pub fn showing_in<'a>(m: &'a Model, pane: &Pane) -> Option<(Showing, &'a Offer)> {
    let offer = shown_in(m, pane)?;
    let state = if m.current_in(pane).is_some_and(|c| c.id() == offer.id) {
        Showing::Open
    } else if m.library.iter().any(|n| n.id == offer.id) {
        Showing::Listed
    } else {
        Showing::Gone
    };
    Some((state, offer))
}

/// The offers, the one the bar shows first: the order `describe.recovery` lists them in.
pub fn in_order(m: &Model) -> Vec<&Offer> {
    let first = shown(m).map(|o| o.id.as_str());
    first
        .and_then(|id| m.recovery.offer_for(id))
        .into_iter()
        .chain(m.recovery.offers.iter().filter(|o| Some(o.id.as_str()) != first))
        .collect()
}

/// The offer the focused window's bar shows, the one Restore and Discard act on.
pub fn shown(m: &Model) -> Option<&Offer> {
    shown_in(m, &m.pane)
}

/// The offer a window's bar shows: its own note's, or else the newest whose note no window shows,
/// on one window's bar only: the focused window's, or, when that bar holds its own note's offer,
/// the lowest numbered window's whose bar is free. An offer appears in the window that shows its
/// note, moves with the person otherwise, and is never offered in two windows at once.
pub fn shown_in<'a>(m: &'a Model, pane: &Pane) -> Option<&'a Offer> {
    if let Some(offer) = own(m, pane) {
        return Some(offer);
    }
    let free = m.panes().filter(|p| own(m, p).is_none()).map(|p| p.window).min_by_key(|w| (*w != m.pane.window, *w));
    if free != Some(pane.window) {
        return None;
    }
    let open: Vec<String> = m.panes().filter_map(|p| m.current_in(p).map(|c| c.id().to_string())).collect();
    m.recovery.offers.iter().find(|o| !open.contains(&o.id))
}

/// The offer for the note a window shows, if it has one.
fn own<'a>(m: &'a Model, pane: &Pane) -> Option<&'a Offer> {
    m.current_in(pane).and_then(|c| m.recovery.offer_for(c.id()))
}

/// *Restore*: the offered text back in its note, open in the window. Reached through the guards,
/// so the note that was open is saved first.
pub fn restore(m: &mut Model, fx: &mut Effects<Notes>) {
    let Some(offer) = shown(m).cloned() else { return say(m, fx, said::NOTHING_OFFERED) };
    m.recovery.offers.retain(|o| o.id != offer.id);
    let on_disk = m.library.iter().find(|n| !n.trash && n.id == offer.id).cloned();
    let file = on_disk.as_ref().map(|n| Fingerprint::of_bytes(n.content.text.as_bytes()));
    let gone = file.is_none();
    // The draft's baseline is the note's text and sidecar as they were when it was written
    // (`recovery::base_of`): equal, and the person's own edits, the sidecar's included, go back
    // as they were; different, and another program changed the note since.
    let moved_on = base_of(on_disk.as_ref().map(|n| &n.content)) != offer.base;
    match on_disk {
        Some(note) => show_note(m, fx, Some(note)),
        // Gone: a note of its own again, not on disk until saved, as a note made here is.
        None => show_note(m, fx, Some(Note { id: offer.id.clone(), content: offer.content.clone(), ..Note::default() })),
    }
    show(m, fx);
    let Some((d, _)) = m.current_mut() else { return };
    d.edit(|c| *c = offer.content);
    if moved_on {
        // Never a silent replace of what the file now holds, nor a silent re-creation of a file
        // someone deleted: the person chooses, as for any outside change.
        d.outside = match file {
            Some(now) => OutsideState::Conflict { now: Some(now) },
            None => OutsideState::Deleted,
        };
    }
    fx.changed(CONTENT);
    fx.changed(FIELDS);
    fx.changed(OUTSIDE);
    let sentence = match (moved_on, gone) {
        (false, _) => said::RESTORED,
        // #917's review, F: no second version to choose between, only Save as new.
        (true, true) => said::RESTORED_GONE,
        (true, false) => said::RESTORED_BESIDE,
    };
    refreshed(m, fx);
    say(m, fx, &left(m, sentence));
    // Saved at once, as the person's own Save would be. One the outside bar holds back is
    // refused in its words, and waits on their choice.
    if !moved_on {
        saving::save(m, fx, How::Asked);
    }
}

/// *Discard*: the offered text is dropped and its draft removed; the note is as its file holds it,
/// or, its file gone, there is nothing left of it.
pub fn discard(m: &mut Model, fx: &mut Effects<Notes>) {
    let Some(offer) = shown(m).cloned() else { return say(m, fx, said::NOTHING_OFFERED) };
    m.recovery.offers.retain(|o| o.id != offer.id);
    fx.discard_draft(&offer.draft);
    fx.changed(OUTSIDE);
    // The window drew the offered text; it draws the note as its file holds it again.
    show(m, fx);
    let on_disk = m.library.iter().any(|n| !n.trash && n.id == offer.id);
    say(m, fx, &left(m, if on_disk { said::DISCARDED } else { said::DISCARDED_GONE }));
}

/// Whether `command` would change a note whose offer the person has not chosen about; if so, it
/// is refused, said, and the editor is put back.
pub fn refused(m: &mut Model, fx: &mut Effects<Notes>, command: &Command) -> bool {
    let Some(id) = m.current().map(|c| c.id().to_string()) else { return false };
    if m.recovery.offer_for(&id).is_none() {
        return false;
    }
    let changes = match command {
        Command::Edit(_) | Command::Metadata { .. } => true,
        Command::Window(action) => {
            matches!(action, WindowAction::Pin | WindowAction::Undo | WindowAction::Redo | WindowAction::Save)
        }
        Command::Surface(act) => matches!(
            act,
            Act::SetTitle(_) | Act::Append(_) | Act::SetContent(_) | Act::Notebook(_) | Act::Tags(_) | Act::Undo | Act::Redo
        ),
        _ => false,
    };
    if !changes {
        return false;
    }
    fx.changed(CONTENT);
    fx.changed(FIELDS);
    if let Command::Surface(_) = command {
        super::surface::refuse(m, fx, said::CHOOSE_FIRST.into());
    } else {
        say(m, fx, said::CHOOSE_FIRST);
    }
    true
}

/// `sentence`, and how many offers are still waiting.
fn left(m: &Model, sentence: &str) -> String {
    match m.recovery.offers.len() {
        0 => sentence.to_string(),
        n => format!("{sentence} {}", said::offered(n)),
    }
}

fn say(m: &mut Model, fx: &mut Effects<Notes>, sentence: &str) {
    m.notice = sentence.into();
    m.lasting_notice = false;
    fx.changed(STATUS);
}
