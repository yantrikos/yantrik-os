//! The only code that changes the model: every button, intent and landing from the library's lane
//! or a browser worker arrives here as a `Command`, and asks for its effects.
//!
//! The person's buttons and the Mind's intents are the same requests (`begin`), so a person and an
//! agent cannot get different answers about whether something worked. The library's commands run
//! on the lane (`yantrik_app::lane`); what they found comes back as `Landed`, and an intent is answered from
//! its own landing. Nothing here runs on a clock: the window changes only when a command moves it,
//! and at rest it draws nothing.
//!
//! - `drafts.rs`: the spec editor's unsaved text, in the framework's draft lane.
//! - `life.rs`: the close and the logout.
//! - this file: the window's own state, the requests and the landings.

use yantrik_app::effects::Effects;
use yantrik_app::for_mind;
use yantrik_app::Refused;

use crate::answers;
use crate::command::{Browse, Command, Landing, Reply, Request, Verb};
use crate::model::{Model, PART_EDITOR, PART_LISTS, PART_NOTICE, SLOW_AT_ONCE};
use crate::work;
use crate::Arcade;

pub mod drafts;
pub mod life;

pub use life::CLOSING;

type Fx = Effects<Arcade>;

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    // An answer is written only in the command that settles it (`answers.rs`); what an earlier
    // command wrote is never anyone else's.
    m.answer = serde_json::Value::Null;
    if (m.closing || m.ending) && starts_work(&command) {
        return Err(Refused::new("closing", CLOSING));
    }
    // In a copy drawn for a Mind the window is read-only: its buttons run as the person, with no
    // consent card, so the Mind goes through the intents, which are graded.
    if let Err(refused) = for_mind::refuse_if(m.for_mind, matches!(command, Command::Button(_) | Command::SpecEdited(_) | Command::PickCharacter(_))) {
        // Said in the window, and the fields drawn again from the model, so a refused edit does
        // not stay in the field as if it had been taken.
        m.notice = refused.sentence;
        fx.changed(PART_NOTICE);
        fx.changed(PART_EDITOR);
        return Ok(());
    }
    match command {
        Command::Button(id) => button(m, fx, &id),
        Command::Run(request) => return begin(m, fx, request, false),
        Command::SpecEdited(text) => {
            m.spec_text = text;
            drafts::typed(m, fx);
            // The editor already shows what was typed.
            fx.unchanged();
        }
        Command::GameEdited(text) => {
            m.game_name = text;
            fx.changed(PART_LISTS);
        }
        Command::PickGame(slug) => {
            m.game_name = slug;
            fx.changed(PART_LISTS);
            fx.changed(PART_EDITOR);
        }
        Command::PickCharacter(slug) => {
            // Clicking a character loads its spec into the editor: the fastest way to make the
            // next one is to look at the last one.
            m.asked += 1;
            let seq = m.asked;
            m.work += 1;
            let reply = m.lane.ask(move |library| work::character_spec(library, seq, &slug));
            fx.spawn_or(Command::LaneFailed { slow: false }, move |_| lands(reply.wait()));
            fx.unchanged();
        }
        Command::Landed(landing) => landed(m, *landing, fx),
        Command::LaneFailed { slow } => lane_failed(m, slow, fx),
        Command::Verdict { job, result } => verdict(m, job, result, fx),
        Command::VerdictLost { job } => {
            m.jobs.finish(job, Err("the worker did not answer".into()));
            m.notice = "A browser job ended without a verdict; `describe` lists it under jobs.".into();
            fx.changed(PART_LISTS);
            fx.changed(PART_NOTICE);
        }
        Command::Reload => {
            // Brought to the front: the library is read again, so the lists are the disk's word
            // and not the last command's (the CLI, or another copy, may have written). Nothing is
            // drawn if it has not moved.
            reload(m, fx);
            fx.unchanged();
        }
        Command::DraftNotKept { why } => {
            say(m, fx, format!("The spec you are typing could not be kept as a draft ({why}); save it as a game or a character so it is not lost."));
        }
        command @ (Command::Closing(_) | Command::Reopened | Command::Ending) => {
            let reopened = matches!(command, Command::Reopened);
            life::update(m, command, fx);
            // A second launch brought the window back: the library is read again too.
            if reopened {
                reload(m, fx);
            }
        }
    }
    Ok(())
}

/// A command that would start something on the lane or a worker.
fn starts_work(command: &Command) -> bool {
    matches!(command, Command::Button(_) | Command::Run(_) | Command::PickCharacter(_))
}

/// What a lane job comes back as.
fn lands(reply: Option<Landing>) -> Command {
    match reply {
        Some(landing) => Command::Landed(Box::new(landing)),
        None => Command::LaneFailed { slow: false },
    }
}

/// A header or workbench button: the same request an intent makes, from the window's own fields.
fn button(m: &mut Model, fx: &mut Fx, id: &str) {
    let Some(verb) = Verb::of_button(id) else {
        fx.unchanged();
        return;
    };
    let args = serde_json::json!({ "spec": m.spec_text, "game": m.game_name });
    match Request::from_args(verb, &args) {
        Ok(request) => {
            if let Err(refused) = begin(m, fx, request, verb.takes_spec()) {
                say(m, fx, refused.sentence);
            }
        }
        Err(sentence) => say(m, fx, sentence),
    }
}

/// Put `sentence` on the notice line.
fn say(m: &mut Model, fx: &mut Fx, sentence: String) {
    m.notice = sentence;
    fx.changed(PART_NOTICE);
}

/// Ask the lane for one command. Its answer is built from its own landing.
fn begin(m: &mut Model, fx: &mut Fx, request: Request, from_editor: bool) -> Result<(), Refused> {
    let slow = request.verb.is_slow();
    if slow {
        if m.jobs.running().len() + m.slow_asked >= SLOW_AT_ONCE {
            return Err(Refused::new(
                "busy",
                format!(
                    "Arcade is already running {SLOW_AT_ONCE} browser jobs; wait for one to land (`describe` lists them under jobs), then ask again."
                ),
            ));
        }
        m.slow_asked += 1;
    }
    m.asked += 1;
    let seq = m.asked;
    m.work += 1;
    let running: Vec<(String, String)> = m.jobs.running().iter().map(|j| (j.action.clone(), j.subject.clone())).collect();
    let reply = m.lane.ask(move |library| work::run(library, seq, request, from_editor, &running));
    fx.spawn_or(Command::LaneFailed { slow }, move |_| lands(reply.wait()));
    // Nothing on screen moves until the command lands.
    fx.unchanged();
    Ok(())
}

/// Reads the library again unless a read is already asked or the app is ending.
fn reload(m: &mut Model, fx: &mut Fx) {
    if !m.reload_asked && !m.closing && !m.ending {
        m.reload_asked = true;
        ask_reload(m, fx);
    }
}

fn ask_reload(m: &mut Model, fx: &mut Fx) {
    m.asked += 1;
    let seq = m.asked;
    m.work += 1;
    let reply = m.lane.ask(move |library| work::reload(library, seq));
    fx.spawn_or(Command::LaneFailed { slow: false }, move |_| lands(reply.wait()));
}

/// A command has landed: the window shows its outcome and the caller is told it.
fn landed(m: &mut Model, landing: Landing, fx: &mut Fx) {
    m.work = m.work.saturating_sub(1);
    let Landing { seq, request, from_editor, result, listing } = landing;
    if request.as_ref().is_some_and(|r| r.verb.is_slow()) {
        m.slow_asked = m.slow_asked.saturating_sub(1);
    }
    // The lane answers in order, but the pool delivers in any: an older listing never replaces a
    // newer one.
    if seq > m.applied {
        m.applied = seq;
        if m.listing != listing {
            m.listing = listing;
            fx.changed(PART_LISTS);
        }
    }
    match result {
        Ok(Reply::Reloaded) => m.reload_asked = false,
        Ok(Reply::Spec(text)) => {
            m.spec_text = text.clone();
            m.kept_text = text;
            m.notice.clear();
            drafts::typed(m, fx);
            fx.changed(PART_EDITOR);
            fx.changed(PART_NOTICE);
        }
        Ok(Reply::Done { value, select, slow }) => {
            if let Some(game) = select {
                m.game_name = game;
                fx.changed(PART_LISTS);
                fx.changed(PART_EDITOR);
            }
            if let Some(request) = request.as_ref().filter(|r| from_editor && r.verb.takes_spec()) {
                kept(m, fx, &request.spec);
            }
            // A browser job asked before the window closed is not started after it: nothing is left
            // to wait for it, and its browser would outlive the app.
            if slow.is_some() && (m.closing || m.ending) {
                m.notice = CLOSING.into();
                fx.changed(PART_NOTICE);
                answers::fail(fx, CLOSING.into());
                life::drained(m, fx);
                return;
            }
            let value = match (slow, request) {
                (Some(browse), Some(request)) => {
                    let job = start_worker(m, fx, browse);
                    work::started(request.verb, &request.game, job)
                }
                _ => value,
            };
            m.notice = work::summary_of(&value);
            fx.changed(PART_NOTICE);
            fx.changed(PART_LISTS);
            answers::settle(m, fx, value);
        }
        Err(sentence) => {
            m.notice = sentence.clone();
            fx.changed(PART_NOTICE);
            answers::fail(fx, sentence);
        }
    }
    life::drained(m, fx);
}

/// The library holds `text` now: it is no longer the editor's to lose.
fn kept(m: &mut Model, fx: &mut Fx, text: &str) {
    m.kept_text = text.to_string();
    drafts::typed(m, fx);
}

/// A browser job, on a worker of its own, with its record started.
fn start_worker(m: &mut Model, fx: &mut Fx, browse: Browse) -> u64 {
    let job = m.jobs.start(browse.verb.name(), &browse.slug);
    let (slow, library) = (m.slow.clone(), m.library.clone());
    fx.spawn_or(Command::VerdictLost { job }, move |_| Command::Verdict {
        job,
        result: slow.run(browse.verb, &library, &browse.slug, &browse.html),
    });
    job
}

/// A browser job's verdict, onto the window: banner, status line, and the library read again
/// because the verdict is in its `verify.json`.
fn verdict(m: &mut Model, job: u64, result: Result<String, String>, fx: &mut Fx) {
    m.notice = match &result {
        Ok(sentence) | Err(sentence) => sentence.clone(),
    };
    m.jobs.finish(job, result);
    fx.changed(PART_NOTICE);
    fx.changed(PART_LISTS);
    if !m.closing && !m.ending {
        ask_reload(m, fx);
    }
}

/// A command on the lane ended without its answer: a panic, or the pool at its bound. The
/// invocation it ran under is told by the framework; the window is told here.
fn lane_failed(m: &mut Model, slow: bool, fx: &mut Fx) {
    m.work = m.work.saturating_sub(1);
    // A look at the library that failed may be asked again.
    m.reload_asked = false;
    if slow {
        m.slow_asked = m.slow_asked.saturating_sub(1);
    }
    m.notice = "Arcade did not answer. It may still have run; look at the lists before trying again.".into();
    fx.changed(PART_NOTICE);
    life::drained(m, fx);
}
