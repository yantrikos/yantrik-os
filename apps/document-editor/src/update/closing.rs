//! Closing the window, quitting, the unsaved-changes prompt, and the session ending.
//!
//! A document with unsaved changes is not left without asking: "<document> has unsaved changes."
//! with *Cancel*, *Discard* and *Save* (prompt.rs), standing in front of whatever was asked for:
//! the window closing, a new document, or another file. *Save* carries the intent out only once
//! the save has landed; a save that fails keeps the prompt up with the reason.
//!
//! The window closed while a save is in flight closes at once and yDoc runs on with no window
//! until the save lands (`fx.linger`), then ends; a second launch meanwhile brings the window back
//! and yDoc runs on. A save that fails then is not lost: the text is in its draft, offered at the
//! next start, and the person is told.
//!
//! The window closed while a file is read or exported waits for that file, then goes on with the
//! close as asked: the usual prompt if anything is unsaved, the window staying until it is
//! answered.
//!
//! The session ending (a logout, SIGTERM, the display gone) asks nothing: the unsaved text goes to
//! its draft in the command that answers, and yDoc ends.

use std::time::Duration;

use yantrik_app::app::WindowId;
use yantrik_app::documents::close::{Next, Stage};
use yantrik_app::documents::{Choice, CloseFlow, OutsideState};
use yantrik_app::effects::Effects;
use yantrik_app::Notification;

use super::{document, drafts, say, saving};
use crate::command::{AfterOpen, AfterSave};
use crate::model::{Cue, Dialog, Model, Work};
use crate::prompt::{Closing, Intent};
use crate::YDoc;

/// How long yDoc runs on without a window for a save in flight to land.
pub const LINGER: Duration = Duration::from_secs(10);

/// Said when the window is closed while a file is read or exported: the close is taken, and goes
/// on once that file is done.
pub const CLOSING_WHEN_DONE: &str = "The window closes once the file being read or written is done.";

/// Said when the save the close waited on failed: nothing closes.
pub const NOT_CLOSED: &str = "The window was not closed, so nothing is lost.";

/// The person closed the window, or asked yDoc to quit.
pub fn window(m: &mut Model, fx: &mut Effects<YDoc>) {
    // The same close again needs no words: its prompt, or its save, is already under way.
    if m.closing.is_some() {
        return;
    }
    match m.busy {
        // Nothing else is unsaved: the window goes now and the save lands with none.
        Some(Work::Save) => {
            m.dialog = Dialog::None;
            fx.close(WindowId::PRIMARY);
            fx.linger(LINGER);
            m.lingering = true;
        }
        Some(_) => {
            m.close_when_done = true;
            say(m, CLOSING_WHEN_DONE);
        }
        None => {
            // A file dialog is put away: the window's close is the answer.
            m.dialog = Dialog::None;
            if m.modified() {
                ask(m, Intent::Close);
            } else {
                fx.close(WindowId::PRIMARY);
            }
        }
    }
}

/// Asks about the document's unsaved changes, in front of `intent`.
pub fn ask(m: &mut Model, intent: Intent) {
    let flow = CloseFlow::new(Some(WindowId::PRIMARY), vec![m.doc.id]);
    m.closing = Some(Closing { intent, flow, via_save_as: false });
    m.dialog_error.clear();
}

/// The person's answer to the prompt.
pub fn answer(m: &mut Model, fx: &mut Effects<YDoc>, choice: Choice) {
    if m.closing.is_none() {
        return;
    }
    // A document with no file, or whose file is gone, needs a place first: the Save As dialog,
    // whose save is the answer. A document whose file was deleted never traps the person in a
    // prompt whose Save can only refuse.
    let homeless = m.path().is_none() || m.d().outside == OutsideState::Deleted;
    if choice == Choice::Save && homeless {
        if let Some(closing) = m.closing.as_mut() {
            closing.via_save_as = true;
        }
        m.dialog = Dialog::SaveAs;
        m.dialog_path = m.home().display().to_string();
        m.dialog_error.clear();
        m.cues.push(Cue::Dialog);
        return;
    }
    let Some(closing) = m.closing.as_mut() else { return };
    let next = closing.flow.answer(choice);
    apply(m, fx, next);
}

/// The Save As dialog's save, when the prompt's *Save* opened it: its landing carries the intent
/// out.
pub fn saving_as(m: &mut Model) {
    if let Some(closing) = m.closing.as_mut().filter(|c| c.via_save_as) {
        let _ = closing.flow.answer(Choice::Save);
    }
}

/// Cancel: the prompt goes, and the document stays as it was.
pub fn cancel(m: &mut Model) {
    m.closing = None;
}

/// What the prompt's answer or a save's landing leads to.
fn apply(m: &mut Model, fx: &mut Effects<YDoc>, next: Next) {
    match next {
        Next::Stay => m.closing = None,
        Next::Wait => {}
        Next::Save(_) => saving::start(m, fx, None, false, AfterSave::Window, false),
        Next::Close { .. } => carry_out(m, fx),
    }
}

/// The draft is dealt with, saved or given up on purpose: whatever the prompt stood in front of.
fn carry_out(m: &mut Model, fx: &mut Effects<YDoc>) {
    let Some(closing) = m.closing.take() else { return };
    match closing.intent {
        Intent::Close => {
            // Given up on purpose, or saved: the draft goes with the document, so the next start
            // does not offer what nobody is missing.
            document::blank(m, fx);
            fx.close(WindowId::PRIMARY);
        }
        Intent::New => {
            document::blank(m, fx);
            m.notice.clear();
        }
        Intent::Open(path) => {
            // Whichever way the prompt was answered, the draft has been dealt with, so this open
            // is no longer discarding anything unasked. The document is replaced only when the
            // file has been read: one that cannot be leaves it as it was.
            super::opening::start(m, fx, path, AfterOpen::Prompted, crate::caller::Caller::person());
        }
    }
}

/// The document was saved: a prompt waiting on it goes ahead, and a linger waiting on it ends.
pub fn saved(m: &mut Model, fx: &mut Effects<YDoc>) {
    let still_unsaved = m.modified();
    if let Some(closing) = m.closing.as_mut() {
        if closing.flow.stage == Stage::Asking && !still_unsaved {
            // Saved by a save that was not the prompt's answer (`save_as`): the question is no
            // longer true, so the prompt goes and the document stays.
            m.closing = None;
        } else {
            let next = closing.flow.progress(&m.documents, None);
            apply(m, fx, next);
        }
    }
    if m.lingering {
        end_after_linger(m, fx, None);
    }
}

/// The document's save failed: a prompt waiting on it stays, with the reason; a linger ends, with
/// the text kept as a draft.
pub fn not_saved(m: &mut Model, fx: &mut Effects<YDoc>, why: &str) {
    if let Some(closing) = m.closing.as_mut() {
        // Blocked: the prompt stays up, asking again, with the reason under it.
        let _ = closing.flow.progress(&m.documents, Some(why.to_string()));
    }
    not_closed(m);
    if m.lingering {
        end_after_linger(m, fx, Some(why));
    }
}

/// A close that waited on a save or an export that failed is dropped, and the failure already
/// said gains that the window stayed.
pub fn not_closed(m: &mut Model) {
    if std::mem::take(&mut m.close_when_done) {
        // Added to the failure already said, in both its forms (notice.rs).
        let (window, agents) = (m.notice.window().to_string(), m.notice.read(true).to_string());
        m.notice.set_about(window != agents, format!("{window} {NOT_CLOSED}"), || format!("{agents} {NOT_CLOSED}"));
    }
}

/// The save the window closed on has landed: yDoc ends. Whatever is unsaved now (that save
/// failed) goes to its draft first and the person is told, since there is no window left to tell
/// them in.
fn end_after_linger(m: &mut Model, fx: &mut Effects<YDoc>, why: Option<&str>) {
    let kept = drafts::checkpoint_now(m, fx);
    if kept {
        let body = match why {
            Some(why) => format!("{why} The text is kept and comes back the next time yDoc opens."),
            None => "Unsaved text is kept and comes back the next time yDoc opens.".to_string(),
        };
        fx.notify(Notification::titled("yDoc could not save everything").body(body));
    }
    m.lingering = false;
    fx.exit();
}

/// After every command: a close that waited for a file to be read or exported goes on once it is.
pub fn resume(m: &mut Model, fx: &mut Effects<YDoc>) {
    if m.close_when_done && m.busy.is_none() {
        m.close_when_done = false;
        m.notice.clear();
        window(m, fx);
    }
}

/// A second launch brought the window back during a linger: yDoc runs on, and the save in flight
/// lands as any other.
pub fn presented(m: &mut Model) {
    m.lingering = false;
}

/// The session or the system is ending yDoc, with no one to ask: the unsaved text is handed to
/// its draft now, rather than after its debounce, and yDoc ends in this one command.
pub fn logging_out(m: &mut Model, fx: &mut Effects<YDoc>) {
    drafts::checkpoint_now(m, fx);
    fx.exit();
}
