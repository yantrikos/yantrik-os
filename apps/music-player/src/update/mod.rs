//! The only code that changes the model: every button, intent, launch, job result and word from
//! the player lands here as a `Command`, and asks for its effects.
//!
//! The person's buttons and the Mind's intents are the same commands where they do the same
//! thing (next, previous, rescan) and share every step where they differ only in what is
//! answered. Everything that speaks to mpv or reads the disk runs in a job; what it found comes
//! back here as a command.
//!
//! - `playback.rs`: play, pause, the queue, seeking, the volume, `play_file`.
//! - `landed.rs`: what the player's jobs found.
//! - `heard.rs`: what mpv says on its own: a file loaded or ended, the position, its death.
//! - `library.rs`: the scan and the folder list.
//! - this file: the app's life — the close, the logout, the settings file.

use std::time::Duration;

use serde_json::Value;
use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use crate::command::{Command, Work};
use crate::kept::Kept;
use crate::model::{Model, PlayerState, NOTHING, PART_LISTS, PART_STATUS, PART_TRANSPORT};
use crate::mpv::Observed;
use crate::player::{ops, Player, Ticket};
use crate::Music;

mod heard;
mod landed;
mod library;
mod playback;

/// What a test checks a `rescan` during a scan is told.
#[cfg(test)]
pub use library::SCANNING;

type Fx = Effects<Music>;

/// How long a close waits for the player to be stopped before the framework ends the app anyway
/// (`fx.linger`'s backstop, which logs that it did). Stopping is a `quit` and half a second, and
/// each command ahead of it gives up within two, so this is reached only by a wedged player:
/// then the process ends and the kernel stops mpv (`PR_SET_PDEATHSIG`).
pub const CLOSE_BACKSTOP: Duration = Duration::from_secs(10);

/// What a command refused while the app is closing is told.
pub const CLOSING: &str = "Music is closing, so nothing more is played.";

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    // An answer is written only in the command that settles it (`answers.rs`); what an earlier
    // command wrote is never anyone else's.
    m.answer = Value::Null;
    // A file handed over by a second launch while a close waits: the launch shows the window
    // again straight after (`Event::Presented`), so the close is over and the file is played.
    if m.closing && !m.ending && matches!(command, Command::Open(_)) {
        m.closing = false;
    }
    if (m.closing || m.ending) && plays(&command) {
        return Err(Refused::new("closing", CLOSING));
    }
    if player_job_landed(&command) {
        m.player_jobs = m.player_jobs.saturating_sub(1);
    }
    match command {
        Command::Mpv { player, event } => {
            heard::update(m, player, event, fx);
            Ok(())
        }
        Command::Loaded { .. }
        | Command::Resumed(_)
        | Command::Paused(_)
        | Command::WentBack(_)
        | Command::Rewound(_)
        | Command::Sought(_)
        | Command::VolumeSet { .. }
        | Command::Muted { .. }
        | Command::Stopped(_)
        | Command::Tidied
        | Command::Checked(_) => {
            landed::update(m, command, fx);
            Ok(())
        }
        Command::Rescan
        | Command::Scanned(_)
        | Command::Scanning { .. }
        | Command::Filter(_)
        | Command::AddFolder(_)
        | Command::FolderChecked { .. }
        | Command::RemoveFolder(_) => {
            library::update(m, command, fx);
            Ok(())
        }
        Command::Attach(inbox) => {
            m.inbox = Some(inbox);
            fx.changed(NOTHING);
            Ok(())
        }
        Command::DismissNotice => {
            m.notice.clear();
            fx.changed(PART_STATUS);
            Ok(())
        }
        Command::Retired(last) => {
            retired(m, last, fx);
            Ok(())
        }
        Command::WorkFailed(work) => {
            failed(m, work, fx);
            Ok(())
        }
        Command::Closing(window) => {
            // The window goes at once; the app runs on without it until the player is reaped.
            m.closing = true;
            fx.close(window);
            fx.linger(CLOSE_BACKSTOP);
            retire(m, fx);
            Ok(())
        }
        Command::Reopened => {
            // The close is cancelled. The player is still stopped, as the person asked; the next
            // play starts it again. A logout's `ending` is never cancelled.
            m.closing = false;
            fx.changed(NOTHING);
            Ok(())
        }
        Command::Ending => {
            // Nothing asks first: a logout has nobody there to answer, and nothing Music holds is
            // unsaved (the folder list and the volume are saved as they change). The player is
            // stopped and reaped, and then the app ends.
            m.ending = true;
            if m.running() || m.player_jobs > 0 {
                retire(m, fx);
            } else {
                fx.exit();
            }
            fx.changed(NOTHING);
            Ok(())
        }
        Command::KeptOutside(kept) => {
            library::adopt(m, kept, fx);
            Ok(())
        }
        Command::NotSaved(why) => {
            m.notice = format!("could not save Music's settings to {}: {why}", m.config_file.display());
            fx.changed(PART_STATUS);
            Ok(())
        }
        other => {
            playback::update(m, other, fx);
            Ok(())
        }
    }
}

/// A command that would start or change what plays.
fn plays(command: &Command) -> bool {
    matches!(
        command,
        Command::PlayPause
            | Command::Stop
            | Command::SeekTo(_)
            | Command::VolumeTo(_)
            | Command::ToggleMute
            | Command::PlayRow(_)
            | Command::QueueRow(_)
            | Command::Next
            | Command::Previous
            | Command::Play(_)
            | Command::Pause
            | Command::Seek(_)
            | Command::SetVolume(_)
            | Command::PlayFile(_)
            | Command::Open(_)
    )
}

/// A command that carries one of the player's jobs back, with its result or without one.
fn player_job_landed(command: &Command) -> bool {
    match command {
        Command::Loaded { .. }
        | Command::Resumed(_)
        | Command::Paused(_)
        | Command::WentBack(_)
        | Command::Rewound(_)
        | Command::Sought(_)
        | Command::VolumeSet { .. }
        | Command::Muted { .. }
        | Command::Stopped(_)
        | Command::Tidied
        | Command::Retired(_) => true,
        Command::WorkFailed(work) => work.is_player(),
        _ => false,
    }
}

/// Start one of the player's jobs: it takes the next ticket, so it reaches mpv after every job
/// started before it. `failed` is applied if it ends without a result.
pub fn job(m: &mut Model, fx: &mut Fx, failed: Work, f: impl FnOnce(&Player, Ticket) -> Command + Send + 'static) {
    m.tickets += 1;
    let ticket = m.player.ticket(m.tickets);
    let player = m.player.clone();
    m.player_jobs += 1;
    fx.spawn_or(Command::WorkFailed(failed), move |_| f(&player, ticket));
}

/// Stop and reap the player, after every command already sent to it.
fn retire(m: &mut Model, fx: &mut Fx) {
    job(m, fx, Work::Retire, |p, ticket| Command::Retired(ops::retire(p, ticket)));
}

/// The player is stopped and reaped: nothing it said before is heard any more. A close or a
/// logout waiting for this ends here.
fn retired(m: &mut Model, last: u64, fx: &mut Fx) {
    m.deaf_to = m.deaf_to.max(last);
    m.hearing = m.hearing.max(last);
    m.state = PlayerState::NotStarted;
    m.observed = Observed::default();
    if m.closing || m.ending {
        fx.exit();
    }
    fx.changed(PART_TRANSPORT);
    fx.changed(PART_LISTS);
}

/// A job ended without its result: a panic, or the pool at its bound. The invocation it ran
/// under is told by the framework; the window is told here.
fn failed(m: &mut Model, work: Work, fx: &mut Fx) {
    match work {
        Work::Retire => {
            // Whatever is left of the player ends with the process: the engine is stopped when
            // its last handle goes, and the kernel stops mpv when the app's threads end.
            if m.closing || m.ending {
                fx.exit();
            }
        }
        Work::Scan => {
            m.scans.running = m.scans.running.saturating_sub(1);
            // The rescan the person asked for meanwhile is the one this message asks them to
            // press; it is not owed on top.
            m.scans.again = false;
            m.status = "The scan did not finish; press Rescan to try again.".into();
            m.agent_status = None;
        }
        Work::Check | Work::Folder => m.notice = "That could not be looked at; try again.".into(),
        Work::Volume => {
            m.notice = "The player did not answer; try again.".into();
            // The person's change that did not run lets the newest waiting one go now; a call's
            // leaves the person's in flight as it was.
            if !crate::answers::owed(fx) {
                m.volume_sending = false;
                if let Some(next) = m.volume_next.take() {
                    playback::send_volume(m, fx, next);
                }
            }
        }
        _ => m.notice = "The player did not answer; try again.".into(),
    }
    fx.changed(PART_STATUS);
}

/// What is kept between runs, as the model holds it now.
pub fn kept(m: &Model) -> Kept {
    Kept { folders: Some(m.folders.clone()), volume: m.volume }
}
