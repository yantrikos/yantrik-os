//! The only code that changes the model: every button, every call from a Mind and every landing
//! from a worker arrives here as a `Command`, and asks for its effects.
//!
//! The person's buttons and the Mind's actions are the same requests, so a person and an agent
//! cannot get different answers about whether something worked. Nothing slow runs here: a name is
//! turned into a file, a picture is made and filed, the folder is read and a backend choice is
//! written down on workers (`jobs.rs`), and what they found comes back as a command.
//!
//! - `make.rs`: the queue: asking for pictures, starting them, filing what comes back, stopping.
//! - `files.rs`: what is done with one picture: the record pane, variations, upscale, open, delete.
//! - `gallery.rs`: the output folder, and its poll.
//! - `backend.rs`: where pictures are made, and what that is graded.
//! - this file: the window's own small state, the refusals, and the dispatch.

use std::time::Duration;

use serde_json::Value;
use yantrik_app::effects::Effects;
use yantrik_app::intents::Origin;
use yantrik_app::timers::TimerId;
use yantrik_app::Refused;

use crate::answers;
use crate::command::Command;
use crate::mind_view;
use crate::model::{Field, Model, PART_FORM, PART_NOTICE, PART_WIDE, WIDE_WINDOW};
use crate::said::Said;
use crate::Studio;

mod backend;
mod files;
mod gallery;
mod make;

type Fx = Effects<Studio>;

/// Ask whether the output folder moved, and read it again if it did. Four seconds, and the ask is
/// two directory timestamps rather than a listing, so this is free while nothing is happening.
pub const POLL: TimerId = TimerId("studio.poll");
pub const POLL_EVERY: Duration = Duration::from_secs(4);
/// The name of the draft the unsent prompt is kept under.
pub const PROMPT_DRAFT: &str = "prompt";
/// Brings `generate` and `variations` to the grade the backend on disk deserves, once the surface
/// is served: the first thing this app does after publishing.
pub const REGRADE: TimerId = TimerId("studio.regrade");

/// The first effects: the poll, and the grade.
#[cfg(test)]
pub use crate::intents::AGENT_REFUSED;

pub fn first(fx: &mut Fx) {
    fx.every(POLL, POLL_EVERY, Command::Poll);
    fx.after(REGRADE, Duration::ZERO, Command::Regrade);
}

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    // An answer is written only in the command that settles it (`answers.rs`); what an earlier
    // command wrote is never anyone else's. And a command draws only the parts it names.
    m.answer = Value::Null;
    fx.unchanged();
    // Drawn for a Mind, nothing is typed, asked for or filed through the window: it says why, as
    // well as the caller, and the refused text does not stay in the field.
    if let Err(refused) = yantrik_app::for_mind::refuse_if(m.for_mind, mind_view::refuses(&command)) {
        fx.changed(PART_FORM);
        refuse(m, fx, refused.sentence);
        return Ok(());
    }
    match command {
        Command::Edit(field, text) => {
            let prompt = field == Field::Prompt;
            m.form.set(field, text);
            fx.changed(PART_FORM);
            // Typing is kept as it goes, so a crash does not lose it either.
            if prompt {
                keep_prompt(m, fx, false);
            }
        }
        Command::Choose(index) => files::choose(m, fx, index),
        Command::Resized(width) => {
            let wide = width > WIDE_WINDOW;
            if m.wide != wide {
                m.wide = wide;
                fx.changed(PART_WIDE);
            }
        }
        Command::Unknown(id) => refuse(m, fx, format!("Studio has no button called `{id}`.")),
        Command::ExplainBackend => backend::explain(m, fx),
        Command::OpenFolder => files::open_folder(m, fx),
        Command::Generate(plan) => make::generate(m, fx, plan.map(|plan| *plan)),
        Command::Variations { of, count, fill } => files::variations(m, fx, of, count, fill),
        Command::Upscale { of, factor } => files::upscale(m, fx, of, factor),
        Command::Open { of } => files::open(m, fx, of),
        Command::Delete { of } => files::delete(m, fx, of),
        Command::SetBackend(ask) => backend::set(m, fx, *ask),
        Command::Cancel(which) => make::cancel(m, fx, which),
        Command::Refresh => gallery::refresh(m, fx),
        Command::Resolved { next, result } => files::resolved(m, fx, next, result),
        Command::Trashed(result) => files::trashed(m, fx, result),
        Command::Opened { shown, outcome } => files::opened(m, fx, shown, outcome),
        Command::Made { id, outcome } => make::made(m, fx, id, outcome),
        Command::MadeLost(id) => make::made(
            m,
            fx,
            id,
            Err(Some("Studio's worker for this picture stopped before it answered.".to_string())),
        ),
        Command::BackendSet(result) => backend::landed(m, fx, result),
        Command::Listed(listed) => gallery::listed(m, fx, *listed),
        Command::Lost(what) => {
            m.reading = None;
            refuse(m, fx, format!("Studio's {what} stopped before it answered. Nothing was changed; try again."));
        }
        Command::Unchanged => m.reading = None,
        Command::Poll | Command::Activated => gallery::poll(m, fx),
        Command::Closing(window) => {
            keep_prompt(m, fx, true);
            fx.close(window);
        }
        Command::Ending => {
            keep_prompt(m, fx, true);
            fx.exit();
        }
        Command::Regrade => backend::regrade(m),
    }
    Ok(())
}

/// Put `sentence` on the notice line.
pub fn say(m: &mut Model, fx: &mut Fx, sentence: String) {
    m.notice = sentence;
    fx.changed(PART_NOTICE);
}

/// Failure is said twice: to the caller in its form, and on the notice line the person sees (and
/// `describe.notice` carries) in the person's.
pub fn refuse(m: &mut Model, fx: &mut Fx, said: impl Into<Said>) {
    let said = said.into();
    answers::fail(fx, said.agent);
    say(m, fx, said.person);
}

/// Whether the command being applied was a Mind's call: the rule for the files an agent may name
/// asks.
pub fn by_agent(fx: &Fx) -> bool {
    matches!(fx.origin(), Origin::Surface { agent: true })
}

/// A required text argument that has to say something, or a refusal naming it.
///
/// The dispatch refuses a *missing* required argument before the update runs. This is the other
/// half: one that is present and blank. `generate prompt=""` must not report success.
pub fn needed(m: &mut Model, fx: &mut Fx, action: &str, name: &str, value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        let agent = format!("`{action}` was given an empty `{name}`.");
        refuse(m, fx, Said::new(agent, "Nothing was named, so nothing was done."));
        return None;
    }
    Some(value.to_string())
}

/// Why a call that would send a prompt somewhere cannot be taken now, if it cannot.
pub fn not_yet(m: &Model) -> Option<Said> {
    if !m.graded {
        return Some(Said::both("Studio is still starting; ask again in a moment."));
    }
    if m.changing_backend {
        return Some(Said::both("Studio is changing where pictures are made; ask again in a moment."));
    }
    None
}

/// Keep the prompt in the box if it was never sent, or let go of the draft if it is empty or was.
/// `now` writes at once (a close, a quit, a logout, and none of them asks); otherwise it is
/// written a moment after the last key.
pub fn keep_prompt(m: &Model, fx: &mut Fx, now: bool) {
    if m.for_mind {
        return;
    }
    let text = m.form.prompt.as_str();
    if text.trim().is_empty() || text.trim() == m.sent_prompt {
        fx.discard_draft(PROMPT_DRAFT);
    } else if now {
        fx.checkpoint_draft(PROMPT_DRAFT, text.as_bytes().to_vec());
    } else {
        fx.save_draft(PROMPT_DRAFT, text.as_bytes().to_vec());
    }
}
