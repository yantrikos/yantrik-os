//! Closing the window, quitting, and the session ending.
//!
//! This screen has no dialog layer to ask in, so closing with unsaved changes asks nothing: the
//! deck's content goes to its draft in the command that answers, and the next start offers it back
//! by name. Nothing is lost, and the person is never asked a question the window cannot show.
//!
//! The window closed while a save is in flight closes at once and yPresent runs on with no window
//! until the save lands (`fx.linger`), then ends; a second launch meanwhile brings the window back
//! and yPresent runs on. A save that fails then is not lost: the deck is in its draft, offered at
//! the next start, and the person is told.
//!
//! The window closed while a file is read or exported waits for that file, then goes on with the
//! close as asked.
//!
//! The session ending (a logout, SIGTERM, the display gone) asks nothing either: the unsaved deck
//! goes to its draft in the command that answers, and yPresent ends.

use std::time::Duration;

use yantrik_app::app::WindowId;
use yantrik_app::effects::Effects;
use yantrik_app::Notification;

use super::{drafts, say};
use crate::model::{Model, Work};
use crate::YPresent;

/// How long yPresent runs on without a window for a save in flight to land.
pub const LINGER: Duration = Duration::from_secs(10);

/// Said when the window is closed while a file is read or exported: the close is taken, and goes
/// on once that file is done.
pub const CLOSING_WHEN_DONE: &str = "The window closes once the file being read or written is done.";

/// Said when the file the close waited on failed: nothing closes.
pub const NOT_CLOSED: &str = "The window was not closed, so nothing is lost.";

/// The person closed the window, or asked yPresent to quit.
pub fn window(m: &mut Model, fx: &mut Effects<YPresent>) {
    match m.busy {
        // The window goes now and the save lands with none.
        Some(Work::Save) => {
            drafts::checkpoint_now(m, fx);
            fx.close(WindowId::PRIMARY);
            fx.linger(LINGER);
            m.lingering = true;
        }
        Some(_) => {
            m.close_when_done = true;
            say(m, CLOSING_WHEN_DONE);
        }
        None => {
            drafts::checkpoint_now(m, fx);
            fx.close(WindowId::PRIMARY);
        }
    }
}

/// The deck was saved: a linger waiting on it ends.
pub fn saved(m: &mut Model, fx: &mut Effects<YPresent>) {
    if m.lingering {
        end_after_linger(m, fx, None);
    }
}

/// The deck's save failed: a linger ends, with the content kept as a draft.
pub fn not_saved(m: &mut Model, fx: &mut Effects<YPresent>, why: &str) {
    not_closed(m);
    if m.lingering {
        end_after_linger(m, fx, Some(why));
    }
}

/// A close that waited on an export or an open that failed is dropped, and the failure already
/// said gains that the window stayed.
pub fn not_closed(m: &mut Model) {
    if std::mem::take(&mut m.close_when_done) {
        // Added to the failure already said, in both its forms.
        let (window, agents) = (m.notice.window().to_string(), m.notice.read(true).to_string());
        m.notice.set_about(window != agents, format!("{window} {NOT_CLOSED}"), || format!("{agents} {NOT_CLOSED}"));
    }
}

/// The save the window closed on has landed: yPresent ends. Whatever is unsaved now (that save
/// failed) goes to its draft first and the person is told, since there is no window left to tell
/// them in.
fn end_after_linger(m: &mut Model, fx: &mut Effects<YPresent>, why: Option<&str>) {
    if drafts::checkpoint_now(m, fx) {
        let body = match why {
            Some(why) => format!("{why} The deck is kept and comes back the next time yPresent opens."),
            None => "The unsaved deck is kept and comes back the next time yPresent opens.".to_string(),
        };
        fx.notify(Notification::titled("yPresent could not save everything").body(body));
    }
    m.lingering = false;
    fx.exit();
}

/// After every command: a close that waited for a file to be read or exported goes on once it is.
pub fn resume(m: &mut Model, fx: &mut Effects<YPresent>) {
    if m.close_when_done && m.busy.is_none() {
        m.close_when_done = false;
        m.notice.clear();
        window(m, fx);
    }
}

/// A second launch brought the window back during a linger: yPresent runs on, and the save in
/// flight lands as any other.
pub fn presented(m: &mut Model) {
    m.lingering = false;
}

/// The session or the system is ending yPresent, with no one to ask: the unsaved deck is handed
/// to its draft now, rather than after its debounce, and yPresent ends in this one command.
pub fn logging_out(m: &mut Model, fx: &mut Effects<YPresent>) {
    drafts::checkpoint_now(m, fx);
    fx.exit();
}
