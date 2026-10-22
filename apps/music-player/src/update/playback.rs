//! Play, pause, the queue, seeking, the volume and `play_file`: what the transport and the Mind
//! ask of the player. Each check that needs only the model is made here, before anything
//! changes; each that needs mpv's word or the disk is made in the job, which answers.

use crate::answers::{agent, owed, refuse};
use crate::command::{Command, Work};
use crate::library::{self, Track};
use crate::model::{Model, PART_LISTS, PART_STATUS, PART_TRANSPORT};
use crate::player::ops;

use super::{job, kept, Fx};

/// What every command that finds the queue empty says.
pub const QUEUE_EMPTY: &str = "The queue is empty: pick a track in the library first.";

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) {
    match command {
        Command::PlayPause => {
            if m.playing() {
                pause(m, fx);
            } else {
                play(m, fx);
            }
        }
        Command::Play(None) => play(m, fx),
        Command::Play(Some(index)) if index < 1 => refuse(m, fx, "bad_argument", "`index` counts from 1."),
        Command::Play(Some(index)) => play_shown(m, fx, index as usize - 1),
        Command::PlayRow(row) => {
            if row >= 0 {
                play_shown(m, fx, row as usize);
            }
        }
        Command::QueueRow(row) => {
            let track = if row >= 0 { m.queue.jump(row as usize).cloned() } else { None };
            match track {
                Some(t) => load(m, fx, t),
                None => refuse(m, fx, "not_in_queue", "That track is no longer in the queue."),
            }
        }
        Command::Pause => pause(m, fx),
        Command::Next => next(m, fx),
        Command::Previous => {
            let answer = owed(fx);
            job(m, fx, Work::Back, move |p, t| Command::WentBack(Box::new(ops::previous(p, t, answer))));
        }
        Command::Seek(seconds) => {
            if !seconds.is_finite() || seconds < 0.0 {
                return refuse(m, fx, "bad_argument", "`seconds` is a point in the track, 0 or more.");
            }
            if let Some(d) = m.observed().duration.filter(|d| seconds > *d) {
                let long = library::clock(Some(d));
                return refuse(m, fx, "bad_argument", format!("The track is {long} long; {seconds} seconds is past its end."));
            }
            seek_to(m, fx, seconds);
        }
        Command::SeekTo(fraction) => match m.observed().duration {
            Some(d) => seek_to(m, fx, d * fraction as f64),
            None => refuse(m, fx, "nothing_playing", "Nothing is playing, so there is nothing to seek in."),
        },
        Command::SetVolume(percent) if !(0.0..=100.0).contains(&percent) => {
            refuse(m, fx, "bad_argument", format!("`percent` is from 0 to 100, not {percent}."))
        }
        Command::SetVolume(percent) => set_volume(m, fx, percent),
        Command::VolumeTo(level) => set_volume(m, fx, level as f64 * 100.0),
        Command::ToggleMute => {
            let muted = !m.muted;
            if m.running() {
                job(m, fx, Work::Mute, move |p, t| Command::Muted { muted, result: ops::mute(p, t, muted) });
            } else {
                m.muted = muted;
                fx.changed(PART_TRANSPORT);
            }
        }
        Command::Stop => job(m, fx, Work::Stop, |p, t| Command::Stopped(ops::stop(p, t))),
        Command::PlayFile(raw) => {
            let raw = raw.trim().to_string();
            if raw.is_empty() {
                return refuse(m, fx, "bad_argument", "`play_file` needs `path`.");
            }
            check(fx, yantrik_foundation::home::expand(&raw), agent(fx));
        }
        // A launch is the person's own: what they opened from Files or named on the command line.
        Command::Open(path) => check(fx, path, false),
        _ => {}
    }
}

/// Play or resume: what is paused, else the queue's current track, else the library from the
/// top.
fn play(m: &mut Model, fx: &mut Fx) {
    if m.loaded() {
        let answer = owed(fx);
        job(m, fx, Work::Resume, move |p, t| Command::Resumed(Box::new(ops::resume(p, t, answer))));
    } else {
        play_from_queue(m, fx);
    }
}

/// `play` with nothing loaded: the queue's current track, or the library from the top.
pub fn play_from_queue(m: &mut Model, fx: &mut Fx) {
    if let Some(t) = m.queue.current().cloned() {
        return load(m, fx, t);
    }
    if m.shown.is_empty() {
        return refuse(
            m,
            fx,
            "empty_library",
            "Nothing to play: the library is empty. Add a folder that holds music under Folders.",
        );
    }
    play_shown(m, fx, 0);
}

/// Start the library at `at` (an index into what the window lists) with the list as the queue.
fn play_shown(m: &mut Model, fx: &mut Fx, at: usize) {
    let tracks = m.shown_tracks();
    match m.queue.start(tracks, at).cloned() {
        Some(t) => load(m, fx, t),
        None => refuse(m, fx, "no_such_track", format!("There is no track {} in the library list.", at + 1)),
    }
}

fn next(m: &mut Model, fx: &mut Fx) {
    match m.queue.next().cloned() {
        Some(t) => load(m, fx, t),
        None if m.queue.is_empty() => refuse(m, fx, "queue_empty", QUEUE_EMPTY),
        None => refuse(m, fx, "queue_end", "This is the last track in the queue."),
    }
}

/// Hand `track` to the player, starting one if there is none. An intent's job waits for mpv to
/// say it is playing, and answers with its report. Everything is drawn: the queue may have
/// changed, and a complaint about the last file is put away now, so what mpv says about this one
/// is never cleared by its own landing.
pub fn load(m: &mut Model, fx: &mut Fx, track: Track) {
    if m.notice.starts_with("Could not play") || m.notice.starts_with("Nothing") {
        m.notice.clear();
    }
    let stale = m.notice.contains("mpv").then(|| m.notice.clone());
    let start = m.start();
    let (answer, agent) = (owed(fx), agent(fx));
    job(m, fx, Work::Load, move |p, t| {
        let loaded = ops::load(p, t, &track, &start, answer, agent);
        Command::Loaded { track, loaded: Box::new(loaded), stale }
    });
    fx.changed(PART_LISTS);
    fx.changed(PART_STATUS);
    fx.changed(PART_TRANSPORT);
}

fn pause(m: &mut Model, fx: &mut Fx) {
    let answer = owed(fx);
    job(m, fx, Work::Pause, move |p, t| Command::Paused(ops::pause(p, t, answer)));
}

fn seek_to(m: &mut Model, fx: &mut Fx, seconds: f64) {
    let answer = owed(fx);
    job(m, fx, Work::Seek, move |p, t| Command::Sought(ops::seek(p, t, seconds, answer)));
}

/// The volume is the window's at once and kept for next time; the player is told in its turn.
/// The person's changes go one at a time: while one is on its way the newest waits in the model,
/// and is sent when that one lands (`send_volume`), so a drag never piles up jobs and the last
/// value is the one mpv ends at. A call's change is sent at once, to be answered with mpv's word.
fn set_volume(m: &mut Model, fx: &mut Fx, percent: f64) {
    let percent = percent.clamp(0.0, 100.0);
    m.volume = percent;
    fx.save_settings(kept(m));
    fx.changed(PART_TRANSPORT);
    if !owed(fx) && m.volume_sending {
        m.volume_next = Some(percent);
    } else {
        send_volume(m, fx, percent);
    }
}

/// Tell the player `percent`. A call's value is the newest: a value of the person's still
/// waiting for the one ahead of it is dropped, so it cannot land after the call's.
pub fn send_volume(m: &mut Model, fx: &mut Fx, percent: f64) {
    let answer = owed(fx);
    if answer {
        m.volume_next = None;
    } else {
        m.volume_sending = true;
    }
    job(m, fx, Work::Volume, move |p, t| Command::VolumeSet { percent, result: ops::volume(p, t, percent, answer) });
}

/// `play_file`'s path, looked at off the UI thread: an agent's held to the home rule first, then
/// any caller's to being a sound file.
fn check(fx: &mut Fx, path: std::path::PathBuf, agent: bool) {
    fx.spawn_or(Command::WorkFailed(Work::Check), move |_| {
        let checked = if agent { crate::seen::may_play(&path.to_string_lossy()) } else { Ok(()) }
            .and_then(|()| library::audio_file(&path))
            .map(|path| library::track_from(&path));
        Command::Checked(checked)
    });
}

/// A track that ended, by its title.
pub fn title_now(m: &Model) -> String {
    m.queue.current().map(|t| t.title.clone()).unwrap_or_default()
}

/// The answer a `previous` at the top of the queue goes on to.
pub fn step_back(m: &mut Model, fx: &mut Fx) {
    match m.queue.previous().cloned() {
        Some(t) => load(m, fx, t),
        None if m.queue.is_empty() => refuse(m, fx, "queue_empty", QUEUE_EMPTY),
        None => {
            let answer = owed(fx);
            job(m, fx, Work::Back, move |p, t| Command::Rewound(ops::rewind(p, t, answer)));
        }
    }
}
