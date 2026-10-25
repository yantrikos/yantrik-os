//! The end of a run: the window closed, a logout or a quit, and the editor's typing kept in the
//! draft lane so neither loses it.
//!
//! The old app had no question to ask on close and asks none now. What it lost was the typing in
//! the editor that no button had committed yet, and any write still running when the window went.
//! Now the typing is in the framework's draft lane, written within its debounce of the last key
//! and at once when the app ends, and offered back at the next start (`lib.rs`); and the end
//! waits for the work in flight (`update`'s tail ends the app once `work` is 0).

use std::time::Duration;

use yantrik_app::app::WindowId;

use super::Fx;
use crate::model::{Model, NOTHING};

/// How long a closed window's work may take before the framework ends the app anyway
/// (`fx.linger`'s backstop, which logs that it did). Each write is a local file, so this is
/// reached only by a disk that stopped answering, or a clipboard that never returned.
pub const CLOSE_BACKSTOP: Duration = Duration::from_secs(10);

/// The last window closed with work in flight or typing unsaved: the typing goes to the draft
/// lane now, the window closes at once, as the person asked, and the process runs on without it
/// until the work lands. A second launch meanwhile opens it again (`Event::Presented`).
pub(super) fn closing(m: &mut Model, window: WindowId, fx: &mut Fx) {
    super::drafts::keep(m, fx, true);
    fx.close(window);
    if m.work > 0 {
        m.closing = true;
        fx.linger(CLOSE_BACKSTOP);
    }
}

/// A logout (a signal, the display going) or a person's quit. Nothing asks first: a logout has
/// nobody there to answer. The typing goes to the draft lane in this command, and the app ends
/// once the work in flight has landed; after a signal the framework still ends it at its own
/// deadline, whatever is in flight.
pub(super) fn ending(m: &mut Model, fx: &mut Fx) {
    super::drafts::keep(m, fx, true);
    m.ending = true;
}

/// A second launch opened the window again while a close waited: the close is over. The work in
/// flight lands as any other, and new work is taken again. A logout's end is not a close and is
/// not cancelled.
pub(super) fn reopened(m: &mut Model, fx: &mut Fx) {
    fx.changed(NOTHING);
    m.closing = false;
}
