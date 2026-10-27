//! A launch on a Mind's display, from start to end: hand the launch to the app's one instance
//! (started on the person's display first when there is none), and show the Mind's window it
//! draws for this launch.
//!
//! The instance answers a Mind's launch with that Mind's window (`Core::present_for_mind`): a
//! new one drawn for this launch's view, which this process then shows until it closes, or the
//! one already open on this display, raised, and this launch ends. An instance from a build before
//! the Mind had windows of its own answers as it always did, and this launch ends as it always
//! did.

use std::time::{Duration, Instant};

use super::helper::{self, Listening};
use super::spawn;
use crate::launch::{self, Launch};
use crate::present::Presented;

/// How long a launch waits for an instance it started to answer: the instance's own start, its
/// model and its surface, on a loaded machine.
pub const START_WAIT: Duration = Duration::from_secs(15);

/// Between two looks at whether the instance is up.
const STEP: Duration = Duration::from_millis(200);

/// What became of a Mind's launch.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The Mind's window was shown here and has closed, or was already open on this display.
    Done,
    /// It could not be done, and why.
    Failed(String),
    /// There is no person's display to start the instance on: this launch runs the app itself,
    /// drawn here and masked throughout, as before.
    RunHere,
}

/// The launch, for `app` (its id, for what is said) under the instance name `instance`.
pub fn run(app: &str, instance: &str, launch: &Launch) -> Outcome {
    let listening = match Listening::bind(instance) {
        Ok(listening) => listening,
        Err(e) => {
            tracing::warn!(app, error = %e, "no socket for a view helper; running here, masked");
            return Outcome::RunHere;
        }
    };
    let deadline = Instant::now() + START_WAIT;
    let mut started = false;
    loop {
        if crate::instance::running(instance) {
            match launch::hand_over_with(instance, launch, Some(listening.path())) {
                Presented::Streamed => {
                    tracing::info!(app, "the instance draws the Mind's window; showing it here");
                    return match helper::show(listening, instance) {
                        Ok(()) => Outcome::Done,
                        Err(why) => Outcome::Failed(why),
                    };
                }
                Presented::Shown | Presented::Elsewhere(_) => {
                    tracing::info!(app, "the Mind's window was already open; raised it");
                    return Outcome::Done;
                }
                Presented::Unanswered(_) if Instant::now() < deadline => {}
                other => {
                    let said = other.sentence(app).unwrap_or_else(|| format!("{app} did not open"));
                    return Outcome::Failed(said);
                }
            }
        } else if !started {
            let session = spawn::persons_session();
            if spawn::person_display(&session).is_none() {
                return Outcome::RunHere;
            }
            if let Err(e) = spawn::start_instance(&session) {
                return Outcome::Failed(format!("{app} could not be started on the person's display: {e}"));
            }
            started = true;
        } else if Instant::now() >= deadline {
            return Outcome::Failed(format!(
                "{app} was started on the person's display and did not come up within {} s",
                START_WAIT.as_secs()
            ));
        }
        std::thread::sleep(STEP);
    }
}
