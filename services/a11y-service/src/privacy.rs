//! Reading "is the person private", and noticing when that changes (#1257 review, M1 and N5).
//!
//! The answer is a file. The watcher reads it two ways: a reading up to [`FRESH`] old for the
//! loop's own look (a firehose of signals must not become a firehose of file reads), and a fresh
//! one whenever something is about to be recorded, because an event told in the first moments
//! of the private time must not be recorded. A fresh reading that says private while the watcher
//! thought not is an edge like any other and is acted on where it is seen ([`edge`]); otherwise
//! a private time shorter than the interval between two looks would never be seen at all.
//!
//! That the person was private is remembered on `Sight`, not here: a private time that begins
//! just before the bus goes away, or ends while the service is reconnecting, is told as a gap
//! by the next session all the same.

use std::time::{Duration, Instant};

/// How long one reading is trusted by [`Privacy::check`].
pub const FRESH: Duration = Duration::from_millis(100);

/// Whether the person is private, read from its file.
pub struct Privacy {
    read: fn() -> bool,
    value: bool,
    read_at: Option<Instant>,
}

impl Privacy {
    pub fn new(read: fn() -> bool) -> Self {
        Self { read, value: false, read_at: None }
    }

    /// The last reading, or a new one if it is older than [`FRESH`].
    pub fn check(&mut self, now: Instant) -> bool {
        if self.read_at.is_none_or(|at| now.duration_since(at) >= FRESH) {
            return self.fresh(now);
        }
        self.value
    }

    /// Read the file now. What was read is also what [`Privacy::check`] answers from here on, so
    /// the loop's next look does not contradict what was just acted on.
    pub fn fresh(&mut self, now: Instant) -> bool {
        self.value = (self.read)();
        self.read_at = Some(now);
        self.value
    }
}

/// A change in whether the person is private.
#[derive(Debug, PartialEq, Eq)]
pub enum Edge {
    /// What was waiting to be told is forgotten, and so is the focus.
    Began,
    /// A reader is told there is a hole where the private time was.
    Ended,
}

/// What a new reading means, given what was believed before it.
pub fn edge(was: bool, now_private: bool) -> Option<Edge> {
    match (was, now_private) {
        (false, true) => Some(Edge::Began),
        (true, false) => Some(Edge::Ended),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    static PRIVATE_NOW: AtomicBool = AtomicBool::new(false);
    static PRIVATE_READS: AtomicU64 = AtomicU64::new(0);

    fn read_private() -> bool {
        PRIVATE_READS.fetch_add(1, Ordering::SeqCst);
        PRIVATE_NOW.load(Ordering::SeqCst)
    }

    /// A firehose of signals is not a firehose of file reads, and the answer is never older than
    /// its limit.
    #[test]
    fn private_mode_is_read_at_most_once_per_interval() {
        let start = Instant::now();
        let mut privacy = Privacy::new(read_private);
        for micros in 0..1_000u64 {
            assert!(!privacy.check(start + Duration::from_micros(micros)));
        }
        assert_eq!(PRIVATE_READS.load(Ordering::SeqCst), 1, "a thousand looks inside the interval, one read");
        PRIVATE_NOW.store(true, Ordering::SeqCst);
        assert!(!privacy.check(start + FRESH / 2), "still the last reading");
        assert!(privacy.check(start + FRESH), "and a fresh one once it is due");
        assert_eq!(PRIVATE_READS.load(Ordering::SeqCst), 2);
    }

    static LATER_NOW: AtomicBool = AtomicBool::new(false);

    fn read_later() -> bool {
        LATER_NOW.load(Ordering::SeqCst)
    }

    /// #1257 review, N5: a fresh reading is what the next look answers, so the loop does not
    /// read "not private" from before what the fresh reading just saw.
    #[test]
    fn a_fresh_reading_is_what_the_next_look_answers() {
        let start = Instant::now();
        let mut privacy = Privacy::new(read_later);
        assert!(!privacy.check(start));
        LATER_NOW.store(true, Ordering::SeqCst);
        assert!(!privacy.check(start + FRESH / 2), "the loop's own look is up to FRESH old");
        assert!(privacy.fresh(start + FRESH / 2), "a fresh reading sees it");
        assert!(privacy.check(start + FRESH / 2 + Duration::from_millis(1)), "and so does the look after it, inside the interval");
    }

    /// #1257 review, N5: the edge is the same whichever look sees it.
    #[test]
    fn private_beginning_and_ending_are_the_only_edges() {
        assert_eq!(edge(false, true), Some(Edge::Began));
        assert_eq!(edge(true, false), Some(Edge::Ended));
        assert_eq!(edge(false, false), None);
        assert_eq!(edge(true, true), None);
    }
}
