//! What mpv last reported, and a condition to wait on for the next change.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use super::client::Event;
use super::engine::REDIRECTED;

/// The last value mpv reported for every observed property, and the last file that ended.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Observed {
    pub time_pos: Option<f64>,
    pub duration: Option<f64>,
    pub paused: bool,
    pub volume: Option<f64>,
    pub muted: bool,
    pub idle: bool,
    pub path: Option<String>,
    /// How many files have loaded since the engine started; a waiter compares it.
    pub loads: u64,
    /// The last `end-file` with an error: what mpv said.
    pub last_error: Option<String>,
    /// How many files have failed since the engine started; a waiter compares it, so an old
    /// failure is never read as the answer to a new load.
    pub errors: u64,
    pub gone: bool,
    /// The last few files that loaded, each with its load's number and what mpv had said by
    /// then. A waiter asks this, not the latest state: mpv may load a file and be handed the
    /// next one before the waiter looks, and the moment the first was playing is then gone.
    pub loaded: VecDeque<(u64, Moment)>,
}

/// What mpv had said at one moment: the fields a report carries.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Moment {
    pub path: Option<String>,
    pub time_pos: Option<f64>,
    pub duration: Option<f64>,
    pub paused: bool,
    pub volume: Option<f64>,
    pub muted: bool,
    pub playing: bool,
}

/// How many loads [`Observed::loaded`] keeps.
const LOADS_KEPT: usize = 8;

impl Observed {
    /// What mpv had said when `path` loaded, if it did after load number `after`.
    pub fn loaded_since(&self, after: u64, path: &str) -> Option<&Moment> {
        self.loaded.iter().rev().find(|(n, m)| *n > after && m.path.as_deref() == Some(path)).map(|(_, m)| m)
    }

    fn moment(&self) -> Moment {
        Moment {
            path: self.path.clone(),
            time_pos: self.time_pos,
            duration: self.duration,
            paused: self.paused,
            volume: self.volume,
            muted: self.muted,
            playing: self.playing(),
        }
    }

    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::Property(name, v) => match name.as_str() {
                "time-pos" => self.time_pos = v.as_f64(),
                "duration" => self.duration = v.as_f64(),
                "pause" => self.paused = v.as_bool().unwrap_or(false),
                "volume" => self.volume = v.as_f64(),
                "mute" => self.muted = v.as_bool().unwrap_or(false),
                "idle-active" => self.idle = v.as_bool().unwrap_or(false),
                "path" => self.path = v.as_str().map(str::to_string),
                _ => {}
            },
            Event::FileLoaded => {
                self.loads += 1;
                self.last_error = None;
                if self.loaded.len() == LOADS_KEPT {
                    self.loaded.pop_front();
                }
                let at = self.moment();
                self.loaded.push_back((self.loads, at));
            }
            Event::EndFile { reason, error } => match reason.as_str() {
                "error" => {
                    self.errors += 1;
                    self.last_error = Some(error.clone().unwrap_or_else(|| "it could not be played".into()));
                }
                // The file turned out to be a playlist and mpv went on to what it lists. That is
                // a refusal: what was asked for did not play.
                "redirect" => {
                    self.errors += 1;
                    self.last_error = Some(REDIRECTED.into());
                }
                _ => {}
            },
            Event::Gone => self.gone = true,
        }
    }

    /// Playing means a file is loaded and not paused, by mpv's account.
    pub fn playing(&self) -> bool {
        !self.gone && !self.idle && self.path.is_some() && !self.paused
    }
}

/// [`Observed`] behind a lock, with a condition to wait on for the next change.
#[derive(Default)]
pub struct Shared {
    state: Mutex<Observed>,
    changed: Condvar,
}

impl Shared {
    pub fn apply(&self, event: &Event) {
        self.state.lock().unwrap().apply(event);
        self.changed.notify_all();
    }

    /// Wakes every waiter to look again at what it waits for, with nothing new reported: the
    /// player was handed another file, and a wait for the last one may be over.
    pub fn wake(&self) {
        let _state = self.state.lock().unwrap();
        self.changed.notify_all();
    }

    pub fn snapshot(&self) -> Observed {
        self.state.lock().unwrap().clone()
    }

    /// Wait up to `limit` for `done` to hold of what mpv has reported, and return the last
    /// report either way.
    pub fn wait_for(&self, limit: Duration, done: impl Fn(&Observed) -> bool) -> (bool, Observed) {
        let deadline = Instant::now() + limit;
        let mut state = self.state.lock().unwrap();
        loop {
            if done(&state) {
                return (true, state.clone());
            }
            let now = Instant::now();
            if now >= deadline {
                return (false, state.clone());
            }
            state = self.changed.wait_timeout(state, deadline - now).unwrap().0;
        }
    }
}
