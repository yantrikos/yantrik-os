//! How many mail calls may wait on the service at once, and whether it just let us down.
//!
//! A call may wait up to a minute (`port::NETWORK_BUDGET`). Against a service that accepts and
//! never answers, every model-driven call would hold a worker for that minute, and clearing the
//! transport's breaker before each one would take away the only thing that makes the second call
//! cheap (#1026 review, L3). So the port keeps two small facts here: a ceiling on calls in
//! flight, and the time of its own last timeout. After a timeout of ours the breaker's verdict
//! stands for a while and the next calls fail at once, in words.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Calls that may be waiting on the mail service together. Tools run a few at a time at most; the
/// ceiling is there for the day one runs away.
pub const MAX_IN_FLIGHT: usize = 4;

/// How long the port believes the breaker after a timeout of its own.
pub const TRUST_BREAKER_FOR: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Flight {
    in_flight: AtomicUsize,
    last_timeout: Mutex<Option<Instant>>,
}

/// One call's place; given back when it is dropped.
pub struct Permit(Arc<Flight>);

impl Flight {
    /// A place for one more call, or `None` when [`MAX_IN_FLIGHT`] are already waiting.
    pub fn enter(self: &Arc<Self>) -> Option<Permit> {
        let before = self.in_flight.fetch_add(1, Ordering::SeqCst);
        if before >= MAX_IN_FLIGHT {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Permit(self.clone()))
    }

    /// The service just failed to answer within the call's budget.
    pub fn note_timeout(&self) {
        if let Ok(mut last) = self.last_timeout.lock() {
            *last = Some(Instant::now());
        }
    }

    /// Whether this port's own call timed out within [`TRUST_BREAKER_FOR`].
    pub fn timed_out_recently(&self) -> bool {
        self.last_timeout.lock().ok().and_then(|last| *last).is_some_and(|at| at.elapsed() < TRUST_BREAKER_FOR)
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_more_than_the_ceiling_wait_together_and_a_place_is_given_back() {
        let flight = Arc::new(Flight::default());
        let held: Vec<_> = (0..MAX_IN_FLIGHT).map(|_| flight.enter().expect("room")).collect();
        assert!(flight.enter().is_none(), "one over the ceiling");
        drop(held);
        assert!(flight.enter().is_some(), "places come back");
    }

    #[test]
    fn a_timeout_is_remembered_only_for_a_while() {
        let flight = Flight::default();
        assert!(!flight.timed_out_recently());
        flight.note_timeout();
        assert!(flight.timed_out_recently());
        *flight.last_timeout.lock().unwrap() = Some(Instant::now() - TRUST_BREAKER_FOR - Duration::from_secs(1));
        assert!(!flight.timed_out_recently());
    }
}
