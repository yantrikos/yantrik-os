//! The draft file: written and removed in jobs, one at a time and in the order they were asked
//! for. A save asked while another is writing waits, and only the latest waits.
//!
//! Beside the draft, a mark says when whether its message was already sent is not known (a send
//! that never came back), so every Compose that opens it says so until it is sent or discarded.
//! The lane copy a send left is dropped only once the draft file holding its text has landed, so
//! the text is never nowhere.

use yantrik_app::effects::Effects;

use super::{clear_notice, say};
use crate::command::{Command, Ending, Work};
use crate::files;
use crate::model::{part, FileOp, Keep, Model, COMPOSING, SENDING};
use crate::state;
use crate::Email;

/// What the notice says when a message is offered again at start.
pub const UNSENT_AT_START: &str = "A message was being sent when Email closed, and whether it went is not known. It is kept as a draft: look in Sent before sending it again.";

/// What the notice says when the text the composer held at a logout is offered again.
pub const WRITING_AT_START: &str = "The message you were writing when Email closed is kept as a draft; Compose reopens it.";

/// What Compose says, every time it opens a draft whose message may already have gone.
pub const UNSURE: &str = "Whether this message was already sent is not known. Look in Sent before sending it again.";

/// The composer's status line for that draft.
pub const UNSURE_STATUS: &str = "Not known whether this was sent";

/// One write or removal of the draft file, in a job; or, while one is running, the next one. A
/// queued operation replaced by a newer one hands its duty to drop the lane copy on.
pub fn file_op(m: &mut Model, op: FileOp, fx: &mut Effects<Email>) {
    if m.composer.writing {
        let release = op.releases_lane() || m.composer.queued.as_ref().is_some_and(FileOp::releases_lane);
        m.composer.queued = Some(op.releasing_lane(release));
        return;
    }
    m.composer.writing = true;
    let path = m.files.draft.clone();
    fx.spawn_or(Command::Unfinished(Work::Draft), move |_| {
        let outcome = match &op {
            FileOp::Save { draft, unsure, .. } => {
                state::save_draft(&path, draft).and_then(|()| files::set_unsure(&path, *unsure))
            }
            FileOp::Clear { .. } => state::clear_draft(&path).and_then(|()| files::set_unsure(&path, false)),
        };
        Command::DraftDone { op, outcome }
    });
}

/// The next file operation, once the one before it has landed.
pub fn next_file_op(m: &mut Model, fx: &mut Effects<Email>) {
    m.composer.writing = false;
    if let Some(op) = m.composer.queued.take() {
        file_op(m, op, fx);
    }
}

/// The lane copy goes once the draft file holds its text, unless a newer send has put its own
/// copy there meanwhile.
fn release_lane(m: &Model, release: bool, fx: &mut Effects<Email>) {
    if release && m.composer.sending.is_none() {
        fx.discard_draft(SENDING);
        fx.discard_draft(COMPOSING);
    }
}

pub fn done(m: &mut Model, op: FileOp, outcome: Result<(), String>, fx: &mut Effects<Email>) {
    fx.changed(part::STATUS);
    let release = op.releases_lane();
    match (op, outcome) {
        (FileOp::Save { draft, keep, unsure, .. }, Ok(())) => {
            m.composer.kept = Ok(Some(draft));
            m.composer.kept_unsure = unsure;
            release_lane(m, release, fx);
            match keep {
                Keep::Quietly => {}
                Keep::Saved => {
                    m.composer.status = "Draft saved".into();
                    clear_notice(m, fx);
                }
                Keep::Closing => {
                    m.composer.status = "Draft saved".into();
                    if let Some(sentence) = closing_sentence(m) {
                        say(m, fx, sentence);
                    }
                }
            }
        }
        (FileOp::Save { keep: Keep::Saved, .. }, Err(e)) => {
            m.composer.status.clear();
            say(m, fx, format!("Could not save the draft: {e}"));
        }
        // The lane copy stays: it is the only copy that lasts.
        (FileOp::Save { .. }, Err(e)) => say(m, fx, format!("Could not keep the unsent message: {e}")),
        (FileOp::Clear { after_send, .. }, Ok(())) => {
            m.composer.kept = Ok(None);
            m.composer.kept_unsure = false;
            release_lane(m, release, fx);
            if after_send {
                clear_notice(m, fx);
            }
        }
        (FileOp::Clear { after_send: true, .. }, Err(e)) => {
            say(m, fx, format!("The message was sent; the draft is still on disk: {e}"))
        }
        // Closing an empty composer: a file that could not be removed is reopened by Compose.
        (FileOp::Clear { .. }, Err(_)) => {}
    }
    next_file_op(m, fx);
}

/// What Cancel's kept draft is said as, which depends on a send pressed before it: still going,
/// landed since (its own sentence already says where the edit went, and stays), or none.
fn closing_sentence(m: &Model) -> Option<&'static str> {
    if m.composer.sending.is_some() {
        Some("What was in the composer is kept as a draft; Compose reopens it. The message is still being sent as it was when Send was pressed.")
    } else if m.composer.closed_during_send {
        None
    } else {
        Some("The unsent message was kept as a draft; Compose reopens it.")
    }
}

/// A message that was being sent when the app last closed becomes the draft file, marked as of
/// unknown outcome. Applied in `start`, before any press, so a draft the person keeps or discards
/// afterwards is written after it. Its lane copy goes when the file has landed.
pub fn write_unsent(m: &mut Model, fx: &mut Effects<Email>) {
    let Some(unsent) = m.composer.unsent_at_start.take() else { return };
    let unsure = m.composer.kept_unsure;
    file_op(m, FileOp::Save { draft: unsent, keep: Keep::Quietly, unsure, release_lane: true }, fx);
}

/// The app is ending: the session, a person's quit, or the window closing. Nothing is asked: the
/// newest text there is goes to the draft lane in this command (the lane is written before the
/// process ends; a job would not be), and the app ends. That is the open composer's text, or else
/// a draft save still queued behind the write in flight, which the end would otherwise drop. A
/// send in flight left its own copy there when Send was pressed. Nothing is sent.
pub fn ending(m: &mut Model, how: Ending, fx: &mut Effects<Email>) {
    let open = (m.composer.open && !m.composer.text.is_empty()).then(|| m.composer.text.clone());
    let queued = match &m.composer.queued {
        Some(FileOp::Save { draft, .. }) => Some(draft.clone()),
        _ => None,
    };
    if let Some(text) = open.or(queued) {
        fx.checkpoint_draft(COMPOSING, serde_json::to_vec(&text).unwrap_or_default());
    }
    match how {
        Ending::Window(window) => fx.close(window),
        Ending::Logout | Ending::Quit => fx.exit(),
    }
}

/// The sentence about it, once the first look has landed or failed, so the look's own notice
/// cannot clear it; unless the person has already kept or discarded the draft.
pub fn offer_unsent(m: &mut Model, fx: &mut Effects<Email>) {
    let Some(said) = m.composer.announce_unsent.take() else { return };
    let sentence = if m.notice.is_empty() { said.to_string() } else { format!("{} {said}", m.notice) };
    say(m, fx, sentence);
}
