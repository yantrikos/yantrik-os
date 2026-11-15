//! A bounded ring a reader follows with a cursor, and can wait on.
//!
//! The shape every "tell me what changed" method in the desktop wants (#1256): a source pushes
//! items from its own thread; a reader asks for everything from the last sequence number it was
//! given, and parks until there is something if there is nothing yet. Three decisions hold it up,
//! and they are the ones the perception ring made first:
//!
//! - **Bounded, and honest about it.** The ring keeps a fixed number of items and overwrites the
//!   oldest. A reader that fell behind is told how many it missed, never handed a shorter list
//!   that looks complete.
//! - **A wait, not a poll.** A reader with nothing to read parks on a condvar until an item
//!   arrives or its patience runs out, so knowing costs nothing while nothing happens.
//! - **A poisoned lock loses one item, not the service.** A source that panicked mid-push must
//!   not take every reader with it.
//!
//! The perception service's own ring predates this one and carries its counters inline; it is the
//! same contract (`seq` in, `next_seq` and `missed` out) and can move here when it is next opened.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// The longest a reader may park. Bounded so a client that vanishes cannot pin a thread.
pub const MAX_WAIT: Duration = Duration::from_secs(30);

struct Inner<T> {
    ring: VecDeque<(u64, T)>,
    next_seq: u64,
    overwritten: u64,
}

/// What a reader gets back: the items from its cursor on, the cursor to ask with next time, and
/// how many items it can never have because the ring had already overwritten them.
#[derive(Debug)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_seq: u64,
    pub missed: u64,
}

/// A shared, cloneable handle: sources hold one, the RPC handler holds one.
pub struct CursorRing<T> {
    inner: Arc<(Mutex<Inner<T>>, Condvar)>,
    capacity: usize,
}

impl<T> Clone for CursorRing<T> {
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner), capacity: self.capacity }
    }
}

impl<T: Clone> CursorRing<T> {
    /// A ring that keeps the newest `capacity` items (at least one).
    pub fn new(capacity: usize) -> Self {
        let inner = Inner { ring: VecDeque::new(), next_seq: 0, overwritten: 0 };
        Self { inner: Arc::new((Mutex::new(inner), Condvar::new())), capacity: capacity.max(1) }
    }

    /// Add one item and wake every waiting reader. The item is built from its own sequence
    /// number, so it can carry it. Returns that number, or `None` when the lock was poisoned and
    /// the item was dropped.
    pub fn push_with(&self, make: impl FnOnce(u64) -> T) -> Option<u64> {
        let (lock, cv) = &*self.inner;
        let mut inner = lock.lock().ok()?;
        let seq = inner.next_seq;
        inner.next_seq += 1;
        inner.ring.push_back((seq, make(seq)));
        if inner.ring.len() > self.capacity {
            inner.ring.pop_front();
            inner.overwritten += 1;
        }
        drop(inner);
        cv.notify_all();
        Some(seq)
    }

    /// Everything from `since` on, waiting up to `wait` (at most [`MAX_WAIT`]) when there is
    /// nothing yet. Zero is a fair `wait`: "what do you have right now".
    pub fn since(&self, since: u64, wait: Duration) -> Page<T> {
        let (lock, cv) = &*self.inner;
        let Ok(mut inner) = lock.lock() else {
            return Page { items: Vec::new(), next_seq: since, missed: 0 };
        };
        if inner.next_seq <= since && !wait.is_zero() {
            // Re-checked under the lock after every wake, so a spurious wake-up is not an empty
            // answer.
            let (guard, _) = cv
                .wait_timeout_while(inner, wait.min(MAX_WAIT), |i| i.next_seq <= since)
                .unwrap_or_else(|e| e.into_inner());
            inner = guard;
        }
        let oldest = inner.ring.front().map_or(inner.next_seq, |(seq, _)| *seq);
        let items = inner.ring.iter().filter(|(seq, _)| *seq >= since).map(|(_, item)| item.clone()).collect();
        Page { items, next_seq: inner.next_seq, missed: oldest.saturating_sub(since) }
    }

    /// The cursor a reader that wants only what happens from now on should start with.
    pub fn next_seq(&self) -> u64 {
        self.inner.0.lock().map_or(0, |inner| inner.next_seq)
    }

    /// How many items the ring has overwritten since it started.
    pub fn overwritten(&self) -> u64 {
        self.inner.0.lock().map_or(0, |inner| inner.overwritten)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_reader_gets_what_came_after_its_cursor_and_the_next_cursor() {
        let ring = CursorRing::new(8);
        for word in ["a", "b", "c"] {
            ring.push_with(|seq| format!("{seq}:{word}"));
        }
        let page = ring.since(1, Duration::ZERO);
        assert_eq!(page.items, ["1:b", "2:c"]);
        assert_eq!((page.next_seq, page.missed), (3, 0));
        assert!(ring.since(page.next_seq, Duration::ZERO).items.is_empty(), "nothing new yet");
    }

    #[test]
    fn a_reader_that_fell_behind_is_told_how_many_it_missed() {
        let ring = CursorRing::new(3);
        for n in 0..10u64 {
            ring.push_with(|_| n);
        }
        let page = ring.since(2, Duration::ZERO);
        assert_eq!(page.items, [7, 8, 9], "only what the ring still holds");
        assert_eq!(page.missed, 5, "items 2 to 6 are gone, and the reader is told");
        assert_eq!(ring.overwritten(), 7);
    }

    #[test]
    fn a_waiting_reader_is_woken_by_the_next_item_not_by_the_clock() {
        let ring = CursorRing::new(8);
        let source = ring.clone();
        let started = Instant::now();
        let pusher = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            source.push_with(|seq| seq);
        });
        let page = ring.since(0, Duration::from_secs(10));
        pusher.join().expect("the source thread");
        assert_eq!(page.items, [0]);
        assert!(started.elapsed() < Duration::from_secs(5), "woken, not timed out");
    }

    #[test]
    fn a_wait_with_nothing_to_read_ends_empty_at_its_limit() {
        let ring: CursorRing<u64> = CursorRing::new(8);
        let started = Instant::now();
        let page = ring.since(0, Duration::from_millis(30));
        assert!(page.items.is_empty());
        assert_eq!(page.next_seq, 0);
        assert!(started.elapsed() >= Duration::from_millis(25));
    }

    #[test]
    fn an_empty_ring_reports_no_gap_to_a_reader_ahead_of_it() {
        let ring: CursorRing<u64> = CursorRing::new(2);
        assert_eq!(ring.since(5, Duration::ZERO).missed, 0);
        assert_eq!(ring.next_seq(), 0);
    }
}
