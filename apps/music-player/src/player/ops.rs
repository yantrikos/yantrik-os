//! What the jobs do with the player, each in its turn. Every function here blocks on mpv (one
//! IPC round trip each, two seconds at most) and runs on a job thread, never the UI thread.
//!
//! The sentences are the app's, as it always said them. A job that speaks for an intent also
//! asks mpv for its report afterwards, so the answer is what mpv says, never the app's opinion;
//! a job for a button skips that and the wait.

use std::time::Duration;

use serde_json::{json, Value};

use super::{Player, Start, Ticket};
use crate::library::{self, Track};
use crate::mpv::{Engine, Moment, Observed};
use crate::seen;

/// How long an intent waits for mpv to say a file it was handed is playing.
pub const LOAD_WAIT: Duration = Duration::from_secs(4);

/// How long `play` waits for mpv to say it resumed.
const RESUME_WAIT: Duration = Duration::from_secs(1);

/// What mpv reports now, asked fresh over the socket rather than read from the last event.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub playing: bool,
    pub paused: bool,
    pub path: Option<String>,
    /// Whether an agent may be told `path` (`seen::for_agents`, which reads the disk).
    pub path_for_agents: bool,
    pub position: Value,
    pub duration: Value,
    pub volume: Value,
    pub muted: bool,
}

impl Report {
    /// The report as mpv's own word had it at one moment: when the file a call asked for loaded.
    fn of(m: &Moment) -> Report {
        Report {
            playing: m.playing,
            paused: m.paused,
            path_for_agents: m.path.as_deref().is_some_and(seen::for_agents),
            path: m.path.clone(),
            position: json!(m.time_pos),
            duration: json!(m.duration),
            volume: json!(m.volume),
            muted: m.muted,
        }
    }
}

pub fn report(e: &Engine) -> Report {
    let get = |p: &str| e.client.get(p).unwrap_or(Value::Null);
    let paused = get("pause").as_bool().unwrap_or(false);
    let idle = get("idle-active").as_bool().unwrap_or(true);
    let path = get("path").as_str().map(str::to_string);
    Report {
        playing: !idle && path.is_some() && !paused,
        paused,
        path_for_agents: path.as_deref().is_some_and(seen::for_agents),
        path,
        position: get("time-pos"),
        duration: get("duration"),
        volume: get("volume"),
        muted: get("mute").as_bool().unwrap_or(false),
    }
}

/// A file loaded in the player, by mpv's last report.
fn loaded(o: &Observed) -> bool {
    o.path.is_some() && !o.idle
}

/// The running player, when a file is loaded in it; the sentence naming `what` otherwise.
fn playing_engine(p: &Player, what: &str) -> Result<std::sync::Arc<Engine>, String> {
    match p.live() {
        Some((_, e)) if loaded(&e.observed.snapshot()) => Ok(e),
        _ => Err(format!("Nothing is playing, so there is nothing to {what}.")),
    }
}

/// What came of handing a track to the player.
#[derive(Debug, Clone, PartialEq)]
pub enum Loaded {
    /// mpv could not be started; the sentence says why.
    NoPlayer(String),
    /// The player would not take the file. `gone` when that is because it died taking it.
    Refused { why: String, gone: bool },
    /// mpv took it. `started` when it was started for this; `for_agents`, whether an agent may
    /// be told the track's path. `answer` is what an intent is answered with: mpv's report once
    /// it is playing, or what it said instead.
    Sent { player: u64, started: bool, for_agents: bool, answer: Option<Result<Played, String>> },
}

/// mpv's word that the track an intent asked for is playing.
#[derive(Debug, Clone, PartialEq)]
pub struct Played {
    pub report: Report,
    /// The path the player had moved on to by the time it was asked for its report (another
    /// call, or the person, played something else meanwhile), and whether an agent may be told
    /// it. The report is then mpv's word at the moment it said this track was playing.
    pub moved_to: Option<(String, bool)>,
}

/// Hand `track` to the player, starting one if there is none, and unpause. `answer` asks for
/// the wait for mpv to say it is playing; `agent`, whether the answer is an agent's.
pub fn load(p: &Player, ticket: Ticket, track: &Track, start: &Start, answer: bool, agent: bool) -> Loaded {
    let turn = ticket.wait();
    let (player, e, started) = match p.live_or_start(start) {
        Ok(found) => found,
        Err(why) => return Loaded::NoPlayer(why),
    };
    // Counted from the player that is playing it: one started for this load starts at 0.
    let before = e.observed.snapshot();
    if let Err(why) = e.client.load(&track.path) {
        let why = format!("Could not play \u{201c}{}\u{201d}: {why}.", track.title);
        return Loaded::Refused { why, gone: !e.is_alive() };
    }
    let mine = p.handed(&e);
    let _ = e.client.set("pause", json!(false));
    drop(turn);
    let for_agents = seen::for_agents(&track.path);
    let answer = answer.then(|| wait_loaded(p, &e, track, &before, mine, agent));
    Loaded::Sent { player, started, for_agents, answer }
}

/// Wait for mpv to have loaded `track`, and answer with what it reported; or say what it said
/// instead. Out of turn: a pause pressed meanwhile is not held up by it. The wait ends early
/// when another file is handed over (`mine` is no longer the last), so however many calls play
/// at once, at most one wait holds a job thread and the player's commands always find one.
fn wait_loaded(p: &Player, e: &Engine, track: &Track, before: &Observed, mine: u64, agent: bool) -> Result<Played, String> {
    let played = |o: &Observed| o.loaded_since(before.loads, &track.path).is_some();
    let superseded = || p.last_handed() != mine;
    let (_, o) = e.observed.wait_for(LOAD_WAIT, |o| played(o) || o.gone || o.errors > before.errors || superseded());
    // mpv said this file loaded, whatever it has done since: the history is asked, never only
    // the latest state, so a file handed over straight after cannot hide it.
    if let Some(at) = o.loaded_since(before.loads, &track.path) {
        let now = report(e);
        return Ok(match now.path {
            Some(ref p) if *p != track.path => {
                let moved_to = Some((p.clone(), now.path_for_agents));
                Played { report: Report::of(at), moved_to }
            }
            _ => Played { report: now, moved_to: None },
        });
    }
    let why = match (&o.last_error, &o.path) {
        (Some(error), _) if o.errors > before.errors => error.clone(),
        _ if o.gone => "the player (mpv) stopped".into(),
        _ if superseded() && o.loads == before.loads => {
            "another file was handed to it before it said this one was playing".into()
        }
        // mpv loaded something, but not this file: say what it is playing instead.
        (_, Some(other)) if o.loads > before.loads => {
            format!("mpv is playing {} instead", library::plain(&seen::shown(other, agent, seen::for_agents(other))))
        }
        _ => "mpv did not report the file as loaded".into(),
    };
    Err(format!("mpv did not play \u{201c}{}\u{201d}: {why}.", track.title))
}

/// What came of `play` with something loaded.
#[derive(Debug, Clone, PartialEq)]
pub enum Resumed {
    /// Nothing is loaded after all (it ended meanwhile): `play` goes on to the queue.
    NotLoaded,
    Failed(String),
    /// Unpaused; with mpv's report, for an intent.
    Done(Option<Report>),
}

/// Unpause what is loaded. For an intent, the turn is held through the wait for mpv to say it
/// resumed (a second at most), so a pause pressed meanwhile cannot become this call's answer.
pub fn resume(p: &Player, ticket: Ticket, answer: bool) -> Resumed {
    let _turn = ticket.wait();
    let Some((_, e)) = p.live().filter(|(_, e)| loaded(&e.observed.snapshot())) else {
        return Resumed::NotLoaded;
    };
    if let Err(why) = e.client.set("pause", json!(false)) {
        return Resumed::Failed(format!("Could not resume: {why}."));
    }
    if !answer {
        return Resumed::Done(None);
    }
    let (ok, _) = e.observed.wait_for(RESUME_WAIT, |o| !o.paused);
    let r = report(&e);
    if !ok || r.paused {
        return Resumed::Failed("mpv was asked to resume and still reports paused.".into());
    }
    Resumed::Done(Some(r))
}

pub fn pause(p: &Player, ticket: Ticket, answer: bool) -> Result<Option<Report>, String> {
    let _turn = ticket.wait();
    let e = playing_engine(p, "pause")?;
    e.client.set("pause", json!(true)).map_err(|why| format!("Could not pause: {why}."))?;
    if !answer {
        return Ok(None);
    }
    let r = report(&e);
    if !r.paused {
        return Err("mpv was asked to pause and does not report paused.".into());
    }
    Ok(Some(r))
}

/// What came of `previous`'s first step.
#[derive(Debug, Clone, PartialEq)]
pub enum Back {
    /// More than three seconds in: back to the start, with mpv's report.
    Restarted(Option<Report>),
    /// Not that far in, or nothing loaded: the queue steps back.
    Before,
    Failed(String),
}

/// Back to the start of the track if it has played for more than three seconds, by mpv's own
/// position (not the once-a-second one the window draws).
pub fn previous(p: &Player, ticket: Ticket, answer: bool) -> Back {
    let _turn = ticket.wait();
    let Some((_, e)) = p.live() else { return Back::Before };
    let o = e.observed.snapshot();
    if o.path.is_none() || o.time_pos.unwrap_or(0.0) <= 3.0 {
        return Back::Before;
    }
    match e.client.command(&[json!("seek"), json!(0), json!("absolute")]) {
        Ok(_) => Back::Restarted(answer.then(|| report(&e))),
        Err(why) => Back::Failed(why),
    }
}

/// `previous` at the top of the queue: the start of what is playing.
pub fn rewind(p: &Player, ticket: Ticket, answer: bool) -> Result<Option<Report>, String> {
    let _turn = ticket.wait();
    let e = playing_engine(p, "go back to")?;
    e.client.command(&[json!("seek"), json!(0), json!("absolute")])?;
    Ok(answer.then(|| report(&e)))
}

pub fn seek(p: &Player, ticket: Ticket, seconds: f64, answer: bool) -> Result<Option<Report>, String> {
    let _turn = ticket.wait();
    let e = playing_engine(p, "seek in")?;
    e.client
        .command(&[json!("seek"), json!(seconds.max(0.0)), json!("absolute")])
        .map_err(|why| format!("Could not seek: {why}."))?;
    Ok(answer.then(|| report(&e)))
}

/// What came of a volume change.
#[derive(Debug, Clone, PartialEq)]
pub enum VolumeSet {
    /// No player: the volume is kept, and mpv starts at it.
    NotRunning,
    Set(Option<Report>),
}

pub fn volume(p: &Player, ticket: Ticket, percent: f64, answer: bool) -> Result<VolumeSet, String> {
    let _turn = ticket.wait();
    let Some((_, e)) = p.live() else { return Ok(VolumeSet::NotRunning) };
    e.client.set("volume", json!(percent)).map_err(|why| format!("Could not set the volume: {why}."))?;
    Ok(VolumeSet::Set(answer.then(|| report(&e))))
}

/// Mute or unmute the running player. `Ok(false)` when there is none: the model keeps it.
pub fn mute(p: &Player, ticket: Ticket, muted: bool) -> Result<bool, String> {
    let _turn = ticket.wait();
    let Some((_, e)) = p.live() else { return Ok(false) };
    e.client.set("mute", json!(muted)).map_err(|why| format!("Could not mute: {why}."))?;
    Ok(true)
}

pub fn stop(p: &Player, ticket: Ticket) -> Result<(), String> {
    let _turn = ticket.wait();
    let e = playing_engine(p, "stop")?;
    e.client.command(&[json!("stop")]).map(|_| ()).map_err(|why| format!("Could not stop: {why}."))
}

/// The "sound file" was a playlist and mpv moved on to what it lists: stopped, if player
/// `number` is still the one running.
pub fn halt(p: &Player, ticket: Ticket, number: u64) {
    let _turn = ticket.wait();
    if let Some((n, e)) = p.live() {
        if n == number {
            let _ = e.client.command(&[json!("stop")]);
        }
    }
}

/// Player `number` went away on its own: it is reaped now, so no zombie waits for the next play.
pub fn bury(p: &Player, ticket: Ticket, number: u64) {
    let _turn = ticket.wait();
    p.bury(number);
}

/// Stops and reaps the player, in its turn: after every command sent before it.
pub fn retire(p: &Player, ticket: Ticket) -> u64 {
    let _turn = ticket.wait();
    p.retire()
}
