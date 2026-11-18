//! Writing: the composer, replies and forwards, sending, and the draft that survives Cancel. The
//! draft file itself is `drafts.rs`.

use yantrik_app::effects::Effects;
use yantrik_app::jobs::{Concurrency, JobKey, Scope};

use super::drafts::{file_op, UNSURE, UNSURE_STATUS};
use super::say;
use crate::backend::calls::{self, Outgoing};
use crate::command::{Command, Listing, Work};
use crate::model::{part, FileOp, Keep, Model, SENDING};
use crate::state::Draft;
use crate::{rows, Email};

/// The composer opened on new text: nothing from an earlier send is pending on it.
fn opened_on_new_text(m: &mut Model) {
    m.composer.open = true;
    m.composer.unsure = false;
    m.composer.closed_during_send = false;
}

/// Open the composer with whatever was being written when it was last closed. The composer used
/// to open empty every time and the half-written message was simply gone. A draft whose message
/// may already have gone says so, every time it is opened, until it is sent or discarded.
pub fn compose_new(m: &mut Model, fx: &mut Effects<Email>) {
    opened_on_new_text(m);
    fx.changed(part::STATUS);
    // Where a Mind looks the model is as anywhere, since `compose` is an intent and builds on the
    // kept draft; the window masks the fields (`view::render`) and never writes the draft back.
    match m.composer.kept.clone() {
        Ok(Some(draft)) => {
            m.composer.text = draft;
            fx.changed(part::COMPOSE_TEXT);
            if m.composer.kept_unsure {
                m.composer.unsure = true;
                m.composer.status = UNSURE_STATUS.into();
                say(m, fx, UNSURE);
            } else {
                m.composer.status = "Draft restored".into();
            }
        }
        Ok(None) => {
            m.composer.text = Draft::default();
            m.composer.status.clear();
            fx.changed(part::COMPOSE_TEXT);
        }
        Err(e) => {
            m.composer.status.clear();
            say(m, fx, format!("Could not read the saved draft: {e}"));
        }
    }
}

/// `compose`: the composer opened as Compose opens it, with these filled in. Sending is not
/// offered on the surface on purpose: a draft can be read before it leaves, and mail that has
/// already gone cannot be taken back.
pub fn compose(m: &mut Model, to: String, subject: String, body: String, fx: &mut Effects<Email>) {
    compose_new(m, fx);
    m.composer.text.to = to;
    m.composer.text.subject = subject;
    m.composer.text.body = body;
    fx.changed(part::COMPOSE_TEXT);
    // New text: not the message whose send may have gone, so it is not said or kept as one.
    if m.composer.unsure {
        m.composer.unsure = false;
        m.composer.status = "Draft restored".into();
        if m.notice == UNSURE {
            super::clear_notice(m, fx);
        }
    }
}

/// Reply, or reply to all: the composer opened on the message that is open. None of this needs a
/// network call; the message is already on screen.
pub fn reply(m: &mut Model, all: bool, fx: &mut Effects<Email>) {
    let Some(open) = m.open.as_ref() else { return };
    // A reply quotes the message and names its sender and subject; where a Mind looks it starts
    // empty (`mind_view`).
    m.composer.text = if m.for_mind { Draft::default() } else { rows::reply(&open.detail, all) };
    opened_on_new_text(m);
    fx.changed(part::COMPOSE_TEXT);
    fx.changed(part::STATUS);
}

pub fn forward(m: &mut Model, fx: &mut Effects<Email>) {
    let detail = m.open.as_ref().map(|o| o.detail.clone()).unwrap_or_default();
    m.composer.text = if m.for_mind { Draft::default() } else { rows::forward(&detail) };
    opened_on_new_text(m);
    fx.changed(part::COMPOSE_TEXT);
    fx.changed(part::STATUS);
}

/// Send what is in the composer. One at a time, and said: Send shows "Sending…" and takes no
/// press while one is out, and a press that arrives anyway is answered on the notice line.
///
/// The text is kept through the framework's draft lane before anything leaves, so a send that
/// fails, panics, or is still waiting for a worker when the window closes leaves it where Compose
/// finds it.
pub fn send(m: &mut Model, mail: Draft, fx: &mut Effects<Email>) {
    let account = m.account();
    if account.is_empty() {
        return say(m, fx, "Cannot send: no email account is configured.");
    }
    if m.composer.sending.is_some() {
        return say(m, fx, "A message is already being sent; wait for it before sending again.");
    }
    m.composer.sending = Some(mail.clone());
    m.composer.text = mail.clone();
    m.sync_status = "Sending\u{2026}".into();
    fx.changed(part::STATUS);
    fx.checkpoint_draft(SENDING, serde_json::to_vec(&mail).unwrap_or_default());
    let service = m.mail.clone();
    fx.spawn_keyed_or(
        JobKey { feature: "send", scope: Scope::App },
        Concurrency::RejectIfRunning,
        Command::Unfinished(Work::Send),
        move |_| {
            let outgoing = Outgoing { to: &mail.to, cc: &mail.cc, bcc: &mail.bcc, subject: &mail.subject, body: &mail.body };
            Command::Sent(calls::send_message(service.as_ref(), &account, &outgoing))
        },
    );
}

pub fn sent(m: &mut Model, outcome: Result<(), String>, fx: &mut Effects<Email>) {
    fx.changed(part::STATUS);
    match outcome {
        Ok(()) => {
            let went = m.composer.sending.take();
            // It went, so its lane copy is not needed: the text is in Sent.
            fx.discard_draft(SENDING);
            m.composer.unsure = false;
            m.sync_status = "Message sent".into();
            m.composer.status.clear();
            let edited = !m.composer.text.is_empty() && went.as_ref() != Some(&m.composer.text);
            if edited {
                // What was typed after Send is kept, where it was typed or where Cancel put it.
                let text = m.composer.text.clone();
                file_op(m, FileOp::Save { draft: text, keep: Keep::Quietly, unsure: false, release_lane: false }, fx);
                let sentence = if m.composer.open {
                    "The message was sent as it was when Send was pressed. What was changed since is still in the composer, kept as a draft."
                } else {
                    "The message was sent as it was when Send was pressed. What was changed since is kept as a draft; Compose reopens it."
                };
                say(m, fx, sentence);
            } else {
                m.composer.open = false;
                // The draft was this message; it has gone.
                file_op(m, FileOp::Clear { after_send: true, release_lane: false }, fx);
            }
            let folder = m.folder.clone();
            // The folder again, so what was sent shows; a failure there is said by the listing.
            if let Err(e) = super::mailbox::load_folder(m, &folder, Listing::AfterSend, fx) {
                tracing::warn!(error = %e, "the folder was not read again after a send");
            }
        }
        Err(e) => {
            // The composer stays open, holding what was typed, and so does the draft. It did not
            // go this time; an earlier send of it whose outcome was not known still is not.
            let unsure = m.composer.unsure;
            let went = m.composer.sending.clone();
            keep_after_send(m, unsure, went, fx);
            m.sync_status = "Not sent".into();
            say(m, fx, format!("Could not send the message: {e}"));
        }
    }
}

/// A send that did not go, or whose outcome is not known (`unsure`): the composer's text becomes
/// the draft, or what was sent (`went`) when the composer was emptied meanwhile, and the lane copy
/// goes once it has landed. A failure is known, so it is never left to be offered as unknown.
pub fn keep_after_send(m: &mut Model, unsure: bool, went: Option<Draft>, fx: &mut Effects<Email>) {
    m.composer.sending = None;
    m.composer.unsure = unsure;
    let text = if m.composer.text.is_empty() { went.unwrap_or_default() } else { m.composer.text.clone() };
    if !text.is_empty() {
        file_op(m, FileOp::Save { draft: text, keep: Keep::Quietly, unsure, release_lane: true }, fx);
    }
}

/// Close the composer and keep what was typed. Closing it used to discard it silently, which is
/// the worst thing a mail client can do to a message that is not finished.
pub fn cancel(m: &mut Model, draft: Draft, fx: &mut Effects<Email>) {
    m.composer.announce_unsent = None;
    m.composer.open = false;
    m.composer.closed_during_send = m.composer.sending.is_some();
    fx.changed(part::STATUS);
    // The draft file is the person's: closing a Mind's composer neither saves nor clears it.
    if m.for_mind {
        m.composer.status.clear();
        return;
    }
    if draft.is_empty() {
        m.composer.status.clear();
        file_op(m, FileOp::Clear { after_send: false, release_lane: false }, fx);
        return;
    }
    m.composer.text = draft.clone();
    let unsure = m.composer.unsure;
    file_op(m, FileOp::Save { draft, keep: Keep::Closing, unsure, release_lane: false }, fx);
}

pub fn save_draft(m: &mut Model, draft: Draft, fx: &mut Effects<Email>) {
    if draft.is_empty() {
        m.composer.status.clear();
        return say(m, fx, "There is nothing in the composer to save.");
    }
    m.composer.announce_unsent = None;
    m.composer.text = draft.clone();
    let unsure = m.composer.unsure;
    file_op(m, FileOp::Save { draft, keep: Keep::Saved, unsure, release_lane: false }, fx);
}
