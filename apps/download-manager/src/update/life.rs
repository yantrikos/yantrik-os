//! The app's life: the close, the logout, and the order they stop the downloads in.
//!
//! Closing the window pauses what is downloading, as it always did: the partial files stay where
//! they are, and a download continues from its bytes when the person resumes it. The old app
//! paused, wrote the list and ended without waiting for a writer to notice. Now the app runs on
//! without its window until every writer has reached its next chunk boundary (where it flushes the
//! file and records itself as paused), or until a backstop passes, and then ends: no file is left
//! mid-write, and the list says `paused` where it would have said `downloading`.

use std::time::Duration;

use yantrik_app::effects::Effects;

use crate::command::Command;
use crate::model::Model;
use crate::Downloads;

type Fx = Effects<Downloads>;

/// How long the desk waits for the writers to stop. A writer is at a chunk boundary within a
/// fraction of a second, unless it is blocked on a socket: then the file is whole up to its last
/// chunk, and the next start reconciles the list against it.
const WRITERS_WAIT: Duration = Duration::from_secs(8);

/// How long a close waits for the desk before the framework ends the app anyway (`fx.linger`'s
/// backstop, which logs that it did): the writers' wait, and the list written behind it.
pub const CLOSE_BACKSTOP: Duration = Duration::from_secs(12);

/// What a command that would start something is told while the app is closing.
pub const CLOSING: &str = "Downloads is closing, so nothing more is started.";

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) {
    match command {
        Command::Closing(window) => {
            // The window goes at once; the app runs on without it until the writers have stopped.
            m.closing = true;
            fx.close(window);
            fx.linger(CLOSE_BACKSTOP);
            settle(m, fx);
        }
        Command::Reopened => {
            // The close is cancelled. The downloads are still paused, as the person asked by
            // closing; the list shows them so and Resume continues them. A logout's `ending` is
            // never cancelled.
            m.closing = false;
            fx.unchanged();
        }
        Command::Ending => {
            // Nothing asks first: a logout has nobody there to answer, and nothing Downloads holds
            // is unsaved (the list is written on every transition). The writers are paused, and
            // then the app ends.
            m.ending = true;
            if m.busy() {
                settle(m, fx);
            } else {
                fx.exit();
            }
            fx.unchanged();
        }
        Command::Settled => settled(m, fx),
        _ => {}
    }
}

/// Pause every writer and write the list, behind every command already asked of the engine.
fn settle(m: &mut Model, fx: &mut Fx) {
    let reply = m.desk.ask(|engine| engine.settle_for_exit(WRITERS_WAIT));
    m.work += 1;
    fx.spawn_or(Command::Settled, move |_| {
        let _ = reply.wait();
        Command::Settled
    });
}

/// The writers have stopped and the list is written. A close or a logout waiting for this ends here.
fn settled(m: &mut Model, fx: &mut Fx) {
    m.work = m.work.saturating_sub(1);
    // Anything a worker had to say as it stopped (a transfer that failed in its last chunk) is
    // sent before the app goes.
    super::sync(m, fx);
    if m.closing || m.ending {
        fx.exit();
    }
}
