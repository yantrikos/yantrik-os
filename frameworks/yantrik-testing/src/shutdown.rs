//! The end of an app under test, timed where the core does it.

use std::time::{Duration, Instant};

use yantrik_app::app::App;

use crate::TestApp;

/// The grace a test's shutdown is given: what the core's bound is measured against.
pub const GRACE: Duration = Duration::from_millis(100);

impl<A: App> TestApp<A> {
    /// The end, as the runner does it, and how long `Core::shutdown` itself took. The clock runs
    /// on the UI thread, around the call alone: every test shares that one thread, so a step
    /// queued behind other tests' steps (many test threads, a busy machine) must not count as
    /// the shutdown's time. The bound the core enforces is `GRACE`.
    pub fn shutdown_timed(&self) -> (bool, Duration) {
        self.with(|u| {
            let began = Instant::now();
            let clean = u.core.shutdown(GRACE);
            (clean, began.elapsed())
        })
    }

    /// The end, as the runner does it.
    pub fn shutdown(&self) -> bool {
        self.shutdown_timed().0
    }
}
