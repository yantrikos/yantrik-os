//! What mpv says on its own, through its reader thread: a property that changed, a file that
//! loaded or ended, its own death. Each comes with the number of the player that said it; a
//! player that has since been replaced or stopped is not heard.

use crate::answers::refuse;
use crate::command::{Command, Work};
use crate::model::{Model, PlayerState, NOTHING, PART_LISTS, PART_TRANSPORT};
use crate::mpv::{Event, Observed, REDIRECTED};
use crate::player::ops;

use super::{job, landed::say, playback, Fx};

/// What the window says when the player goes away.
pub const PLAYER_GONE: &str = "The player (mpv) stopped. Press play to start it again.";

/// Whether what player `player` says is heard: it is the one in hand, or a newer one, which
/// becomes the one in hand. One stopped by a close or a logout is never heard again.
pub fn hear(m: &mut Model, player: u64) -> bool {
    if player <= m.deaf_to || player < m.hearing {
        return false;
    }
    if player > m.hearing {
        m.hearing = player;
        m.observed = Observed::default();
    }
    m.state = PlayerState::Running;
    true
}

pub fn update(m: &mut Model, player: u64, event: Event, fx: &mut Fx) {
    // Closing or ending: the player is being stopped, and nothing it says starts anything.
    if m.closing || m.ending || !hear(m, player) {
        fx.changed(NOTHING);
        return;
    }
    m.observed.apply(&event);
    match event {
        Event::Property(name, value) => {
            if name == "volume" {
                if let Some(v) = value.as_f64() {
                    m.volume = v;
                }
            }
            if name == "mute" {
                m.muted = value.as_bool().unwrap_or(false);
            }
            fx.changed(PART_TRANSPORT);
        }
        // Everything is drawn: the row of the track now playing is marked in both lists.
        Event::FileLoaded => {}
        Event::EndFile { reason, error } => match reason.as_str() {
            // Auto-advance: the end of a track plays the next one.
            "eof" => {
                match m.queue.next().cloned() {
                    Some(t) => playback::load(m, fx, t),
                    None => say(m, fx, "End of the queue."),
                }
                fx.changed(PART_LISTS);
                fx.changed(PART_TRANSPORT);
            }
            "error" => {
                let title = playback::title_now(m);
                let why = error.unwrap_or_else(|| "mpv could not read it".into());
                refuse(m, fx, "not_played", format!("Could not play \u{201c}{title}\u{201d}: {why}."));
                fx.changed(PART_TRANSPORT);
            }
            // The "sound file" was a playlist and mpv is moving on to what it lists. mpv is told
            // not to (`--access-references=no`); if it does anyway, it is stopped and the file is
            // refused rather than followed.
            "redirect" => {
                job(m, fx, Work::Halt, move |p, t| {
                    ops::halt(p, t, player);
                    Command::Tidied
                });
                let title = playback::title_now(m);
                refuse(m, fx, "not_played", format!("Could not play \u{201c}{title}\u{201d}: {REDIRECTED}."));
                fx.changed(PART_TRANSPORT);
            }
            _ => fx.changed(PART_TRANSPORT),
        },
        Event::Gone => {
            job(m, fx, Work::Halt, move |p, t| {
                ops::bury(p, t, player);
                Command::Tidied
            });
            m.state = PlayerState::Stopped;
            refuse(m, fx, "player_stopped", PLAYER_GONE);
            fx.changed(PART_TRANSPORT);
        }
    }
}
