//! A Mind's draft: its own slot, apart from the person's (#1184).
//!
//! `compose` from a Mind never changes the person's kept or open draft, and never inherits its Cc
//! or Bcc. When the person has unsent text it refuses, and says why. The draft starts empty and is
//! kept, with the files it carries, in `mind_draft.json`, so a restart does not lose the file and
//! with it the refusal to send without it. Everything the composer does to a draft goes here while
//! it shows this one (`Composer::in_slot`).

use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use super::say;
use crate::model::{part, FileOp, Keep, Model};
use crate::state::{checked_attachment, clear_mind_draft, load_mind_draft, save_mind_draft, shelf_path, Draft, MindDraft};
use crate::Email;

/// What the composer says while a draft carries a file the mail service cannot send. It has a row
/// of its own that wraps (`email.slint`), so it fits at any width and tells the person why Send will
/// refuse before they press it.
pub const ATTACHED_STATUS: &str = "A file is attached to this draft. Email cannot send attachments yet, so it will not be sent from here.";

/// What the composer says on a draft Yantrik prepared that carries no file.
const PREPARED_STATUS: &str = "Draft prepared for you";

const PERSON_WRITING: &str = "You have an unsent message of your own, and a draft from Yantrik would not replace it. Send or discard yours first.";

/// Why a Mind's `compose` cannot start now: the person has text of their own in the way. Said only
/// when it is true: an emptied slot, or one nobody touched, is not the person's text. Saves still
/// in flight count, because the text in them is the person's and is not yet in `kept`.
fn person_has_text(m: &Model) -> Option<Refused> {
    let c = &m.composer;
    let kept = matches!(&c.kept, Ok(Some(d)) if !d.is_empty());
    let open_theirs = c.open && !c.in_slot && !c.text.is_empty();
    let saving = c.saving || matches!(c.queued, Some(FileOp::Save { .. }));
    let edited = matches!(&c.mind, Ok(Some(slot)) if slot.touched && !slot.text.is_empty())
        || (c.open && c.in_slot && edited_since_shown(m));
    if kept || open_theirs || saving || edited || c.unsent_at_start.is_some() || c.sending.is_some() {
        return Some(Refused::new("person_is_writing", PERSON_WRITING));
    }
    c.kept.as_ref().err().map(|e| Refused::new("kept_unreadable", format!("Your own saved draft could not be read, so a draft from Yantrik was not started: {e}")))
}

/// The window's text is no longer what the slot holds: the person has been typing in it.
fn edited_since_shown(m: &Model) -> bool {
    matches!(&m.composer.mind, Ok(Some(slot)) if slot.text != m.composer.text)
}

fn home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(Into::into)
}

/// `compose` from a Mind: a new draft in the Mind's slot, shown in the composer.
pub fn compose(m: &mut Model, to: String, subject: String, body: String, attach: String, commitment: String, fx: &mut Effects<Email>) -> Result<(), Refused> {
    if let Err(e) = &m.composer.mind {
        return Err(Refused::new("draft_unreadable", format!("The draft Yantrik prepared earlier could not be read, so it was not replaced: {e}")));
    }
    if let Some(refused) = person_has_text(m) {
        return Err(refused);
    }
    let attachments = if attach.trim().is_empty() {
        Vec::new()
    } else {
        vec![checked_attachment(&attach, home().as_deref()).map_err(|e| Refused::new("cannot_attach", e))?]
    };
    let commitment = Some(commitment.trim().to_string()).filter(|c| !c.is_empty());
    // A draft the person touched, waiting on its shelf, is theirs: nothing replaces it, and nothing
    // is changed when this is refused.
    let waiting = commitment.as_deref().and_then(|c| shelf_path(&m.files.mind_draft, c));
    if waiting.as_deref().is_some_and(shelf_is_theirs) {
        return Err(Refused::new("person_is_writing", PERSON_WRITING));
    }
    shelve_held(m, commitment.as_deref())?;
    let slot = MindDraft { text: Draft { to, subject, body, ..Draft::default() }, attachments, touched: false, commitment, generation: 0 };
    save_mind_draft(&m.files.mind_draft, &slot).map_err(|e| Refused::new("not_kept", format!("The draft could not be kept, and the earlier one was left as it was: {e}")))?;
    // Only now, with the new draft written, does the one it replaces go.
    if let Some(old) = waiting {
        remove_shelf(&old);
    }
    m.composer.mind = Ok(Some(slot));
    show(m, fx);
    Ok(())
}

/// A shelf the person must not lose: one they touched, or one that cannot be read to tell.
fn shelf_is_theirs(shelf: &std::path::Path) -> bool {
    match load_mind_draft(shelf) {
        Ok(None) => false,
        Ok(Some(d)) => d.touched,
        Err(_) => true,
    }
}

fn remove_shelf(shelf: &std::path::Path) {
    if let Err(e) = clear_mind_draft(shelf) {
        tracing::warn!(error = %e, "a replaced prepared draft's file could not be removed");
    }
}

/// Each promise keeps its own draft. The slot holds one at a time, so when a draft for another
/// promise comes, the one in the slot is written to its own file beside it and waits there, to be
/// opened by `open_prepared_for` or cleared by `forget_prepared`. This only writes: nothing is
/// removed here, so a failure leaves every draft where it was. It never writes over a shelf the
/// person touched.
fn shelve_held(m: &mut Model, incoming: Option<&str>) -> Result<Option<std::path::PathBuf>, Refused> {
    let Ok(Some(held)) = &m.composer.mind else { return Ok(None) };
    let Some(held_for) = held.commitment.as_deref().filter(|c| Some(*c) != incoming) else { return Ok(None) };
    let Some(shelf) = shelf_path(&m.files.mind_draft, held_for) else { return Ok(None) };
    if shelf_is_theirs(&shelf) {
        return Err(Refused::new("person_is_writing", PERSON_WRITING));
    }
    save_mind_draft(&shelf, held).map_err(|e| Refused::new("not_kept", format!("The draft Yantrik prepared earlier could not be kept, so a new one was not started: {e}")))?;
    if matches!(&m.composer.left_on_shelf, Some((s, _)) if *s == shelf) {
        m.composer.left_on_shelf = None;
    }
    Ok(Some(shelf))
}

/// Open the composer on the slot.
pub fn show(m: &mut Model, fx: &mut Effects<Email>) {
    let Ok(Some(slot)) = m.composer.mind.clone() else { return };
    // A notice about an earlier open (a failed one) is not about this one.
    super::clear_notice(m, fx);
    m.composer.open = true;
    m.composer.in_slot = true;
    m.composer.unsure = false;
    m.composer.closed_during_send = false;
    m.composer.text = slot.text;
    m.composer.attachments = slot.attachments.clone();
    m.composer.status = if slot.attachments.is_empty() { PREPARED_STATUS } else { ATTACHED_STATUS }.into();
    fx.changed(part::STATUS);
    fx.changed(part::COMPOSE_TEXT);
}

/// What the composer holds goes back to the slot: the text as the window has it, with the files.
/// A change the person made marks the draft as theirs.
pub fn park(m: &mut Model) {
    if !m.composer.in_slot {
        return;
    }
    let text = m.composer.text.clone();
    let Ok(Some(slot)) = &mut m.composer.mind else { return };
    if slot.text != text {
        slot.text = text;
        slot.touched = true;
        slot.generation = slot.generation.saturating_add(1);
    }
    // Cancel on a slot with nothing left in it discards it: there is nothing to keep.
    if slot.is_empty() {
        return discard(m);
    }
    let saved = save_mind_draft(&m.files.mind_draft, slot);
    let held_for = slot.commitment.clone();
    match saved {
        Ok(()) => drop_left_on_shelf(m, held_for.as_deref()),
        Err(e) => tracing::warn!(error = %e, "the draft Yantrik prepared could not be kept"),
    }
}

/// The copy a failed open left on a shelf is older than the slot once the slot has been written
/// since (`held_for` is the slot's promise, if it holds one): it goes, but only while it still loads
/// as exactly the draft recorded, so nothing the person changed there is removed. A record for
/// another promise is stale and is only forgotten.
fn drop_left_on_shelf(m: &mut Model, held_for: Option<&str>) {
    let Some((shelf, recorded)) = m.composer.left_on_shelf.take() else { return };
    if held_for.is_none() || recorded.commitment.as_deref() != held_for {
        return;
    }
    if matches!(load_mind_draft(&shelf), Ok(Some(on_shelf)) if on_shelf == recorded) {
        remove_shelf(&shelf);
    }
}

/// At start: a slot that holds promise C while a touched shelf for C exists. The higher `generation`
/// is the newer copy: a newer shelf is written to the slot first, atomically, and only then removed
/// (if that write fails both stay, and the session shows the shelf's text as a failed open would);
/// an older or identical shelf is removed; two different copies of one generation are both kept.
pub fn adopt_newer_shelf(m: &mut Model) {
    let Ok(Some(slot)) = &m.composer.mind else { return };
    let Some(shelf) = slot.commitment.as_deref().and_then(|c| shelf_path(&m.files.mind_draft, c)) else { return };
    let Ok(Some(other)) = load_mind_draft(&shelf) else { return };
    if !other.touched || other.commitment != slot.commitment {
        return;
    }
    match other.generation.cmp(&slot.generation) {
        std::cmp::Ordering::Less => remove_shelf(&shelf),
        std::cmp::Ordering::Equal if other == *slot => remove_shelf(&shelf),
        std::cmp::Ordering::Equal => tracing::warn!("two versions of a prepared draft, neither known to be newer: both kept"),
        std::cmp::Ordering::Greater => match save_mind_draft(&m.files.mind_draft, &other) {
            Ok(()) => {
                remove_shelf(&shelf);
                m.composer.mind = Ok(Some(other));
            }
            Err(e) => {
                tracing::warn!(error = %e, "a newer prepared draft on its shelf could not be moved into the slot");
                m.composer.mind = Ok(Some(other.clone()));
                m.composer.left_on_shelf = Some((shelf, other));
            }
        },
    }
}

/// The draft went: its slot is emptied.
pub fn discard(m: &mut Model) {
    let discarded = match std::mem::replace(&mut m.composer.mind, Ok(None)) {
        Ok(Some(d)) => Some(d),
        _ => None,
    };
    m.composer.in_slot = false;
    m.composer.attachments.clear();
    if let Err(e) = clear_mind_draft(&m.files.mind_draft) {
        tracing::warn!(error = %e, "the prepared draft's file could not be removed");
    }
    // Its own shelf, when it holds the very draft discarded, is a duplicate that would come back.
    drop_left_on_shelf(m, discarded.as_ref().and_then(|d| d.commitment.as_deref()));
    if let Some(d) = discarded {
        let shelf = d.commitment.as_deref().and_then(|c| shelf_path(&m.files.mind_draft, c));
        if let Some(shelf) = shelf.filter(|s| matches!(load_mind_draft(s), Ok(Some(on_shelf)) if on_shelf == d)) {
            remove_shelf(&shelf);
        }
    }
    // The slot is free: a touched draft waiting on a shelf comes into the entry.
    promote_waiting(m);
}

/// The person's entry "Draft Yantrik prepared for Maya": opens the prepared draft in the composer.
/// Plain Compose never does.
pub fn open_prepared(m: &mut Model, fx: &mut Effects<Email>) {
    if !matches!(m.composer.mind, Ok(Some(_))) {
        return say(m, fx, "There is no draft prepared by Yantrik to open.");
    }
    if m.composer.open && !m.composer.in_slot && !m.composer.text.is_empty() {
        return say(m, fx, "You have a message open. Cancel keeps it as a draft; then open the prepared one.");
    }
    show(m, fx);
}

/// The entry for one promise's draft: it comes into the slot, and the draft that was there goes to
/// its shelf. Nothing is lost either way; the person's own message in the composer is never moved.
pub fn open_prepared_for(m: &mut Model, commitment: &str, fx: &mut Effects<Email>) {
    let commitment = commitment.trim();
    if matches!(&m.composer.mind, Ok(Some(s)) if s.commitment.as_deref() == Some(commitment)) {
        return open_prepared(m, fx);
    }
    let Some(shelf) = shelf_path(&m.files.mind_draft, commitment) else { return say(m, fx, "There is no draft prepared by Yantrik to open.") };
    let waiting = match load_mind_draft(&shelf) {
        Ok(Some(d)) => d,
        Ok(None) => return say(m, fx, "There is no draft prepared by Yantrik to open."),
        Err(e) => return say(m, fx, format!("That prepared draft could not be read: {e}")),
    };
    if m.composer.open && !m.composer.in_slot && !m.composer.text.is_empty() {
        return say(m, fx, "You have a message open. Cancel keeps it as a draft; then open the prepared one.");
    }
    park(m);
    // A draft of the person's with no promise has no shelf to go to: it stays where it is.
    if matches!(&m.composer.mind, Ok(Some(h)) if h.commitment.is_none() && h.touched) {
        return say(m, fx, "You have changed the draft in the prepared slot, so it was not moved. Discard it first, then open the other.");
    }
    let shelved = match shelve_held(m, Some(commitment)) {
        Ok(s) => s,
        Err(refused) => return say(m, fx, refused.sentence),
    };
    // The shelf goes only once the slot holds what was on it. If the write fails, the draft that was
    // in the slot is on its shelf and the one being opened is still on its own.
    if let Err(e) = save_mind_draft(&m.files.mind_draft, &waiting) {
        // The shelf copy goes only when the slot file on disk holds the same draft: then a discard
        // or send of the slot cannot promote a duplicate. If park's own write failed too, the slot
        // file is older and the shelf holds the only copy of the newest text; a slot file that
        // cannot be read is never a copy, so the shelf stays then too.
        if let (Some(copy), Ok(Some(held))) = (shelved, m.composer.mind.clone()) {
            let same = match load_mind_draft(&m.files.mind_draft) {
                Ok(on_disk) => on_disk.as_ref() == Some(&held),
                Err(_) => false,
            };
            if same {
                remove_shelf(&copy);
            } else {
                // Kept as the copy this failed open left: the slot's draft is a later version of it,
                // so it goes once the slot catches up (a good save in `park`) or the draft is discarded.
                m.composer.left_on_shelf = Some((copy, held));
            }
        }
        return say(m, fx, format!("The draft could not be opened, and nothing was lost: {e}"));
    }
    remove_shelf(&shelf);
    m.composer.mind = Ok(Some(waiting));
    show(m, fx);
}

/// The person's **Discard prepared draft**: the slot is emptied and its file removed. The person's
/// own draft is not touched.
pub fn discard_prepared(m: &mut Model, fx: &mut Effects<Email>) {
    if !matches!(m.composer.mind, Ok(Some(_))) {
        return;
    }
    let shown = m.composer.in_slot && m.composer.open;
    discard(m);
    if shown {
        m.composer.open = false;
        m.composer.text = Draft::default();
        fx.changed(part::COMPOSE_TEXT);
    }
    m.composer.status.clear();
    fx.changed(part::STATUS);
}

/// A draft the person touched, waiting on a shelf, whose slot is free comes into the slot, where the
/// entry "Draft Yantrik prepared" opens it. Without this a touched shelf whose card is gone would be
/// on disk with no way in.
pub fn promote_waiting(m: &mut Model) {
    if !matches!(m.composer.mind, Ok(None)) {
        return;
    }
    let slot_file = m.files.mind_draft.clone();
    let Some(dir) = slot_file.parent() else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut shelves: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            let id = name.strip_prefix("mind_draft.").and_then(|n| n.strip_suffix(".json")).unwrap_or_default();
            shelf_path(&slot_file, id).is_some_and(|s| s == *p)
        })
        .collect();
    shelves.sort();
    for shelf in shelves {
        let Ok(Some(d)) = load_mind_draft(&shelf) else { continue };
        if !d.touched || save_mind_draft(&slot_file, &d).is_err() {
            continue;
        }
        remove_shelf(&shelf);
        m.composer.mind = Ok(Some(d));
        return;
    }
}

/// A touched draft whose promise is gone becomes the person's own draft when they have none (the
/// one place Compose reopens), and waits for the slot otherwise. Never writes over their draft.
/// One that carries a file never moves: `draft.json` cannot hold the file, and the person's draft
/// would then be sent without it. It stays on its shelf for the "Draft Yantrik prepared" entry.
fn keep_for_person(m: &mut Model, shelf: &std::path::Path, d: MindDraft, fx: &mut Effects<Email>) {
    if !d.attachments.is_empty() {
        // Its promise is closed: the shelf keeps the draft and its file but no id, so once promoted
        // to the slot it is kept there instead of being shelved where no card opens it.
        if d.commitment.is_some() {
            if let Err(e) = save_mind_draft(shelf, &MindDraft { commitment: None, ..d }) {
                tracing::warn!(error = %e, "a kept prepared draft could not be rewritten without its closed promise");
            }
        }
        return;
    }
    let c = &m.composer;
    let free = matches!(c.kept, Ok(None)) && !c.open && !c.writing && !c.saving && c.queued.is_none() && c.unsent_at_start.is_none() && c.sending.is_none();
    if free && crate::state::save_draft(&m.files.draft, &d.text).is_ok() {
        m.composer.kept = Ok(Some(d.text));
        m.composer.kept_unsure = false;
        remove_shelf(shelf);
        say(m, fx, "A draft you changed, prepared for a promise that is closed, is kept as your own draft; Compose reopens it.");
    }
}

/// The person's **Remove attachment**: the file goes from the draft, and what is in the composer is
/// then the person's own draft (`draft.json`), sendable like any other; what they write from here on
/// is kept there. Refused while they have a draft of their own, which this would overwrite.
pub fn remove_attachment(m: &mut Model, fx: &mut Effects<Email>) {
    if !(m.composer.in_slot && m.composer.open) {
        return say(m, fx, "Open the prepared draft first.");
    }
    let c = &m.composer;
    if !matches!(c.kept, Ok(None)) || c.saving || matches!(c.queued, Some(FileOp::Save { .. })) {
        return say(m, fx, "You have an unsent message of your own. Send or discard it first, then remove the file from this one.");
    }
    let text = m.composer.text.clone();
    discard(m);
    m.composer.status = "File removed. This is now your own draft.".into();
    fx.changed(part::STATUS);
    if !text.is_empty() {
        super::drafts::file_op(m, FileOp::Save { draft: text, keep: Keep::Quietly, unsure: false, release_lane: false }, fx);
    }
}

/// What `forget_prepared` says while the person has the prepared draft open. `commitment_hands::classify`
/// keeps a clear owed on "try again".
const SLOT_OPEN: &str = "The person has the prepared draft open, so it was left alone; try again when it is closed.";

/// `forget_prepared`: the promise was forgotten, or its item expired. The draft prepared for it goes
/// unless the person has made it their own by changing it; a draft prepared for another promise stays.
///
/// It never touches a slot the person has open in the composer, whoever asks: it refuses, and marks the
/// slot theirs if the window's text is no longer what it was, so a later clear leaves it alone too.
/// The window is not closed under them, edited or not.
pub fn forget_prepared(m: &mut Model, commitment: &str, fx: &mut Effects<Email>) -> Result<(), Refused> {
    let commitment = commitment.trim();
    let shelf_of = shelf_path(&m.files.mind_draft, commitment);
    // The copy a failed open left, while the slot holds the later version of the same draft: the
    // slot's own handling below decides for both.
    let left_for_slot = matches!((&m.composer.left_on_shelf, &m.composer.mind), (Some((s, _)), Ok(Some(slot)))
        if slot.commitment.as_deref() == Some(commitment) && shelf_of.as_deref() == Some(s.as_path()));
    // Its shelf: an untouched draft goes; one the person touched is theirs, and is kept for them.
    if let Some(shelf) = shelf_of.filter(|_| !left_for_slot) {
        match load_mind_draft(&shelf) {
            Ok(Some(d)) if !d.touched => remove_shelf(&shelf),
            Ok(Some(d)) => keep_for_person(m, &shelf, d, fx),
            _ => {}
        }
    }
    // A touched draft in the slot is theirs and stays, but its promise is closed: it keeps no id, so
    // opening another card keeps it in the slot instead of shelving it where no card opens it.
    if let Ok(Some(slot)) = &mut m.composer.mind {
        if slot.touched && slot.commitment.as_deref() == Some(commitment) {
            slot.commitment = None;
            match save_mind_draft(&m.files.mind_draft, slot) {
                Ok(()) if left_for_slot => drop_left_on_shelf(m, Some(commitment)),
                Ok(()) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "the prepared draft could not be kept");
                    // The slot file still holds the promise, and so does the copy the failed open left:
                    // keep the id until a save succeeds, so that save (`park`) or a discard still
                    // removes that older copy instead of leaving it to come back.
                    if left_for_slot {
                        slot.commitment = Some(commitment.to_string());
                    }
                }
            }
        }
    }
    let ours = matches!(&m.composer.mind, Ok(Some(slot)) if !slot.touched && slot.commitment.as_deref() == Some(commitment) && !commitment.is_empty());
    if !ours {
        promote_waiting(m);
        return Ok(());
    }
    // Open in front of the person: an Email started for an agent has no window and nobody typing, and
    // a draft there is only the agent's, so it goes as before.
    if m.composer.in_slot && m.composer.open && m.person_here {
        if edited_since_shown(m) {
            park(m);
        }
        return Err(Refused::new("prepared_open", SLOT_OPEN));
    }
    discard_prepared(m, fx);
    Ok(())
}
