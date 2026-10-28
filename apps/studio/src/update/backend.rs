//! Where pictures are made, and what that is graded.
//!
//! `set_backend` decides where every later prompt goes, so it is graded `sensitive`, and the grade
//! of the two actions that send a prompt follows it: `generate` and `variations` are `standard` to
//! a model on the person's own network and `sensitive` to anything else. The grade is published
//! when the surface is served, so it is brought to what the backend on disk deserves at start
//! (`regrade`, the first command after the surface is served; until it has run those two are
//! refused) and again the moment a change lands (`landed`), before the call that made it is
//! answered. A caller therefore cannot point Studio at a service and generate in the same breath
//! under the old, local grade, which is the one direction a grade must never be wrong in.
//!
//! A change is written down by a worker. While it is, the two actions that send a prompt are
//! refused, and a picture already queued goes to the backend it was queued against.

use serde_json::json;
use yantrik_app::control;
use yantrik_app::effects::Effects;

use super::{by_agent, needed, refuse, say};
use crate::answers;
use crate::backend;
use crate::command::{BackendAsk, BackendSet, Command};
use crate::intents::AGENT_REFUSED;
use crate::config::Kind;
use crate::jobs;
use crate::model::{display, Model, PART_BACKEND, PART_STATUS};
use crate::Studio;

type Fx = Effects<Studio>;

/// The two actions whose grade follows the backend.
const FOLLOWS_THE_BACKEND: [&str; 2] = ["generate", "variations"];

/// Publish the grade the configuration deserves. Called on the thread that owns the window, which
/// is where the registry lives. Not fatal when it cannot be: the app keeps working, and the log
/// says the grade on the approval card may be the one from before.
pub fn publish_grade(m: &mut Model) {
    let grade = m.grade();
    for action in FOLLOWS_THE_BACKEND {
        if let Err(problem) = control::regrade(action, grade) {
            tracing::error!("the grade of `{action}` could not follow the backend ({grade}): {problem}");
        }
    }
    m.published_grade = grade;
    m.graded = true;
}

/// The first command after the surface is served.
pub fn regrade(m: &mut Model) {
    publish_grade(m);
}

/// The "Where pictures are made" button. There is no dialog here on purpose. Where a person's
/// prompts go is a decision worth one file and one sentence, not a form with a field for a secret;
/// so the button says where the decision lives.
pub fn explain(m: &mut Model, fx: &mut Fx) {
    let sentence = format!(
        "Pictures are made by {} right now. Changing where Studio sends prompts is the person's choice; \
         to change it, edit ~/.config/yantrik/studio.json.",
        m.facts().place
    );
    say(m, fx, sentence);
}

/// `set_backend`: checked and written down on a worker; answered when it has landed.
pub fn set(m: &mut Model, fx: &mut Fx, ask: BackendAsk) {
    // The framework refuses an agent's call to this intent before it gets here
    // (`Agents::Refused`, intents.rs). This is the second lock, for any route that reaches the
    // command without it: refused before anything else is read, and nothing is connected to.
    if by_agent(fx) {
        return refuse(m, fx, AGENT_REFUSED);
    }
    let Some(kind) = needed(m, fx, "set_backend", "kind", &ask.kind) else { return };
    if m.changing_backend {
        return refuse(m, fx, "Studio is already changing where pictures are made; ask again in a moment.");
    }
    m.changing_backend = true;
    let ask = BackendAsk { kind, ..ask };
    let (folder, settings) = (m.config.output_folder.clone(), m.places.settings.clone());
    fx.spawn_or(
        Command::BackendSet(Err("Studio's worker stopped before it wrote the choice down. Nothing was changed.".into())),
        move |_| Command::BackendSet(jobs::set_backend(&ask, &folder, settings.as_deref())),
    );
}

/// A backend change was checked and written down, or refused.
pub fn landed(m: &mut Model, fx: &mut Fx, result: Result<BackendSet, String>) {
    m.changing_backend = false;
    let set = match result {
        Ok(set) => set,
        Err(problem) => return refuse(m, fx, problem),
    };
    // The notice is only set for the thing that stops a generation. `facts.note` also carries the
    // everyday caveats, and those belong in `describe`, where a caller reads them, rather than at
    // the front of every one-line summary.
    let notice = if set.config.backend.kind == Kind::OpenAiImages {
        set.config.backend.api_key().err().unwrap_or_default()
    } else {
        String::new()
    };
    m.config = set.config;
    m.backend = backend::build(&m.config);
    m.backend_generation += 1;
    m.notice = notice;
    // Before the call that made the change is answered: the grade follows the backend at once.
    publish_grade(m);
    if !set.written {
        m.notice = "The backend is changed for this session, but there is no home directory to write it down in, so it will not survive a restart.".to_string();
    }
    fx.changed(PART_BACKEND);
    fx.changed(PART_STATUS);
    fx.changed(crate::model::PART_NOTICE);
    let facts = m.facts();
    // Never the key, and never the variable's contents. The name is here because a person has to be
    // told which variable to export.
    answers::settle(
        m,
        fx,
        json!({
            "backend": facts.kind,
            "where": facts.place,
            "configured": facts.configured,
            "api_key_env": crate::config::shown_variable(&m.config.backend.api_key_env),
            "generate_is_now_graded": facts.grade,
            "prompt_leaves_this_machine": facts.prompt_leaves,
            "note": facts.note,
            "output_folder": display(&m.places.gallery),
        }),
    );
}
