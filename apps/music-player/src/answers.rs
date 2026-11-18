//! The answer each intent is owed, settled in the command where its own work lands, and the
//! refusals, said twice.
//!
//! An intent's reply is read from the model when it settles, so each one settles in the command
//! that carries its own result: `m.answer` is written and `fx.settle` asked for in that one
//! command, and nothing can land between the two. The answer is built from that result alone
//! (the player's report its own job asked for, the scan it started), never from whatever the
//! window shows when it lands. A command with no invocation (a button, the player's own word)
//! never writes `m.answer`.
//!
//! A failure is said twice, as it always was: to the caller, and on screen in `notice`, which is
//! also `describe.notice`. So a refused intent is not a refused command: the command records the
//! sentence and settles its invocation with the refusal.

use serde_json::{json, Value};
use yantrik_app::effects::Effects;
use yantrik_app::intents::Origin;
use yantrik_app::Refused;

use crate::library;
use crate::model::{Model, PART_STATUS};
use crate::player::ops::{Played, Report};
use crate::seen;
use crate::Music;

type Fx = Effects<Music>;

/// Whether the command being applied is an agent's: it came through the surface with an agent's
/// token. Carried with the invocation through every job it started.
pub fn agent(fx: &Fx) -> bool {
    matches!(fx.origin(), Origin::Surface { agent: true })
}

/// Whether the command being applied owes a surface call its answer.
pub fn owed(fx: &Fx) -> bool {
    fx.invocation().is_some()
}

/// Settle the invocation this command runs under with `answer`, if it has one.
pub fn settle(m: &mut Model, fx: &mut Fx, answer: Value) {
    if owed(fx) {
        m.answer = answer;
        fx.settle(Ok(()));
    }
}

/// Say why something could not be done, on the notice line, and to the call it answers.
pub fn refuse(m: &mut Model, fx: &mut Fx, kind: &'static str, sentence: impl Into<String>) {
    let sentence = sentence.into();
    m.notice = sentence.clone();
    fx.changed(PART_STATUS);
    if owed(fx) {
        fx.settle(Err(Refused::new(kind, sentence)));
    }
}

/// Refuse the call this command answers, and say nothing on screen: what mpv did instead of
/// playing a file is the caller's answer, and the window hears it from mpv itself.
pub fn fail(fx: &mut Fx, kind: &'static str, sentence: String) {
    if owed(fx) {
        fx.settle(Err(Refused::new(kind, sentence)));
    }
}

/// mpv's report, as this caller may see it.
pub fn report(r: &Report, agent: bool) -> Value {
    json!({
        "playing": r.playing,
        "paused": r.paused,
        "path": r.path.as_deref().map(|p| seen::shown(p, agent, r.path_for_agents)),
        "position": r.position,
        "duration": r.duration,
        "volume": r.volume,
        "muted": r.muted,
        "reported_by": "mpv",
    })
}

/// Keep the verdict a report carries on the file the player is on, for `describe`.
pub fn remember(m: &mut Model, r: &Report) {
    if let Some(path) = &r.path {
        m.playing_for_agents = Some((path.clone(), r.path_for_agents));
    }
}

/// Settle with mpv's report, kept for `describe` too.
pub fn reported(m: &mut Model, fx: &mut Fx, r: &Report) {
    remember(m, r);
    let answer = report(r, agent(fx));
    settle(m, fx, answer);
}

/// Settle with mpv's report and the title of the track it is playing. When the player had moved
/// on by the time it was asked (another call, or the person, played something else meanwhile),
/// the report is mpv's word as this track started, and the answer says where the player is now.
pub fn playing(m: &mut Model, fx: &mut Fx, played: &Played, title: &str) {
    let mut answer = report(&played.report, agent(fx));
    answer["track"] = json!(title);
    match &played.moved_to {
        None => remember(m, &played.report),
        Some((path, _)) => {
            let now = library::title_of(path);
            answer["player_moved_since"] = json!(format!("The player has since moved on to \u{201c}{now}\u{201d}."));
        }
    }
    settle(m, fx, answer);
}
