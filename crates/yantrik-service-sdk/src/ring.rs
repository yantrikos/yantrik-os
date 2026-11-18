//! A bounded ring a reader follows with a cursor, and can wait on.
//!
//! The shape every "tell me what changed" method in the desktop wants (#1256): a source pushes
//! items from its own thread; a reader asks for everything from the last sequence number it was
//! given, and parks until there is something if there is nothing yet. Four decisions hold it up,
//! and the first three are the ones the perception ring made first:
//!
//! - **Bounded, and honest about it.** The ring keeps a fixed number of items and overwrites the
//!   oldest. A reader that fell behind is told how many it missed, never handed a shorter list
//!   that looks complete.
//! - **A wait, not a poll.** A reader with nothing to read parks on a condvar until an item
//!   arrives or its patience runs out, so knowing costs nothing while nothing happens.
//! - **A panic in one source is not the end of the ring.** A lock poisoned by a thread that
//!   panicked while holding it is used as it stands: the ring's own state is only ever changed
//!   in whole steps, so it is still sound, and every other source and reader carries on.
//! - **A page, not the whole ring.** A reader is handed at most as many items as it asked for
//!   and the cursor for the rest, so one far-behind reader does not copy the ring under its lock.
//!
//! The perception service's own ring predates this one and carries its counters inline; it is the
//! same contract (`seq` in, `next_seq` and `missed` out) and can move here when it is next opened.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// The longest a reader may park. Bounded so a client that vanishes cannot pin a thread.
pub const MAX_WAIT: Duration = Duration::from_secs(30);

struct Inner<T> {
    ring: VecDeque<(u64, T)>,
    next_seq: u64,
    overwritten: u64,
}

/// What a reader gets back.
#[derive(Debug)]
pub struct Page<T> {
    /// The items from the reader's cursor on, oldest first, at most the limit it gave.
    pub items: Vec<T>,
    /// The cursor to ask with next time.
    pub next_seq: u64,
    /// How many items the reader can never have, because the ring had overwritten them.
    pub missed: u64,
    /// There are more items after these: ask again with `next_seq`.
    pub more: bool,
    /// The reader's cursor was ahead of the ring (a ring that restarted, or a made-up number),
    /// so it was read from the ring's own next item instead.
    pub reset: bool,
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

    fn lock(&self) -> MutexGuard<'_, Inner<T>> {
        self.inner.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Add one item and wake every waiting reader. The item is built from its own sequence
    /// number, so it can carry it; it is built before the ring is touched, so a `make` that
    /// panics adds nothing and uses no number.
    pub fn push_with(&self, make: impl FnOnce(u64) -> T) -> u64 {
        let mut inner = self.lock();
        let seq = inner.next_seq;
        let item = make(seq);
        inner.next_seq += 1;
        inner.ring.push_back((seq, item));
        if inner.ring.len() > self.capacity {
            inner.ring.pop_front();
            inner.overwritten += 1;
        }
        drop(inner);
        self.inner.1.notify_all();
        seq
    }

    /// At most `limit` items from `since` on, waiting up to `wait` (at most [`MAX_WAIT`]) when
    /// there is nothing yet. Zero is a fair `wait`: "what do you have right now". A page is at
    /// least one item, so a reader that follows `more` always moves.
    pub fn since(&self, since: u64, wait: Duration, limit: usize) -> Page<T> {
        let limit = limit.max(1);
        let mut inner = self.lock();
        // A cursor from the future would wait out its whole patience while items arrived
        // behind it. It is read from here on instead, and told so.
        let reset = since > inner.next_seq;
        let since = since.min(inner.next_seq);
        if inner.next_seq <= since && !wait.is_zero() {
            // Re-checked under the lock after every wake, so a spurious wake-up is not an empty
            // answer.
            let (guard, _) = self.inner.1.wait_timeout_while(inner, wait.min(MAX_WAIT), |i| i.next_seq <= since).unwrap_or_else(|e| e.into_inner());
            inner = guard;
        }
        let oldest = inner.ring.front().map_or(inner.next_seq, |(seq, _)| *seq);
        let mut from = inner.ring.iter().skip_while(|(seq, _)| *seq < since);
        let items: Vec<(u64, T)> = from.by_ref().take(limit).cloned().collect();
        let more = from.next().is_some();
        let next_seq = if more { items.last().map_or(since, |(seq, _)| seq + 1) } else { inner.next_seq };
        Page { items: items.into_iter().map(|(_, item)| item).collect(), next_seq, missed: oldest.saturating_sub(since), more, reset }
    }

    /// The cursor a reader that wants only what happens from now on should start with.
    pub fn next_seq(&self) -> u64 {
        self.lock().next_seq
    }

    /// How many items the ring has overwritten since it started.
    pub fn overwritten(&self) -> u64 {
        self.lock().overwritten
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const ALL: usize = usize::MAX;

    #[test]
    fn a_reader_gets_what_came_after_its_cursor_and_the_next_cursor() {
        let ring = CursorRing::new(8);
        for word in ["a", "b", "c"] {
            ring.push_with(|seq| format!("{seq}:{word}"));
        }
        let page = ring.since(1, Duration::ZERO, ALL);
        assert_eq!(page.items, ["1:b", "2:c"]);
        assert_eq!((page.next_seq, page.missed, page.more, page.reset), (3, 0, false, false));
        assert!(ring.since(page.next_seq, Duration::ZERO, ALL).items.is_empty(), "nothing new yet");
    }

    #[test]
    fn a_reader_that_fell_behind_is_told_how_many_it_missed() {
        let ring = CursorRing::new(3);
        for n in 0..10u64 {
            ring.push_with(|_| n);
        }
        let page = ring.since(2, Duration::ZERO, ALL);
        assert_eq!(page.items, [7, 8, 9], "only what the ring still holds");
        assert_eq!(page.missed, 5, "items 2 to 6 are gone, and the reader is told");
        assert_eq!(ring.overwritten(), 7);
    }

    #[test]
    fn a_reader_far_behind_gets_a_page_and_the_cursor_for_the_rest() {
        let ring = CursorRing::new(100);
        for n in 0..50u64 {
            ring.push_with(|_| n);
        }
        let first = ring.since(0, Duration::ZERO, 20);
        assert_eq!(first.items.len(), 20, "no more than it asked for");
        assert_eq!((first.next_seq, first.more), (20, true));
        let second = ring.since(first.next_seq, Duration::ZERO, 20);
        assert_eq!(second.items.first(), Some(&20), "nothing skipped, nothing repeated");
        let last = ring.since(40, Duration::ZERO, 20);
        assert_eq!((last.items.len(), last.next_seq, last.more), (10, 50, false));
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
        let page = ring.since(0, Duration::from_secs(10), ALL);
        pusher.join().expect("the source thread");
        assert_eq!(page.items, [0]);
        assert!(started.elapsed() < Duration::from_secs(5), "woken, not timed out");
    }

    #[test]
    fn a_wait_with_nothing_to_read_ends_empty_at_its_limit() {
        let ring: CursorRing<u64> = CursorRing::new(8);
        let started = Instant::now();
        let page = ring.since(0, Duration::from_millis(30), ALL);
        assert!(page.items.is_empty());
        assert_eq!(page.next_seq, 0);
        assert!(started.elapsed() >= Duration::from_millis(25));
    }

    /// #1257 review, L2: a cursor ahead of the ring used to wait out its whole patience while
    /// items arrived, and then return none of them.
    #[test]
    fn a_cursor_from_the_future_reads_from_now() {
        let ring = CursorRing::new(8);
        ring.push_with(|seq| seq);
        let source = ring.clone();
        let started = Instant::now();
        let pusher = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            source.push_with(|seq| seq);
        });
        let page = ring.since(u64::MAX, Duration::from_secs(10), ALL);
        pusher.join().expect("the source thread");
        assert!(page.reset, "the reader is told its cursor was not one of ours");
        assert_eq!(page.items, [1], "it gets what came after it asked");
        assert_eq!(page.next_seq, 2);
        assert!(started.elapsed() < Duration::from_secs(5), "woken by the item, not by the clock");
        assert!(!ring.since(2, Duration::ZERO, ALL).reset, "a cursor at the ring's own next is not ahead");
    }

    /// #1257 review, L1: a std mutex stays poisoned, so "one item lost" used to mean every later
    /// push dropped and every wait returning at once.
    #[test]
    fn a_source_that_panics_while_pushing_leaves_the_ring_working() {
        let ring = CursorRing::new(8);
        ring.push_with(|seq| seq);
        let source = ring.clone();
        let panicked = std::thread::spawn(move || source.push_with(|_| panic!("a source that fails mid-push"))).join();
        assert!(panicked.is_err());
        assert_eq!(ring.push_with(|seq| seq), 1, "the failed push used no number, and the next one works");
        let page = ring.since(0, Duration::ZERO, ALL);
        assert_eq!(page.items, [0, 1]);
        assert_eq!(ring.next_seq(), 2);
    }

    /// #1257 review, N11: a page limit of zero returned `more: true` with the cursor where it
    /// was, and a reader that followed `more` looped for ever.
    #[test]
    fn a_page_limit_of_zero_still_moves_the_cursor() {
        let ring = CursorRing::new(8);
        ring.push_with(|seq| seq);
        let page = ring.since(0, Duration::ZERO, 0);
        assert_eq!((page.items.len(), page.next_seq, page.more), (1, 1, false));
    }

    #[test]
    fn an_empty_ring_reports_no_gap() {
        let ring: CursorRing<u64> = CursorRing::new(2);
        let page = ring.since(0, Duration::ZERO, ALL);
        assert_eq!((page.missed, page.next_seq), (0, 0));
    }
}
