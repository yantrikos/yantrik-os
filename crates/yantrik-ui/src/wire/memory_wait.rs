//! How long the Memory screen waits for an answer before it says so.
//!
//! VM 520, 4 October, OS ace075fd: the screen sat on "1049 stored · searching" over an empty
//! body for as long as the companion was busy, because the wait had no end and the empty state
//! is hidden while a search is running. The newest memories no longer wait on the companion
//! (`crate::memory_reader`), but a search still does — it needs the companion's embedder — and so
//! does the listing when the store can only be read through the engine. Whatever it waits on,
//! the screen gives up after [`BUSY_AFTER`] and offers Retry. Never a blank page.
use std::time::{Duration, Instant};

/// How long a memory search or listing may run before the screen says the memories are busy.
pub const BUSY_AFTER: Duration = Duration::from_secs(3);

/// Where one wait stands.
#[derive(Debug, PartialEq)]
pub enum Wait<T> {
    /// No answer yet, and still within [`BUSY_AFTER`].
    Pending,
    /// The answer.
    Answered(T),
    /// No answer in time: the screen says so and offers Retry.
    Busy,
}

/// Judge a wait that began at `started`, at `now`, given what (if anything) has arrived. An
/// answer that arrives at the deadline still counts: the person asked for it.
pub fn poll<T>(started: Instant, now: Instant, reply: Option<T>) -> Wait<T> {
    match reply {
        Some(answer) => Wait::Answered(answer),
        None if now.saturating_duration_since(started) >= BUSY_AFTER => Wait::Busy,
        None => Wait::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_ends_in_an_answer_or_in_busy_never_in_nothing() {
        let t0 = Instant::now();
        assert_eq!(poll::<u8>(t0, t0, None), Wait::Pending);
        assert_eq!(poll::<u8>(t0, t0 + Duration::from_millis(2900), None), Wait::Pending);
        assert_eq!(poll::<u8>(t0, t0 + BUSY_AFTER, None), Wait::Busy, "the deadline itself is busy");
        assert_eq!(poll::<u8>(t0, t0 + Duration::from_secs(10), None), Wait::Busy);
        assert_eq!(poll(t0, t0 + Duration::from_millis(10), Some(7)), Wait::Answered(7));
        assert_eq!(poll(t0, t0 + BUSY_AFTER, Some(7)), Wait::Answered(7), "an answer at the deadline counts");
    }
}
