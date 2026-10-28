//! The app's life: the close, the logout, and what each waits for.
//!
//! The library's commands are quick (a file written, a game compiled), but one that is on the lane
//! when the window closes must land: a spec is whole or it is not there, and the person is told
//! which. So a close with a command on the lane closes the window and the app runs on without it
//! until the lane is empty, or until a backstop passes. A browser job (play, verify, screenshot)
//! is not waited for: it writes nothing but its verdict and screenshot, each whole or not at all,
//! and ending abandons it as it always did.

use std::time::Duration;

use yantrik_app::effects::Effects;

use super::drafts;
use crate::command::Command;
use crate::model::Model;
use crate::Arcade;

type Fx = Effects<Arcade>;

/// How long a close waits for the lane before the framework ends the app anyway (`fx.linger`'s
/// backstop, which logs that it did).
pub const CLOSE_BACKSTOP: Duration = Duration::from_secs(8);

/// What a command that would start something is told while the app is closing.
pub const CLOSING: &str = "Arcade is closing, so nothing more is started.";

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) {
    match command {
        Command::Closing(window) => {
            // The window goes at once; the app runs on without it until the lane is empty. The
            // editor's text is written now, in case the backstop is what ends the app.
            m.closing = true;
            // A closed app has no use for a browser job: its browser is stopped and its folders
            // removed now, not left for a grace period the framework would only wait out, and the
            // job is ended with the closing sentence.
            crate::browser::kill_live();
            m.jobs.fail_running(CLOSING);
            drafts::at_the_end(m, fx);
            fx.close(window);
            fx.linger(CLOSE_BACKSTOP);
            drained(m, fx);
        }
        Command::Reopened => {
            // The close is cancelled. A logout's `ending` is never cancelled.
            m.closing = false;
            fx.unchanged();
        }
        Command::Ending => {
            // Nothing asks first: a logout has nobody there to answer, and the one thing Arcade
            // holds that is unsaved, the editor's text, is written as a draft.
            m.ending = true;
            drafts::at_the_end(m, fx);
            // A browser job is not waited for: its browser has a debugging port open, so it is
            // stopped with the app and not left running.
            crate::browser::kill_live();
            if !m.busy() {
                fx.exit();
            }
            fx.unchanged();
        }
        _ => {}
    }
}

/// A command has landed (or failed): a close or a logout waiting for the lane ends here.
pub fn drained(m: &mut Model, fx: &mut Fx) {
    if (m.closing || m.ending) && !m.busy() {
        drafts::at_the_end(m, fx);
        crate::browser::kill_live();
        fx.exit();
    }
}
