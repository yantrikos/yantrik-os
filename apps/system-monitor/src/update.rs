//! The only function that changes the model: every button, intent, timer and job result lands
//! here as a `Command`, and asks for its effects.

use yantrik_app::effects::Effects;
use yantrik_app::jobs::{Concurrency, JobKey, Scope};
use yantrik_app::mind::AskError;
use yantrik_app::rail::{self, RailCommand};
use yantrik_app::Refused;

use crate::figures::Figures;
use crate::machine;
use crate::model::{Command, Model, PART_AI, PART_NOTICE, PART_PROCESSES, PART_READINGS, PART_QUIET, PROCESS_LIMIT};
use crate::outcome;
use crate::readout::Readout;
use crate::report;
use crate::{SystemMonitor, OFF_SCREEN_EVERY};

/// The kill's job ended without an answer (it panicked, or the pool could not start it).
const KILL_DID_NOT_FINISH: &str = "the attempt to end it did not finish; look at the list before trying again";
/// The AI question's job ended without an answer.
const EXPLAIN_DID_NOT_FINISH: &str = "the question did not finish; try again";

pub fn update(m: &mut Model, command: Command, fx: &mut Effects<SystemMonitor>) -> Result<(), Refused> {
    match command {
        Command::Poll => {
            if m.sampling.is_some() || !due(m, fx) {
                // Nothing read: nothing to draw, and nothing new for the rail to look at.
                fx.unchanged();
                return Ok(());
            }
            fx.changed(PART_QUIET);
            let seq = m.next_sample;
            m.next_sample += 1;
            m.sampling = Some(seq);
            m.sampled_at = Some(fx.now());
            let machine = m.machine.clone();
            let sort = m.sort_key();
            fx.spawn_keyed_or(
                JobKey {
                    feature: "sample",
                    scope: Scope::App,
                },
                Concurrency::RejectIfRunning,
                Command::SampleFailed { seq },
                move |_| Command::Sampled {
                    seq,
                    reading: machine.reading(sort, PROCESS_LIMIT),
                },
            );
        }
        Command::Sampled { seq, reading } => {
            if m.sampling != Some(seq) {
                // Not the reading in flight: delivered twice, or after the one it belonged to
                // was given up on. The screen keeps the newer machine.
                fx.changed(PART_QUIET);
                return Ok(());
            }
            m.sampling = None;
            let drawn = Figures::of(m);
            m.readout = Readout::from_snapshot(&reading.snap);
            m.cpu_window_ms = reading.snap.cpu.window_ms;
            m.status = report::assess(&reading.snap);
            m.procs = reading.procs;
            m.reading = Some(reading.provenance);
            if Figures::of(m) != drawn {
                // A frame will draw it, if the window is on screen: the next poll asks.
                m.shown.owe(fx.frames());
            }
            fx.changed(PART_READINGS);
            fx.changed(PART_PROCESSES);
            fx.changed(PART_NOTICE);
            // The rail's rows say the same figures: refreshed in this command, so a reading is
            // one frame, not a second one when the rail's debounce runs 300 ms later.
            rail::update::<SystemMonitor>(m, RailCommand::Refresh, fx)?;
        }
        Command::Back => {
            fx.unchanged();
            m.shown.back();
            if m.off_screen {
                // What is on screen may be half a minute old: read the machine now.
                fx.then(Command::Poll);
            }
        }
        Command::SampleFailed { seq } => {
            fx.changed(PART_QUIET);
            if m.sampling == Some(seq) {
                m.sampling = None;
            }
        }
        Command::SortBy(column) => {
            m.sort_column = column;
            fx.changed(PART_PROCESSES);
        }
        Command::SortProcesses { column, by } => {
            m.sort_column = column;
            m.sorted_by = by;
            fx.changed(PART_PROCESSES);
        }
        Command::Search(query) => {
            // The list narrows as the person types; it used to wait for the next poll, so the
            // box looked as if it did nothing for up to two seconds after every key.
            m.search = query;
            fx.changed(PART_PROCESSES);
        }
        Command::End { pid, force } => {
            let pid = match outcome::checked_pid(pid) {
                Ok(pid) => pid,
                Err(reason) => {
                    not_ended(m, reason, fx);
                    return Ok(());
                }
            };
            let name = m.process_name(pid);
            let machine = m.machine.clone();
            // The kill and the read-back of the process table block for up to a second: a job's.
            // The answer is the person's on the strip and the Mind's through `settle`.
            fx.spawn_or(
                Command::Ended {
                    outcome: Err(format!("{pid}: {KILL_DID_NOT_FINISH}")),
                },
                move |_| Command::Ended {
                    outcome: machine::end(machine.as_ref(), pid, &name, force),
                },
            );
            fx.changed(PART_QUIET);
        }
        Command::Ended { outcome } => match outcome {
            Ok(observed) => {
                tracing::info!("{}", observed.sentence());
                m.failure = None;
                m.ended = Some(observed);
                fx.changed(PART_NOTICE);
                fx.settle(Ok(()));
            }
            Err(reason) => not_ended(m, reason, fx),
        },
        Command::Explain => {
            if let Some(hint) = m.rail.reach().hint() {
                m.ai.response = hint.into();
                fx.changed(PART_AI);
                return Ok(());
            }
            let prompt = crate::rail::machine_question(m);
            m.ai.working = true;
            m.ai.response.clear();
            fx.changed(PART_AI);
            let mind = m.mind.clone();
            // One question at a time: a second press replaces the first, whose late answer is
            // dropped rather than drawn over the newer one.
            fx.spawn_keyed_or(
                JobKey {
                    feature: "ai",
                    scope: Scope::App,
                },
                Concurrency::ReplacePrevious,
                Command::Explained(Err(AskError::Failed(EXPLAIN_DID_NOT_FINISH.into()))),
                move |_| Command::Explained(mind.ask(&prompt)),
            );
        }
        Command::Explained(outcome) => {
            m.ai.working = false;
            m.ai.response = match outcome {
                Ok(text) => text,
                Err(e) => e.to_string(),
            };
            fx.changed(PART_AI);
        }
        Command::DismissAi => {
            // Clears the answer as well as the panel: reopening it later must not show a reading
            // of the machine as it was some minutes ago.
            m.ai.response.clear();
            fx.changed(PART_AI);
        }
        Command::Rail(command) => rail::update::<SystemMonitor>(m, command, fx)?,
    }
    Ok(())
}

/// Whether a poll takes a reading: always while the window is on screen; off screen (the last
/// reading that changed the screen was never drawn), once every [`OFF_SCREEN_EVERY`], so the
/// Mind's `describe` stays that fresh with nobody looking and nothing is drawn meanwhile.
fn due(m: &mut Model, fx: &Effects<SystemMonitor>) -> bool {
    if m.shown.on_screen(fx.frames()) {
        m.off_screen = false;
        return true;
    }
    m.off_screen = true;
    m.sampled_at.is_none_or(|at| fx.now().saturating_duration_since(at) >= OFF_SCREEN_EVERY)
}

/// A kill that did not end its process: said on the strip where the person is looking, and
/// answered to whoever asked, so the two cannot come away with different accounts. A button's
/// refusal is a notice, not a refused command (whose effects would be dropped).
fn not_ended(m: &mut Model, reason: String, fx: &mut Effects<SystemMonitor>) {
    tracing::warn!("Kill failed: {reason}");
    m.failure = Some(reason.clone());
    fx.changed(PART_NOTICE);
    fx.settle(Err(Refused::new("not_ended", reason)));
}
