//! What the player's jobs found, each applied in the command that carries it, and each intent
//! answered there from its own job's result.

use serde_json::json;

use crate::answers::{self, refuse};
use crate::command::Command;
use crate::model::{Model, PlayerState, NOTHING, PART_TRANSPORT};
use crate::player::ops::{Back, Loaded, Resumed, VolumeSet};

use super::{heard, playback, Fx};

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) {
    match command {
        Command::Loaded { track, loaded, stale } => match *loaded {
            Loaded::NoPlayer(why) => {
                m.state = PlayerState::Unavailable(why.clone());
                refuse(m, fx, "no_player", why);
            }
            // The player died taking the file: the window says what its death always said, the
            // caller what became of its file.
            Loaded::Refused { why, gone: true } => {
                m.state = PlayerState::Stopped;
                refuse(m, fx, "not_played", why);
                m.notice = heard::PLAYER_GONE.into();
            }
            Loaded::Refused { why, gone: false } => refuse(m, fx, "not_played", why),
            Loaded::Sent { player, started, for_agents, answer } => {
                m.playing_for_agents = Some((track.path.clone(), for_agents));
                // The complaint about a player that could not start, or went away, is over.
                if started && heard::hear(m, player) && stale.is_some_and(|s| s == m.notice) {
                    m.notice.clear();
                }
                m.status = format!("Playing \u{201c}{}\u{201d}", track.title);
                m.agent_status = None;
                match answer {
                    Some(Ok(played)) => answers::playing(m, fx, &played, &track.title),
                    // What mpv said instead is the caller's; the window hears it from mpv itself.
                    Some(Err(why)) => answers::fail(fx, "not_played", why),
                    None => {}
                }
            }
        },
        Command::Resumed(resumed) => match *resumed {
            Resumed::NotLoaded => playback::play_from_queue(m, fx),
            Resumed::Failed(why) => refuse(m, fx, "not_resumed", why),
            Resumed::Done(report) => reported(m, fx, report),
        },
        Command::Paused(result) => landed(m, fx, "not_paused", result),
        Command::WentBack(back) => match *back {
            Back::Restarted(report) => reported(m, fx, report),
            Back::Before => playback::step_back(m, fx),
            Back::Failed(why) => refuse(m, fx, "not_moved", why),
        },
        Command::Rewound(result) => landed(m, fx, "not_moved", result),
        Command::Sought(result) => landed(m, fx, "not_moved", result),
        Command::VolumeSet { result, .. } if !answers::owed(fx) => {
            // The person's: the volume asked for since is sent now, so the last one wins.
            m.volume_sending = false;
            if let Err(why) = result {
                refuse(m, fx, "not_set", why);
            }
            match m.volume_next.take() {
                Some(next) => playback::send_volume(m, fx, next),
                None => fx.changed(NOTHING),
            }
        }
        Command::VolumeSet { percent, result } => match result {
            // Nothing to ask: the volume is kept and mpv starts with it.
            Ok(VolumeSet::NotRunning) => {
                let answer = json!({
                    "volume": percent,
                    "player": "not running; mpv starts at this volume",
                    "reported_by": "music",
                });
                answers::settle(m, fx, answer);
                fx.changed(PART_TRANSPORT);
            }
            Ok(VolumeSet::Set(report)) => reported(m, fx, report),
            Err(why) => refuse(m, fx, "not_set", why),
        },
        Command::Muted { muted, result } => match result {
            // mpv reports the change itself, and the window draws that.
            Ok(true) => fx.changed(NOTHING),
            Ok(false) => {
                m.muted = muted;
                fx.changed(PART_TRANSPORT);
            }
            Err(why) => refuse(m, fx, "not_muted", why),
        },
        Command::Stopped(Ok(())) => say(m, fx, "Stopped."),
        Command::Stopped(Err(why)) => refuse(m, fx, "not_stopped", why),
        Command::Tidied => fx.changed(NOTHING),
        Command::Checked(Ok(track)) => {
            // The file is the whole queue.
            m.queue.start(vec![track.clone()], 0);
            playback::load(m, fx, track);
        }
        Command::Checked(Err(why)) => refuse(m, fx, "not_a_sound_file", why),
        _ => {}
    }
}

/// A job that answers with mpv's report when an intent asked for one.
fn landed(m: &mut Model, fx: &mut Fx, kind: &'static str, result: Result<Option<crate::player::ops::Report>, String>) {
    match result {
        Ok(report) => reported(m, fx, report),
        Err(why) => refuse(m, fx, kind, why),
    }
}

fn reported(m: &mut Model, fx: &mut Fx, report: Option<crate::player::ops::Report>) {
    match report {
        Some(r) => answers::reported(m, fx, &r),
        // A button's: mpv's own reports redraw the transport as they come.
        None => fx.changed(NOTHING),
    }
}

/// The status line says what happened.
pub fn say(m: &mut Model, fx: &mut Fx, status: impl Into<String>) {
    m.status = status.into();
    m.agent_status = None;
    fx.changed(crate::model::PART_STATUS);
}
