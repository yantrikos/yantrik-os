//! Closing the window, quitting, the unsaved-changes question, and the session ending.
//!
//! A sheet with unsaved changes is not left without asking: "<sheet> has changes that are not
//! saved" with *Cancel*, *Discard* and *Save*, standing in front of whatever was asked for: the
//! window closing, or another file. *Save* carries the request out only once the save has landed.
//! The question is shown while `Model::pending` holds it and never otherwise, and there is one at
//! a time. The sheet's draft is kept all the while, so a crash behind the question loses nothing.
//!
//! The window closed while a save is in flight closes at once and ySheets runs on with no window
//! until the save lands (`fx.linger`), then ends; a second launch meanwhile brings the window back
//! and ySheets runs on. A save that fails then is not lost: the sheet is in its draft, offered at
//! the next start, and the person is told.
//!
//! The window closed while a file is read waits for that file, then goes on with the close as
//! asked. The session ending (a logout, SIGTERM, the display gone) asks nothing: the unsaved sheet
//! goes to its draft in the command that answers, and ySheets ends.

use std::time::Duration;

use yantrik_app::app::WindowId;
use yantrik_app::effects::Effects;
use yantrik_app::Notification;

use super::{drafts, picking, say};
use crate::model::{Dialog, Model, Pending, Work};
use crate::YSheets;

/// How long ySheets runs on without a window for a save in flight to land.
pub const LINGER: Duration = Duration::from_secs(10);

/// Said when the window is closed while a file is read: the close is taken, and goes on once that
/// file is done.
pub const CLOSING_WHEN_DONE: &str = "The window closes once the file being read is done.";

/// Said when the file the close waited on failed: nothing closes.
pub const NOT_CLOSED: &str = "The window was not closed, so nothing is lost.";

/// The person closed the window, or asked ySheets to quit. What is in the formula bar is committed
/// first, as clicking away would: the window tells the model what it holds (`Cue::CommitField`),
/// and the close goes on when that arrives (`window::header`). In a window drawn for a Mind nothing
/// is ever committed from the window, and under a dialog the bar is not in use.
pub fn requested(m: &mut Model, fx: &mut Effects<YSheets>) {
    if m.for_mind || m.dialog_up() || m.closing_after_commit {
        return window(m, fx);
    }
    m.closing_after_commit = true;
    m.cues.push(crate::model::Cue::CommitField);
}

/// The close, once the formula bar is committed.
pub fn window(m: &mut Model, fx: &mut Effects<YSheets>) {
    // The same close again needs no words: its question is already up.
    if m.asking() {
        return;
    }
    match m.busy {
        // The window goes now and the save lands with none; the sheet is in its draft meanwhile.
        Some(Work::Save) => {
            drafts::checkpoint_now(m, fx, None);
            fx.close(WindowId::PRIMARY);
            fx.linger(LINGER);
            m.lingering = true;
        }
        Some(Work::Open) => {
            m.close_when_done = true;
            say(m, CLOSING_WHEN_DONE);
        }
        None => {
            // A path dialog is put away: the window's close is the answer.
            m.dialog = Dialog::None;
            if m.modified() {
                ask(m, Pending::Close);
            } else {
                fx.close(WindowId::PRIMARY);
            }
        }
    }
}

/// Asks about the sheet's unsaved changes, in front of `pending`.
pub fn ask(m: &mut Model, pending: Pending) {
    m.dialog = Dialog::None;
    m.confirm_replace = None;
    m.pending = Some(pending);
}

/// The question's *Discard* for a close: the sheet is given up on purpose, its draft with it, so
/// the next start does not offer what nobody is missing.
pub fn close_discarding(m: &mut Model, fx: &mut Effects<YSheets>) {
    let name = m.doc.draft.clone();
    drafts::forget(m, fx, &name);
    // Nothing is kept from here: the draft is not made again, whatever the sheet still holds.
    m.given_up = true;
    fx.close(WindowId::PRIMARY);
}

/// The question's *Save* landed: what waited on it goes on.
pub fn continue_pending(m: &mut Model, fx: &mut Effects<YSheets>) {
    match m.pending.take() {
        Some(Pending::Open) => picking::pick(m, fx, false),
        Some(Pending::Close) => fx.close(WindowId::PRIMARY),
        None => {}
    }
}

/// A save landed: a linger waiting on it ends.
pub fn saved(m: &mut Model, fx: &mut Effects<YSheets>) {
    if m.lingering {
        end_after_linger(m, fx, None);
    }
}

/// A save did not land: the question that waited on it is over (the window stays, with the
/// reason), and a linger ends with the sheet kept as a draft.
pub fn not_saved(m: &mut Model, fx: &mut Effects<YSheets>, why: &str) {
    m.pending = None;
    not_closed(m);
    if m.lingering {
        end_after_linger(m, fx, Some(why));
    }
}

/// A close that waited on a file that failed is dropped, and the failure already said gains that
/// the window stayed.
pub fn not_closed(m: &mut Model) {
    if std::mem::take(&mut m.close_when_done) {
        let (window, agents) = (m.notice.window().to_string(), m.notice.read(true).to_string());
        m.notice.set_about(window != agents, format!("{window} {NOT_CLOSED}"), || format!("{agents} {NOT_CLOSED}"));
    }
}

/// The save the window closed on has landed: ySheets ends. Whatever is unsaved now (that save
/// failed) goes to its draft first and the person is told, since there is no window left to tell
/// them in.
fn end_after_linger(m: &mut Model, fx: &mut Effects<YSheets>, why: Option<&str>) {
    if drafts::checkpoint_now(m, fx, None) {
        let body = match why {
            Some(why) => format!("{why} The sheet is kept and comes back the next time ySheets opens."),
            None => "The unsaved sheet is kept and comes back the next time ySheets opens.".to_string(),
        };
        fx.notify(Notification::titled("ySheets could not save everything").body(body));
    }
    m.lingering = false;
    fx.exit();
}

/// After every command: a close that waited for a file to be read goes on once it is.
pub fn resume(m: &mut Model, fx: &mut Effects<YSheets>) {
    if m.close_when_done && m.busy.is_none() {
        m.close_when_done = false;
        m.notice.clear();
        window(m, fx);
    }
}

/// A second launch brought the window back during a linger: ySheets runs on, and the save in
/// flight lands as any other.
pub fn presented(m: &mut Model) {
    m.lingering = false;
}

/// The session or the system is ending ySheets, with no one to ask: the unsaved sheet is handed to
/// its draft now, rather than after its debounce, and ySheets ends in this one command.
pub fn logging_out(m: &mut Model, fx: &mut Effects<YSheets>) {
    // The formula bar's text, if it was not entered, goes into the draft: it is unsaved text, and a
    // logout asks nothing. It is read from the window here, in the one command that has no time to
    // be told it first.
    m.ending = true;
    let bar = m.bar();
    drafts::checkpoint_now(m, fx, bar.as_deref());
    fx.exit();
}
