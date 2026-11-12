//! A Mind's draft: its own slot, apart from the person's (#1184).
//!
//! `compose` from a Mind never changes the person's kept or open draft, and never inherits its Cc
//! or Bcc. When the person has unsent text it refuses, and says why. The draft starts empty and is
//! kept, with the files it carries, in `mind_draft.json`, so a restart does not lose the file and
//! with it the refusal to send without it. Everything the composer does to a draft goes here while
//! it shows this one (`Composer::in_slot`).

use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use crate::model::{part, Model};
use crate::state::{checked_attachment, save_mind_draft, clear_mind_draft, Draft, MindDraft};
use crate::Email;

/// What the composer says while a draft carries a file the mail service cannot send.
pub const ATTACHED_STATUS: &str = "A file is attached to this draft. Email cannot send attachments yet, so it will not be sent from here.";

/// What the composer says on a draft Yantrik prepared that carries no file.
const PREPARED_STATUS: &str = "Draft prepared for you";

const PERSON_WRITING: &str = "You have an unsent message of your own, and a draft from Yantrik would not replace it. Send or discard yours first.";

/// Why a Mind's `compose` cannot start now, if the person has text of their own in the way.
fn person_has_text(m: &Model) -> Option<&'static str> {
    let c = &m.composer;
    let kept = !matches!(c.kept, Ok(None));
    let open_theirs = c.open && !c.in_slot && !c.text.is_empty();
    let edited = matches!(&c.mind, Ok(Some(slot)) if slot.touched) || (c.open && c.in_slot && edited_since_shown(m));
    (kept || open_theirs || edited || c.unsent_at_start.is_some() || c.sending.is_some()).then_some(PERSON_WRITING)
}

/// The window's text is no longer what the slot holds: the person has been typing in it.
fn edited_since_shown(m: &Model) -> bool {
    matches!(&m.composer.mind, Ok(Some(slot)) if slot.text != m.composer.text)
}

fn home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(Into::into)
}

/// `compose` from a Mind: a new draft in the Mind's slot, shown in the composer.
pub fn compose(m: &mut Model, to: String, subject: String, body: String, attach: String, fx: &mut Effects<Email>) -> Result<(), Refused> {
    if let Err(e) = &m.composer.mind {
        return Err(Refused::new("draft_unreadable", format!("The draft Yantrik prepared earlier could not be read, so it was not replaced: {e}")));
    }
    if let Some(why) = person_has_text(m) {
        return Err(Refused::new("person_is_writing", why));
    }
    let attachments = if attach.trim().is_empty() {
        Vec::new()
    } else {
        vec![checked_attachment(&attach, home().as_deref()).map_err(|e| Refused::new("cannot_attach", e))?]
    };
    let slot = MindDraft { text: Draft { to, subject, body, ..Draft::default() }, attachments, touched: false };
    save_mind_draft(&m.files.mind_draft, &slot).map_err(|e| Refused::new("not_kept", format!("The draft could not be kept: {e}")))?;
    m.composer.mind = Ok(Some(slot));
    show(m, fx);
    Ok(())
}

/// Open the composer on the slot.
pub fn show(m: &mut Model, fx: &mut Effects<Email>) {
    let Ok(Some(slot)) = m.composer.mind.clone() else { return };
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
    }
    if let Err(e) = save_mind_draft(&m.files.mind_draft, slot) {
        tracing::warn!(error = %e, "the draft Yantrik prepared could not be kept");
    }
}

/// The draft went: its slot is emptied.
pub fn discard(m: &mut Model) {
    m.composer.mind = Ok(None);
    m.composer.in_slot = false;
    m.composer.attachments.clear();
    if let Err(e) = clear_mind_draft(&m.files.mind_draft) {
        tracing::warn!(error = %e, "the prepared draft's file could not be removed");
    }
}
