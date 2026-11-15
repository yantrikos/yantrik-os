//! The meeting brief's own clock.
//!
//! The brief used to be ticked inside the companion worker's think arm, so a long, blocked or
//! dead turn stopped every brief (the stall of gate 11). It has a thread of its own now: every
//! [`EVERY`] it makes the same tick the worker made, with the same guards, and publishes the
//! answer for the shell to draw. The thread never waits on a turn, and a turn never waits on it.
//!
//! Only a plain store is ticked here. An encrypted one is readable only by the engine, which the
//! worker holds, so the worker keeps ticking it; [`ticks_here`] is the one rule both sides read,
//! so a store is never ticked twice and no brief is announced twice.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use yantrik_companion::anticipation::AnticipationEngine;
use yantrik_companion::meeting_brief::Outcome;

/// How often a brief is looked for: the same minute the worker's think tick had.
pub const EVERY: Duration = Duration::from_secs(60);
/// How long a connection waits on the worker's lock before the tick is skipped.
const BUSY: Duration = Duration::from_secs(2);
/// How finely the loop looks at its stop flag, so the thread ends soon after the bridge does.
const LOOK: Duration = Duration::from_millis(50);

/// Whether this thread ticks the store (and so the worker does not): the store must be readable
/// without the engine.
pub fn ticks_here() -> bool {
    crate::memory_reader::is_plain()
}

/// Whether the companion worker's think arm ticks the briefs: not when the brief's own clock
/// does (`plain`), and not when the table is missing (`unavailable`).
pub fn worker_ticks(unavailable: bool, plain: bool) -> bool {
    !unavailable && !plain
}

/// The thread, and the name it greets the person by. Dropping it stops the thread.
pub struct BriefTick {
    stop: Arc<AtomicBool>,
    name: Arc<Mutex<String>>,
}

impl BriefTick {
    /// Start ticking the store at `db_path` for `user_name`.
    pub fn start(db_path: String, user_name: String) -> BriefTick {
        let tick = BriefTick::new(user_name);
        let (stop, name) = (tick.stop.clone(), tick.name.clone());
        let spawned = std::thread::Builder::new().name("yos-meeting-brief-tick".into()).spawn(move || {
            run(EVERY, &stop, &name, &ticks_here, &|name, enabled| tick_store(&db_path, name, enabled));
        });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "could not start the meeting brief clock; briefs ride the companion's think tick");
        }
        tick
    }

    fn new(user_name: String) -> BriefTick {
        BriefTick { stop: Arc::new(AtomicBool::new(false)), name: Arc::new(Mutex::new(user_name)) }
    }

    /// A handle with no thread, for a bridge whose worker is a stub.
    #[cfg(test)]
    pub(crate) fn idle(user_name: String) -> BriefTick {
        BriefTick::new(user_name)
    }

    /// The person was renamed.
    pub fn rename(&self, name: &str) {
        *self.name.lock().unwrap_or_else(|e| e.into_inner()) = name.to_string();
    }
}

impl Drop for BriefTick {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// One tick of the store at `db_path`, on a short connection of this thread's own. Nothing when
/// the table is missing, as the worker's `briefs_unavailable` has it.
fn tick_store(db_path: &str, name: &str, enabled: Option<bool>) -> Option<Outcome> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .and_then(|c| c.busy_timeout(BUSY).map(|()| c))
        .map_err(|e| tracing::debug!(error = %e, "meeting brief: the store could not be opened"))
        .ok()?;
    AnticipationEngine::ensure_table(&conn).ok()?;
    Some(yantrik_companion::meeting_brief::local::tick_fresh(&conn, name, enabled))
}

/// Every `every`, until `stop`: tick, unless the store is not ours to tick, and publish what was
/// found. A tick that panics costs that tick only.
fn run(every: Duration, stop: &AtomicBool, name: &Mutex<String>, ours: &dyn Fn() -> bool, tick: &dyn Fn(&str, Option<bool>) -> Option<Outcome>) {
    let mut waited = Duration::ZERO;
    while !stop.load(Ordering::Acquire) {
        std::thread::sleep(LOOK.min(every));
        waited += LOOK.min(every);
        if waited < every {
            continue;
        }
        waited = Duration::ZERO;
        if !ours() {
            continue;
        }
        let who = name.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tick(&who, crate::wire::meeting_brief::switch())));
        match made {
            Ok(Some(outcome)) => crate::wire::meeting_brief::publish(outcome),
            Ok(None) => {}
            Err(_) => tracing::warn!("meeting brief: a tick failed; the next one tries again"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn counting(ticks: &Arc<AtomicUsize>) -> impl Fn(&str, Option<bool>) -> Option<Outcome> + '_ {
        move |_, _| {
            ticks.fetch_add(1, Ordering::SeqCst);
            None
        }
    }

    /// Run the loop on a thread for `for_`, with `tick`, and say how often it ticked.
    fn ticks_in(for_: Duration, plain: bool) -> usize {
        let ticks = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (t, s) = (ticks.clone(), stop.clone());
        let handle = std::thread::spawn(move || run(Duration::from_millis(50), &s, &Mutex::new("Pat".into()), &|| plain, &counting(&t)));
        std::thread::sleep(for_);
        stop.store(true, Ordering::Release);
        let _ = handle.join();
        ticks.load(Ordering::SeqCst)
    }

    #[test]
    fn briefs_tick_with_no_think_and_no_worker_at_all() {
        // There is no companion worker here, and no think command was ever sent: the clock alone ticks.
        assert!(ticks_in(Duration::from_millis(400), true) >= 3, "the brief tick ran without a Think");
    }

    #[test]
    fn a_store_only_the_engine_can_read_is_left_to_the_worker() {
        assert_eq!(ticks_in(Duration::from_millis(400), false), 0, "the store would be ticked twice");
    }

    #[test]
    fn the_worker_and_the_clock_never_tick_the_same_store() {
        for plain in [false, true] {
            for unavailable in [false, true] {
                let clock = plain; // `ticks_here` is `plain`: the clock ticks exactly when it is true.
                assert!(!(clock && worker_ticks(unavailable, plain)), "a plain store would be ticked twice");
            }
        }
        assert!(worker_ticks(false, false), "an encrypted store is still ticked by the worker");
        assert!(!worker_ticks(true, false), "no table, no tick");
    }

    #[test]
    fn a_tick_that_panics_does_not_end_the_clock() {
        let calls = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (c, s) = (calls.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            run(Duration::from_millis(50), &s, &Mutex::new("Pat".into()), &|| true, &|_, _| {
                if c.fetch_add(1, Ordering::SeqCst) == 0 {
                    panic!("boom");
                }
                None
            })
        });
        std::thread::sleep(Duration::from_millis(400));
        stop.store(true, Ordering::Release);
        let _ = handle.join();
        assert!(calls.load(Ordering::SeqCst) >= 3, "the clock stopped after one panic");
    }
}
