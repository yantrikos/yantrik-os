//! The order the player's commands reach mpv in.
//!
//! Every command to mpv runs in a job, off the UI thread, and the framework runs jobs on a pool
//! of threads. Two jobs started one after the other could then reach mpv in either order: a
//! pause and the play pressed just after it, or two steps of a volume drag. So each job is given
//! a [`Ticket`] when `update` starts it, numbered in the order the commands were applied, and
//! waits for its turn before it speaks to mpv. The pool takes jobs first in, first out, so every
//! ticket before a waiting one is already running or done, and a wait cannot deadlock.
//!
//! A ticket is passed when it is dropped, used or not: a job the pool refused, a job that
//! panicked, a job that found nothing to do. None of them holds up the tickets behind it.

use std::collections::BTreeSet;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// The most a ticket waits for the ones before it. Each command to mpv gives up within two
/// seconds and a start within five, so this is reached only by a fault; the job then goes ahead
/// rather than hang a thread of the pool.
const WAIT_AT_MOST: Duration = Duration::from_secs(30);

/// Where the tickets stand.
#[derive(Default)]
pub struct Turnstile {
    state: Mutex<Passed>,
    turned: Condvar,
}

#[derive(Default)]
struct Passed {
    /// The ticket whose turn it is. Tickets count from 1.
    next: u64,
    /// Tickets passed ahead of their turn, waiting for the ones before them to pass.
    early: BTreeSet<u64>,
}

impl Turnstile {
    pub fn new() -> Arc<Turnstile> {
        Arc::new(Turnstile { state: Mutex::new(Passed { next: 1, early: BTreeSet::new() }), turned: Condvar::new() })
    }

    fn pass(&self, number: u64) {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if number >= s.next {
            s.early.insert(number);
            loop {
                let next = s.next;
                if !s.early.remove(&next) {
                    break;
                }
                s.next += 1;
            }
        }
        self.turned.notify_all();
    }

    /// The ticket whose turn it is, for a test.
    #[cfg(test)]
    pub fn next(&self) -> u64 {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).next
    }
}

/// One job's place in the line.
pub struct Ticket {
    lane: Arc<Turnstile>,
    number: u64,
}

/// A ticket whose turn it is. Dropping it lets the next one go.
pub struct Turn(#[allow(dead_code)] Ticket);

impl Ticket {
    pub fn new(lane: &Arc<Turnstile>, number: u64) -> Ticket {
        Ticket { lane: lane.clone(), number }
    }

    /// Waits until every ticket before this one has passed.
    pub fn wait(self) -> Turn {
        let deadline = Instant::now() + WAIT_AT_MOST;
        {
            let mut s = self.lane.state.lock().unwrap_or_else(|p| p.into_inner());
            while s.next < self.number {
                let now = Instant::now();
                if now >= deadline {
                    tracing::warn!(ticket = self.number, waiting_on = s.next, "a command to mpv went ahead of one that never finished");
                    break;
                }
                s = self.lane.turned.wait_timeout(s, deadline - now).unwrap_or_else(|p| p.into_inner()).0;
            }
        }
        Turn(self)
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        self.lane.pass(self.number);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tickets_take_their_turns_in_number_order_whichever_thread_runs_first() {
        let lane = Turnstile::new();
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut threads = Vec::new();
        // Started last first, so the later tickets are waiting before the first one runs.
        for n in (1..=5).rev() {
            let ticket = Ticket::new(&lane, n);
            let order = order.clone();
            threads.push(std::thread::spawn(move || {
                let _turn = ticket.wait();
                order.lock().unwrap().push(n);
            }));
            std::thread::sleep(Duration::from_millis(10));
        }
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(*order.lock().unwrap(), vec![1, 2, 3, 4, 5]);
        assert_eq!(lane.next(), 6);
    }

    #[test]
    fn a_ticket_dropped_unused_never_holds_up_the_ones_behind_it() {
        let lane = Turnstile::new();
        let first = Ticket::new(&lane, 1);
        let second = Ticket::new(&lane, 2);
        let third = Ticket::new(&lane, 3);
        // The second is dropped before its turn (a job the pool refused): it waits for the first.
        drop(second);
        assert_eq!(lane.next(), 1);
        drop(first.wait());
        assert_eq!(lane.next(), 3, "the first passed, and the second with it");
        let started = Instant::now();
        drop(third.wait());
        assert!(started.elapsed() < Duration::from_secs(1), "the third did not wait for the second");
        assert_eq!(lane.next(), 4);
    }
}
