//! The only function that changes the model: every button, intent and job result lands here as a
//! `Command`, and asks for its effects.
//!
//! Every mutation goes through [`start_job`], the button and the Mind's action alike, so a
//! person and a mind cannot be told different things about the same command. What this replaces
//! was nine copies of `let _ = Command::new(runtime_cmd())...output();`, run on the window's
//! thread: the result of every `start`, `stop`, `restart`, `rm -f`, `run -d`, `pull`, `rmi`,
//! `volume create` and `volume rm` was dropped on the floor.

use serde_json::json;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Origin;
use yantrik_app::jobs::{Concurrency, JobKey, Scope};
use yantrik_app::mind::AskError;
use yantrik_app::rail;
use yantrik_app::Refused;

use crate::machine::Listing;
use crate::model::{
    Command, Job, Landed, Model, Named, Verb, PART_AI, PART_LISTING, PART_LOGS, PART_NOTICE, PART_TAB, TABS,
};
use crate::runtime;
use crate::verdict;
use crate::Containers;

/// The notice when a job ended without an answer (it panicked, or the pool was full).
pub const LOST: &str = "The container runtime did not answer; try again.";

pub fn update(m: &mut Model, command: Command, fx: &mut Effects<Containers>) -> Result<(), Refused> {
    match command {
        Command::Refresh => start_job(m, fx, Job::Refresh),
        Command::ShowTab(tab) => {
            m.tab = tab.min(TABS.len() - 1);
            fx.changed(PART_TAB);
        }
        Command::Row { verb, id } => {
            let job = match verb {
                Verb::Logs => Job::Logs { id, named: None },
                verb => Job::Change { argv: verb.argv(&id), named: None },
            };
            start_job(m, fx, job);
        }
        Command::Named { verb, needle } => {
            // Validated before anything changes: a container that is not here is a refusal,
            // never the success of having done nothing to it.
            let rows = m.rows();
            let (id, name) = runtime::resolve(&rows, &needle)
                .map(|i| rows[i].clone())
                .ok_or_else(|| Refused::new("no_container", format!("no container here is called \"{needle}\"")))?;
            let named = Some(Named { verb, id: id.clone(), name });
            let job = match verb {
                Verb::Logs => Job::Logs { id, named },
                verb => Job::Change { argv: verb.argv(&id), named },
            };
            start_job(m, fx, job);
        }
        Command::RunImage(image) => {
            // The button is disabled on an empty field; spaces alone still name no image, and the
            // runtime would only answer with an error about an empty reference.
            let image = image.trim();
            if !image.is_empty() {
                start_job(m, fx, change(&["run", "-d", "--", image]));
            }
        }
        Command::PullImage(image) => start_job(m, fx, change(&["pull", "--", &image])),
        Command::RemoveImage(id) => start_job(m, fx, change(&["rmi", &id])),
        Command::CreateVolume(name) => start_job(m, fx, change(&["volume", "create", &name])),
        Command::RemoveVolume(name) => start_job(m, fx, change(&["volume", "rm", &name])),
        Command::Landed(landed) => landed_job(m, fx, *landed),
        Command::Lost(job) => {
            forget(m, &job);
            m.notice = LOST.to_string();
            fx.changed(PART_NOTICE);
        }
        Command::LogsClosed => {
            m.logs.open = false;
            fx.changed(PART_LOGS);
        }
        Command::Summarize => {
            // The header's AI button asks what the rail's suggestion asks, of the same companion.
            let Some(prompt) = crate::rail::log_question(m) else {
                m.ai.response = "There is no log open to read.".into();
                fx.changed(PART_AI);
                return Ok(());
            };
            m.ai.working = true;
            m.ai.response.clear();
            fx.changed(PART_AI);
            let mind = m.mind.clone();
            fx.spawn_keyed_or(
                JobKey { feature: "ai", scope: Scope::App },
                Concurrency::ReplacePrevious,
                Command::Summarized(Err(AskError::Failed("the question did not finish".into()))),
                move |_| Command::Summarized(mind.ask(&prompt)),
            );
        }
        Command::Summarized(outcome) => {
            m.ai.working = false;
            m.ai.response = match outcome {
                Ok(text) => text,
                Err(e) => e.to_string(),
            };
            fx.changed(PART_AI);
        }
        Command::DismissAi => {
            m.ai.response.clear();
            fx.changed(PART_AI);
        }
        Command::Rail(command) => rail::update::<Containers>(m, command, fx)?,
    }
    Ok(())
}

fn change(argv: &[&str]) -> Job {
    Job::Change {
        argv: argv.iter().map(|a| a.to_string()).collect(),
        named: None,
    }
}

/// One job for the runtime, off the UI thread. Each turn runs alone (`Machine`), so they keep
/// the order they were asked in; a second press of a button whose first has not landed is the
/// same request and starts nothing.
fn start_job(m: &mut Model, fx: &mut Effects<Containers>, job: Job) {
    // Nothing is drawn until the job lands; the notice is the narrowest part to name, so the
    // lists are not rebuilt for a press.
    fx.changed(PART_NOTICE);
    if let Some(argv) = job.argv() {
        if fx.origin() == Origin::Local && m.in_flight.contains(&argv) {
            tracing::debug!(?argv, "already asked; this press is the same request");
            return;
        }
        m.in_flight.push(argv);
    }
    let machine = m.machine.clone();
    let lost = Command::Lost(job.clone());
    fx.spawn_or(lost, move |_| {
        let (exit, listing) = match job.argv() {
            None => (None, machine.list()),
            Some(argv) => {
                let performed = machine.perform(&argv);
                (Some(performed.exit), performed.listing)
            }
        };
        Command::Landed(Box::new(Landed { job, exit, listing }))
    });
}

/// A job's command stopped being in flight.
fn forget(m: &mut Model, job: &Job) {
    if let Some(argv) = job.argv() {
        if let Some(at) = m.in_flight.iter().position(|a| *a == argv) {
            m.in_flight.remove(at);
        }
    }
}

/// The machine re-read, then the command's own verdict on screen and to whoever asked.
///
/// The re-read comes first on purpose: it writes the notice about the machine, and the command's
/// message is the more specific of the two, so it has to be written last.
fn landed_job(m: &mut Model, fx: &mut Effects<Containers>, landed: Landed) {
    let Landed { job, exit, listing } = landed;
    forget(m, &job);
    let runtime = listing.runtime;
    let turn = listing.turn;
    adopt(m, fx, listing);

    let (Some(exit), Some(argv)) = (exit, job.argv()) else {
        // A refresh: answered from the committed model by its intent's reply.
        return;
    };
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    // Both streams: a container's own stderr is part of its log and `docker logs` passes it
    // through, so taking stdout alone showed an empty pane for every program that logs the
    // ordinary way — which reads as "this container said nothing".
    let log_text = match (&job, &exit) {
        (Job::Logs { .. }, runtime::Exit::Ran { code: Some(0), stdout, stderr }) => format!("{stdout}{stderr}"),
        _ => String::new(),
    };
    fx.changed(PART_NOTICE);
    if let Err(reason) = runtime::outcome(runtime, &argv, exit) {
        m.notice = reason.clone();
        fx.settle(Err(Refused::new("runtime_refused", reason)));
        return;
    }
    m.notice.clear();
    match job {
        Job::Refresh => {}
        Job::Change { named: None, .. } => fx.settle(Ok(())),
        Job::Change { named: Some(named), .. } => match verdict::of(m, &named) {
            Ok(answer) => {
                m.answer = answer;
                fx.settle(Ok(()));
            }
            // A command that exited zero but did not do what it was asked is a failure too.
            // Said on screen as well as returned, because a row that is still in the list after
            // a delete otherwise reads as "the delete has not drawn yet".
            Err(reason) => {
                m.notice = reason.clone();
                fx.settle(Err(Refused::new("not_done", reason)));
            }
        },
        Job::Logs { id, named } => {
            if let Some(named) = &named {
                m.answer = json!({ "container": named.name, "tail": verdict::last_lines(&log_text, 40) });
            }
            // An older read that lands after a newer one is not drawn over it.
            if turn > m.logs.turn {
                m.logs.text = log_text;
                m.logs.container = id;
                m.logs.open = true;
                m.logs.turn = turn;
                fx.changed(PART_LOGS);
            }
            fx.settle(Ok(()));
        }
    }
}

/// A listing, unless the one on screen was read in a later turn: then this one is stale.
fn adopt(m: &mut Model, fx: &mut Effects<Containers>, listing: Listing) {
    if listing.turn <= m.listed_turn {
        tracing::debug!(turn = listing.turn, shown = m.listed_turn, "a stale listing, dropped");
        return;
    }
    // The state of the machine, said on screen. A command's own failure is more specific than
    // this and overwrites it after.
    m.notice = listing.availability.trouble(listing.runtime).unwrap_or_default();
    m.runtime = listing.runtime.to_string();
    m.health = listing.availability;
    m.containers = listing.containers;
    m.images = listing.images;
    m.volumes = listing.volumes;
    m.listed_turn = listing.turn;
    fx.changed(PART_LISTING);
    fx.changed(PART_NOTICE);
}
