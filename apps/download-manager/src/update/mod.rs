//! The only code that changes the model: every button, intent, timer tick and landing from the
//! engine's desk arrives here as a `Command`, and asks for its effects.
//!
//! The person's buttons and the Mind's intents are the same commands, so a person and an agent
//! cannot get different answers about whether something worked. The engine's calls run on the
//! desk (`calls.rs`); what they found comes back as `Landed`. The workers move bytes on their own
//! threads and bump the engine's revision; a look at it (`sync`) every 250 ms while anything is
//! running draws the list when that revision moved, and nothing at all otherwise.
//!
//! - `calls.rs`: the calls to the engine, and their answers.
//! - `life.rs`: the close and the logout.
//! - this file: the window's own state, the landings and the look at the engine.

use std::time::Duration;

use yantrik_app::effects::Effects;
use yantrik_app::intents::Origin;
use yantrik_app::timers::TimerId;
use yantrik_app::Refused;

use crate::answers;
use crate::command::{Call, Command, Landed};
use crate::model::{Model, PART_AI, PART_BANNER, PART_FORM, PART_LIST};
use crate::Downloads;

pub mod calls;
pub mod life;

pub use life::CLOSING;

type Fx = Effects<Downloads>;

/// How often the engine is looked at while a download is running.
pub const TICK: Duration = Duration::from_millis(250);
const LOOK: TimerId = TimerId("downloads.look");

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    // An answer is written only in the command that settles it (`answers.rs`); what an earlier
    // command wrote is never anyone else's.
    m.answer = serde_json::Value::Null;
    m.form_done = false;
    if (m.closing || m.ending) && starts_work(&command) {
        return Err(Refused::new("closing", CLOSING));
    }
    match command {
        Command::Add { url, sha256, save_dir } => {
            calls::start(m, fx, Call::Add, move |e, agent| calls::add(e, agent, &url, &sha256, &save_dir));
        }
        Command::Pause(id) => calls::start(m, fx, Call::Pause, move |e, _| calls::on_row(e, id, |e, id| e.pause(id))),
        Command::Resume(id) => calls::start(m, fx, Call::Resume, move |e, _| calls::on_row(e, id, |e, id| e.resume(id))),
        Command::Cancel(id) => calls::start(m, fx, Call::Cancel, move |e, agent| calls::cancel(e, agent, id)),
        Command::Retry(id) => calls::start(m, fx, Call::Retry, move |e, agent| calls::retry(e, agent, id)),
        Command::Verify { id, sha256 } => {
            calls::start(m, fx, Call::Verify, move |e, _| calls::verify(e, id, sha256.as_deref()));
        }
        Command::OpenFolder(id) => {
            calls::start(m, fx, Call::OpenFolder, move |e, _| calls::on_row(e, id, calls::open_folder));
        }
        Command::PauseAll => calls::start(m, fx, Call::PauseAll, |e, _| {
            e.pause_all();
            Ok(serde_json::json!({ "paused": e.totals().paused }))
        }),
        Command::ResumeAll => calls::start(m, fx, Call::ResumeAll, |e, _| {
            e.resume_all();
            Ok(serde_json::json!({ "active": e.totals().active }))
        }),
        Command::ClearCompleted => calls::start(m, fx, Call::ClearCompleted, |e, _| {
            Ok(serde_json::json!({ "cleared": e.clear_completed() }))
        }),

        Command::Landed(landed) => landed_call(m, *landed, fx),
        Command::WorkFailed(call) => work_failed(m, call, fx),
        Command::Tick => {
            if !sync(m, fx) {
                // Nothing the workers did is on screen: a window at rest draws nothing.
                fx.unchanged();
            }
        }

        Command::Filter(filter) => {
            m.filter = filter;
            fx.changed(PART_LIST);
        }
        Command::Sort(sort) => {
            m.sort = sort;
            fx.changed(PART_LIST);
        }
        Command::Search(query) => {
            m.search = query;
            fx.changed(PART_LIST);
        }
        Command::ToggleSelect(id) => {
            let now_selected = m.engine.get(id).map(|d| !d.selected).unwrap_or(false);
            m.engine.set_selected(id, now_selected);
            fx.changed(PART_LIST);
        }
        Command::SelectAll(selected) => {
            m.engine.select_all(selected);
            m.all_selected = selected;
            fx.changed(PART_LIST);
        }
        Command::AiExplain => {
            // A reason the Mind cannot answer is said where the answer would have gone; otherwise
            // the same path the rail's suggestion takes, so the two cannot come to differ.
            fx.changed(PART_AI);
            if let Some(hint) = m.rail.reach().hint() {
                m.ai_hint = Some(hint.to_string());
                return Ok(());
            }
            m.ai_hint = None;
            return yantrik_app::rail::update::<Downloads>(
                m,
                yantrik_app::rail::RailCommand::Activate(crate::rail::EXPLAIN.into()),
                fx,
            );
        }
        Command::AiDismiss => {
            m.ai_hint = None;
            fx.changed(PART_AI);
        }
        Command::Rail(rail) => return yantrik_app::rail::update::<Downloads>(m, rail, fx),

        command @ (Command::Settled | Command::Closing(_) | Command::Reopened | Command::Ending) => {
            life::update(m, command, fx);
        }
    }
    Ok(())
}

/// A command that would start a transfer or a check.
fn starts_work(command: &Command) -> bool {
    matches!(
        command,
        Command::Add { .. } | Command::Resume(_) | Command::Retry(_) | Command::Verify { .. } | Command::ResumeAll
    )
}

/// A call has landed: the window shows its outcome and the caller is told it.
///
/// The banner takes the call's refusal, or is cleared by its success, and the engine's own notice
/// is given up either way: the person has had it on screen and has now done something else, and
/// leaving it to be raised again by the next look would make a banner about the last restart
/// impossible to get past. `describe` keeps reporting it.
fn landed_call(m: &mut Model, landed: Landed, fx: &mut Fx) {
    m.work = m.work.saturating_sub(1);
    let Landed { call, answer } = landed;
    m.engine.acknowledge_notice();
    m.banner.clear();
    m.banner_info = false;
    match answer {
        Ok(value) => {
            // The dialog closes only on the person's own add; a rejected URL stays on screen to be
            // fixed, and an agent's add is none of the dialog's business.
            if call == Call::Add && matches!(fx.origin(), Origin::Local) {
                m.form_done = true;
                fx.changed(PART_FORM);
            }
            m.refresh_banner();
            sync(m, fx);
            fx.changed(PART_BANNER);
            answers::settle(m, fx, value);
        }
        Err(reason) => {
            m.banner = reason.clone();
            sync(m, fx);
            fx.changed(PART_BANNER);
            answers::fail(fx, reason);
        }
    }
}

/// A call ended without its answer: a panic, or the pool at its bound. The invocation it ran under
/// is told by the framework; the window is told here.
fn work_failed(m: &mut Model, call: Call, fx: &mut Fx) {
    m.work = m.work.saturating_sub(1);
    m.banner = format!("The download manager did not answer ({call:?}). It may still have run; look at the list before trying again.");
    m.banner_info = false;
    sync(m, fx);
    fx.changed(PART_BANNER);
}

/// Look at the engine: tell the person what the workers have to say, note whether the list moved,
/// and keep the clock running exactly while a download is. Whether the list or the banner changed.
///
/// Whether anything is running is read before the revision: a worker that finishes between the
/// two has bumped the revision already, so a clock stopped on "nothing running" has not missed it.
pub fn sync(m: &mut Model, fx: &mut Fx) -> bool {
    let working = m.engine.working();
    let revision = m.engine.revision();
    for notification in m.engine.announcements() {
        fx.notify(notification);
    }
    let mut changed = false;
    if revision != m.seen {
        m.seen = revision;
        fx.changed(PART_LIST);
        changed = true;
        if m.refresh_banner() {
            fx.changed(PART_BANNER);
        }
    }
    if working && !m.ticking {
        m.ticking = true;
        fx.every(LOOK, TICK, Command::Tick);
    } else if !working && m.ticking {
        m.ticking = false;
        fx.stop_timer(LOOK);
    }
    changed
}
