//! Working the sheet out: always in a job, never on the window's thread.
//!
//! A formula can read up to twenty million cells (`formula::budget`), so what a cell works out to
//! is not something a keystroke waits for. Typing changes the text and the revision at once
//! (`Sheet::put`); the job is given the text at that revision and hands back what every cell comes
//! to; and its result is taken only if the sheet is still at that revision. A result for an older
//! revision is dropped (`update::solved`), because a newer job has been, or is being, started for
//! the text that replaced it.

use yantrik_app::effects::Effects;
use yantrik_app::jobs::{Concurrency, JobKey, Scope};

use crate::command::{Command, Reply};
use crate::{Model, YSheets};

/// After every command: the sheet in front is worked out again if it is not, unless a job for its
/// revision is on its way or has been lost.
pub fn ensure(m: &mut Model, fx: &mut Effects<YSheets>) {
    let sheet = m.sheet();
    if sheet.is_solved() || m.solving == sheet.rev() || m.solve_lost == Some(sheet.rev()) {
        return;
    }
    spawn(m, fx, None);
}

/// Starts the job that works out the sheet as it is now. With a `reply`, the call that needs the
/// result is answered from this job's own, whatever is typed meanwhile; without, a newer job for
/// the same sheet takes this one's place.
pub fn spawn(m: &mut Model, fx: &mut Effects<YSheets>, reply: Option<Reply>) {
    let snapshot = m.sheet().snapshot();
    let before = m.sheet().values().clone();
    let rev = snapshot.rev();
    m.solving = rev;
    let answers = reply.is_some();
    let lost = Command::SolveLost { rev, then: reply.clone() };
    let job = move |_: &yantrik_app::Cancel| {
        let solved = snapshot.solve();
        let changed = solved.changed_from(&before);
        Command::Solved { solved: Box::new(solved), changed, then: reply }
    };
    if answers {
        // A call waits for this one: it is never replaced, or the call would be told so.
        fx.spawn_or(lost, job);
    } else {
        let key = JobKey { feature: "solve", scope: Scope::App };
        fx.spawn_keyed_or(key, Concurrency::ReplacePrevious, lost, job);
    }
}
