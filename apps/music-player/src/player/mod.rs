//! The player as the app's jobs drive it: one mpv at a time, started on the first play, started
//! again on the next play after it went away, and stopped when the app ends.
//!
//! The model holds a [`Player`], as Calendar's holds its service client: a handle shared with the
//! jobs, never touched by `update` except to number a ticket. Everything that speaks to mpv runs
//! in a job (`ops.rs`), in ticket order (`turn.rs`), so the window never waits on the player and
//! two commands never reach it out of order.
//!
//! What mpv says arrives on its reader thread and is posted to the UI thread as a command, with
//! the number of the player that said it, so a report from a player that has since gone is told
//! apart. The playing position is posted once a second at most: the window shows whole seconds,
//! and while nothing plays mpv says nothing, so nothing is drawn.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::mpv::{self, Engine, Event};

pub mod ops;
pub mod turn;

pub use turn::Ticket;
use turn::Turnstile;

/// Everything outside the app the player needs: the program, where its socket may go, and the
/// folder a first run lists. One value, so a test gives the app its stand-ins in one place.
#[derive(Debug, Clone)]
pub struct Rig {
    /// `mpv`, or a stand-in in the tests.
    pub program: String,
    /// The session's runtime directory, where the player's socket goes.
    pub runtime: Option<PathBuf>,
    /// `~/Music`, when there is one: the library a first run lists.
    pub music: Option<PathBuf>,
}

impl Rig {
    /// This machine: mpv, `XDG_RUNTIME_DIR`, and `~/Music` if it is a folder. Read once, before
    /// the window exists.
    #[cfg_attr(test, allow(dead_code))]
    pub fn live() -> Rig {
        let music = yantrik_foundation::place::home().join("Music");
        Rig {
            program: "mpv".into(),
            runtime: yantrik_foundation::xdg::runtime_dir(),
            music: music.is_dir().then_some(music),
        }
    }
}

/// What the app started at these paths plays through: this machine, or under a test the
/// stand-ins that test installed for these paths.
#[cfg(not(test))]
pub fn for_app(_paths: &yantrik_app::Paths) -> Rig {
    Rig::live()
}

#[cfg(test)]
pub use crate::tests::rig_for as for_app;

/// Where the reader thread sends what a player said: the player's number, and the event.
pub type Forward = Arc<dyn Fn(u64, Event) + Send + Sync>;

/// The one player, shared by the model and the jobs.
pub struct Player {
    program: String,
    runtime: Option<PathBuf>,
    lane: Arc<Turnstile>,
    /// The running mpv and its number, once one was started.
    engine: Mutex<Option<(u64, Arc<Engine>)>>,
    /// The number the last player started was given. Numbers only grow.
    started: AtomicU64,
    /// Files handed to the player so far. A wait for one file ends when another is handed over
    /// (`ops::load`), so at most one call's wait holds a job thread.
    loads: AtomicU64,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Player")
    }
}

/// How the next player is started, if one has to be.
#[derive(Clone)]
pub struct Start {
    /// The volume, 0 to 100, and whether it is muted: what the window holds.
    pub volume: f64,
    pub muted: bool,
    pub forward: Forward,
}

impl Player {
    pub fn new(rig: &Rig) -> Arc<Player> {
        Arc::new(Player {
            program: rig.program.clone(),
            runtime: rig.runtime.clone(),
            lane: Turnstile::new(),
            engine: Mutex::new(None),
            started: AtomicU64::new(0),
            loads: AtomicU64::new(0),
        })
    }

    /// The place in line of the job `update` is about to start, numbered by the model.
    pub fn ticket(&self, number: u64) -> Ticket {
        Ticket::new(&self.lane, number)
    }

    /// The running player, if there is one that has not gone away.
    pub fn live(&self) -> Option<(u64, Arc<Engine>)> {
        let slot = self.engine.lock().unwrap_or_else(|p| p.into_inner());
        slot.as_ref().filter(|(_, e)| e.is_alive()).map(|(n, e)| (*n, e.clone()))
    }

    /// The running player, or a new one: `Ok((number, engine, started_now))`, or why mpv could
    /// not be started, in the person's words. Blocks for up to the start timeout: from a job,
    /// in its turn.
    pub fn live_or_start(&self, start: &Start) -> Result<(u64, Arc<Engine>, bool), String> {
        if let Some((n, e)) = self.live() {
            return Ok((n, e, false));
        }
        // One that went away is reaped here, off the UI thread.
        let gone = self.engine.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some((_, e)) = gone {
            e.stop();
        }
        let socket = mpv::socket_path(self.runtime.as_deref()).map_err(|e| e.to_string())?;
        let number = self.started.fetch_add(1, Ordering::SeqCst) + 1;
        let engine = Engine::start(&self.program, socket, start.volume, throttled(number, start.forward.clone()))
            .map_err(|e| e.to_string())?;
        if start.muted {
            let _ = engine.client.set("mute", json!(true));
        }
        let engine = Arc::new(engine);
        *self.engine.lock().unwrap_or_else(|p| p.into_inner()) = Some((number, engine.clone()));
        Ok((number, engine, true))
    }

    /// Counts one more file handed to the player `e` and wakes any wait for the one before it.
    /// Returns this file's number.
    pub fn handed(&self, e: &Engine) -> u64 {
        let n = self.loads.fetch_add(1, Ordering::SeqCst) + 1;
        e.observed.wake();
        n
    }

    /// The number of the last file handed to the player.
    pub fn last_handed(&self) -> u64 {
        self.loads.load(Ordering::SeqCst)
    }

    /// Stops the player, if there is one, and reaps it: the end of the app, or a close. Returns
    /// the number of the last player started, so what it says from now on is not heard. From a
    /// job, in its turn.
    pub fn retire(&self) -> u64 {
        let engine = self.engine.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some((_, e)) = engine {
            e.stop();
        }
        self.started.load(Ordering::SeqCst)
    }

    /// Reaps player `number` if it is the one in hand and has gone away; a newer one is left.
    pub fn bury(&self, number: u64) {
        let mut slot = self.engine.lock().unwrap_or_else(|p| p.into_inner());
        if slot.as_ref().is_some_and(|(n, e)| *n == number && !e.is_alive()) {
            if let Some((_, e)) = slot.take() {
                drop(slot);
                e.stop();
            }
        }
    }

    /// The running player's process id, for a test that checks it is gone.
    #[cfg(test)]
    pub fn pid(&self) -> Option<u32> {
        self.engine.lock().unwrap_or_else(|p| p.into_inner()).as_ref().map(|(_, e)| e.pid())
    }
}

/// What the throttle last passed on, and the position it held back since.
#[derive(Default)]
struct Held {
    passed: Option<f64>,
    held: Option<f64>,
}

/// The reader thread's events for player `number`, with the position passed on only when the
/// second it shows changes, or it goes back. A position held back is passed on when the player
/// pauses or a file ends, so what is drawn and described while it rests is where it stopped.
fn throttled(number: u64, forward: Forward) -> impl Fn(Event) + Send + Sync + 'static {
    let state = Mutex::new(Held::default());
    move |event: Event| {
        let mut s = state.lock().unwrap_or_else(|p| p.into_inner());
        match &event {
            Event::Property(name, value) if name == "time-pos" => {
                if let Some(t) = value.as_f64() {
                    if s.passed.is_some_and(|before| t >= before && t.floor() == before.floor()) {
                        s.held = Some(t);
                        return;
                    }
                    s.passed = Some(t);
                }
                s.held = None;
            }
            Event::Property(name, value) if name == "pause" && value.as_bool() == Some(true) => flush(&mut s, number, &forward),
            Event::EndFile { .. } => flush(&mut s, number, &forward),
            _ => {}
        }
        drop(s);
        forward(number, event);
    }
}

/// Pass on the position held back, if there is one.
fn flush(s: &mut Held, number: u64, forward: &Forward) {
    if let Some(t) = s.held.take() {
        s.passed = Some(t);
        forward(number, Event::Property("time-pos".into(), json!(t)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn the_position_is_passed_on_once_a_second_and_whenever_it_goes_back() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let forward: Forward = Arc::new(move |n, e| sink.lock().unwrap().push((n, e)));
        let on_event = throttled(7, forward);
        for t in [0.0, 0.25, 0.5, 0.99, 1.0, 1.5, 2.01, 1.2, 1.3, 30.0] {
            on_event(Event::Property("time-pos".into(), json!(t)));
        }
        on_event(Event::Property("time-pos".into(), Value::Null));
        on_event(Event::Property("pause".into(), json!(true)));
        let seen = seen.lock().unwrap();
        let positions: Vec<Value> = seen.iter().filter_map(|(_, e)| match e {
            Event::Property(n, v) if n == "time-pos" => Some(v.clone()),
            _ => None,
        }).collect();
        assert_eq!(positions, vec![json!(0.0), json!(1.0), json!(2.01), json!(1.2), json!(30.0), Value::Null]);
        assert!(seen.iter().all(|(n, _)| *n == 7), "every event carries its player's number");
        assert!(seen.iter().any(|(_, e)| matches!(e, Event::Property(n, _) if n == "pause")));
    }

    #[test]
    fn a_pause_passes_on_the_position_it_held_back_first() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let forward: Forward = Arc::new(move |_, e| sink.lock().unwrap().push(e));
        let on_event = throttled(1, forward);
        for t in [12.0, 12.4, 12.9] {
            on_event(Event::Property("time-pos".into(), json!(t)));
        }
        on_event(Event::Property("pause".into(), json!(true)));
        let seen = seen.lock().unwrap();
        assert_eq!(
            *seen,
            vec![
                Event::Property("time-pos".into(), json!(12.0)),
                Event::Property("time-pos".into(), json!(12.9)),
                Event::Property("pause".into(), json!(true)),
            ],
            "paused at 12.9, the window and describe say 12.9 before they say paused"
        );
    }
}
