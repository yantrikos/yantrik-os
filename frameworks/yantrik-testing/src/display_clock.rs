//! Slint's time on the headless platform (`Platform::duration_since_start`), which its animations
//! and own timers read.
//!
//! It is the machine's time, moved on by however far the clock was held and stepped, so it never
//! goes back. While held ([`hold`]) it stands still and moves only by [`Hold::step`]: an animation
//! then advances one display frame per frame drawn, as on a screen, however long a debug build
//! takes to render each (`TestApp::on_screen`).
//!
//! One clock for the process, as Slint has one platform: a hold is taken on a test's thread and
//! read on the UI thread. While one test holds it, another's animations wait with it.

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

struct Clock {
    /// The machine's time this clock counts from, set at its first reading.
    start: Option<Instant>,
    /// How far ahead of the machine's time the clock is, from the holds before.
    ahead: Duration,
    /// The time while held.
    held: Option<Duration>,
    /// Holds not yet dropped: the clock runs again when the last one goes.
    holds: u32,
}

static CLOCK: Mutex<Clock> = Mutex::new(Clock { start: None, ahead: Duration::ZERO, held: None, holds: 0 });

fn clock() -> MutexGuard<'static, Clock> {
    // Poisoned only by a panic in the few lines below, which leave the clock whole.
    CLOCK.lock().unwrap_or_else(|p| p.into_inner())
}

impl Clock {
    fn elapsed(&mut self) -> Duration {
        self.start.get_or_insert_with(Instant::now).elapsed()
    }

    fn now(&mut self) -> Duration {
        match self.held {
            Some(held) => held,
            None => self.elapsed() + self.ahead,
        }
    }
}

/// The time Slint reads now.
pub(crate) fn now() -> Duration {
    clock().now()
}

/// The clock held still until this is dropped. Dropping the last hold lets it run on with the
/// machine's time from where it was; a hold taken inside another keeps the outer one.
pub(crate) struct Hold(());

pub(crate) fn hold() -> Hold {
    let mut clock = clock();
    if clock.holds == 0 {
        let now = clock.now();
        clock.held = Some(now);
    }
    clock.holds += 1;
    Hold(())
}

impl Hold {
    /// `by` passes on the held clock.
    pub(crate) fn step(&self, by: Duration) {
        let mut clock = clock();
        let now = clock.now();
        clock.held = Some(now + by);
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        let mut clock = clock();
        clock.holds = clock.holds.saturating_sub(1);
        if clock.holds > 0 {
            return;
        }
        if let Some(held) = clock.held.take() {
            // On from the time held to, never back: the machine's time plus what it lacks.
            let elapsed = clock.elapsed();
            clock.ahead = held.saturating_sub(elapsed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_clock_moves_only_by_its_steps_and_never_goes_back_when_let_go() {
        let outer = hold();
        let at = now();
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(now(), at, "held: the machine's time does not move it");
        outer.step(Duration::from_secs(3));
        {
            let inner = hold();
            inner.step(Duration::from_millis(16));
        }
        assert_eq!(now(), at + Duration::from_secs(3) + Duration::from_millis(16), "still held");
        drop(outer);
        assert!(now() >= at + Duration::from_secs(3), "let go, it runs on from where it was");
    }
}
