//! The ring, and the wait.
//!
//! Sources push observations from their own threads; readers ask for everything after the last
//! sequence number they saw. Two decisions are load-bearing:
//!
//! **Bounded, and honest about it.** The ring holds a fixed number of observations and overwrites
//! the oldest. A perception system must not be able to exhaust memory because something started
//! writing files in a loop. A reader that falls behind is told how many it missed rather than
//! being handed a silently shortened list — a gap you can see is a different thing from a gap you
//! cannot.
//!
//! **A wait, not a poll.** The whole argument for watching from the kernel is that it costs
//! nothing while nothing is happening. Answering `perception.since` by returning immediately would
//! push the polling one layer up and give all of it back, so a reader with nothing to read parks
//! on a condvar until either something arrives or its patience runs out.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::observation::{Actor, Kind, Observation};

/// How many observations are kept for readers that have not caught up.
///
/// A minute or two of ordinary activity. Enough that a reader can drop its connection, reconnect
/// and lose nothing; small enough that a runaway source cannot grow it without bound.
const CAPACITY: usize = 2048;

/// The longest a reader may park. Bounded so a client that vanishes cannot pin a thread forever.
pub const MAX_WAIT: Duration = Duration::from_secs(30);

/// How long two reports of the same save are treated as one save.
///
/// Generous on purpose. The two arrive milliseconds apart in practice — two threads draining two
/// descriptors — and the cost of the window being too wide is that somebody who saved the same
/// file twice inside two seconds is reported once, which is also a fair description of what they
/// did.
const COALESCE_WINDOW: f64 = 2.0;

#[derive(Default)]
struct Inner {
    ring: VecDeque<Observation>,
    next_seq: u64,
    /// Sequence of the oldest observation still held. A reader asking for something older than
    /// this has missed events, and needs to be told so.
    oldest: u64,
    /// Counts by kind since start, for `perception.snapshot`. Cheap, and it answers "has anything
    /// been happening at all" without walking the ring.
    launched: u64,
    ended: u64,
    saved: u64,
    /// Scratch writes: the machinery of a save, counted separately so the ratio is visible. A
    /// desktop reporting thousands of these and no saves is one where the rename group is down.
    wrote: u64,
    /// Saves that turned out to be the same save described a second time. Counted rather than
    /// silent, so the ring never looks quieter than the disk actually was.
    coalesced: u64,
    executed: u64,
    dropped_out_of_scope: u64,
    /// Observations evicted from the back of the ring because it was full. Counted rather than
    /// silent, so a reader can see how much the ring has overwritten without doing arithmetic on
    /// `oldest`.
    overwritten: u64,
    /// Launches filtered as the OS's own chatter before they reached the ring. Counted, never
    /// described, so the filter is visible rather than a silent hole.
    dropped_noise: u64,
}

/// Shared, cloneable handle. Sources hold one, the RPC handler holds one.
#[derive(Clone)]
pub struct Bus {
    inner: Arc<(Mutex<Inner>, Condvar)>,
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus {
    pub fn new() -> Self {
        Self { inner: Arc::new((Mutex::new(Inner::default()), Condvar::new())) }
    }

    /// Record an observation and wake anyone waiting.
    pub fn push(&self, kind: Kind, actor: Option<crate::observation::Actor>) {
        let (lock, cv) = &*self.inner;
        let Ok(mut inner) = lock.lock() else {
            // A poisoned lock means a source thread panicked mid-push. Losing this observation is
            // better than propagating the panic into every other source.
            return;
        };

        // One save reaches this function twice: the descriptor group sees the close, the rename
        // group sees the directory entry change, and both resolve to the same document a few
        // milliseconds apart. Two true statements about one thing a person did — and announcing it
        // twice would wake everything expensive twice.
        if already_reported(&kind, actor.as_ref(), &inner.ring) {
            inner.coalesced += 1;
            return;
        }

        let seq = inner.next_seq;
        inner.next_seq += 1;
        match &kind {
            Kind::Launched { .. } => inner.launched += 1,
            Kind::Ended { .. } => inner.ended += 1,
            Kind::Saved { .. } => inner.saved += 1,
            Kind::Wrote { .. } => inner.wrote += 1,
            Kind::Executed { .. } => inner.executed += 1,
            _ => {}
        }

        inner.ring.push_back(Observation::new(seq, kind, actor));
        if inner.ring.len() > CAPACITY {
            inner.ring.pop_front();
            inner.oldest += 1;
            inner.overwritten += 1;
        }
        drop(inner);
        cv.notify_all();
    }

    /// Note that something happened which the scope would not let us report.
    ///
    /// Counted, never described. The count is what makes the eyelid visible — a caller can see
    /// that the daemon is declining to say things without learning anything about them.
    pub fn note_out_of_scope(&self) {
        if let Ok(mut inner) = self.inner.0.lock() {
            inner.dropped_out_of_scope += 1;
        }
    }

    /// Note that a launch was filtered as the OS's own chatter before it reached the ring.
    ///
    /// Counted, never described. The count is what makes the filter visible — a caller can see
    /// how much the daemon is declining to record without learning anything about it.
    pub fn note_noise(&self) {
        if let Ok(mut inner) = self.inner.0.lock() {
            inner.dropped_noise += 1;
        }
    }

    /// Everything after `since`, waiting up to `wait` if there is nothing yet.
    pub fn since(&self, since: u64, wait: Duration) -> Page {
        let (lock, cv) = &*self.inner;
        let Ok(mut inner) = lock.lock() else {
            return Page { observations: Vec::new(), next_seq: since, missed: 0 };
        };

        if inner.next_seq <= since && !wait.is_zero() {
            // `wait_timeout_while` re-checks under the lock after every wake, so a spurious
            // wakeup does not turn into an empty answer.
            let (guard, _) = cv
                .wait_timeout_while(inner, wait.min(MAX_WAIT), |i| i.next_seq <= since)
                .unwrap_or_else(|e| e.into_inner());
            inner = guard;
        }

        // A reader that fell off the back of the ring learns the size of its blind spot instead of
        // receiving a shorter list that looks complete.
        let missed = inner.oldest.saturating_sub(since);
        let observations: Vec<Observation> =
            inner.ring.iter().filter(|o| o.seq >= since).cloned().collect();
        Page { observations, next_seq: inner.next_seq, missed }
    }

    pub fn counts(&self) -> Counts {
        let Ok(inner) = self.inner.0.lock() else {
            return Counts::default();
        };
        Counts {
            launched: inner.launched,
            ended: inner.ended,
            saved: inner.saved,
            wrote: inner.wrote,
            coalesced: inner.coalesced,
            executed: inner.executed,
            dropped_out_of_scope: inner.dropped_out_of_scope,
            overwritten: inner.overwritten,
            dropped_noise: inner.dropped_noise,
            held: inner.ring.len() as u64,
            next_seq: inner.next_seq,
        }
    }
}

/// Whether this save is already in the ring under another description.
///
/// Same document, same process, inside [`COALESCE_WINDOW`]. `how` is deliberately not compared:
/// the whole point is that a closed descriptor and a rename are two accounts of one save, and
/// requiring them to agree about the mechanism would make the rule never fire.
///
/// First report wins, and that is not arbitrary — it is the one a reader may already have been
/// woken for, and preferring the later one would mean retracting something already delivered.
fn already_reported(kind: &Kind, actor: Option<&Actor>, ring: &VecDeque<Observation>) -> bool {
    let Kind::Saved { path, .. } = kind else { return false };
    let pid = actor.map(|a| a.pid);
    let now = crate::observation::now();

    ring.iter()
        .rev()
        .take_while(|o| now - o.at <= COALESCE_WINDOW)
        .any(|o| match &o.kind {
            Kind::Saved { path: held, .. } => {
                held == path && o.actor.as_ref().map(|a| a.pid) == pid
            }
            _ => false,
        })
}

pub struct Page {
    pub observations: Vec<Observation>,
    pub next_seq: u64,
    pub missed: u64,
}

#[derive(Default, serde::Serialize)]
pub struct Counts {
    pub launched: u64,
    pub ended: u64,
    pub saved: u64,
    pub wrote: u64,
    pub coalesced: u64,
    pub executed: u64,
    pub dropped_out_of_scope: u64,
    pub overwritten: u64,
    pub dropped_noise: u64,
    pub held: u64,
    pub next_seq: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::observation::SaveShape;

    fn launch(bus: &Bus, cmd: &str) {
        bus.push(Kind::Launched { command: cmd.into() }, None);
    }

    fn saver(pid: i32) -> Option<Actor> {
        Some(Actor { pid, name: "soffice".into(), parent: None })
    }

    fn save(bus: &Bus, pid: i32, path: &str, how: SaveShape) {
        bus.push(Kind::Saved { path: path.into(), how }, saver(pid));
    }

    #[test]
    fn one_save_described_twice_is_one_save() {
        // The atomic save as it actually arrives. The descriptor group resolves the renamed inode
        // and reports a close on the document; the rename group reports the same document a few
        // milliseconds later from its own thread. Two accounts of one thing a person did, and
        // waking the expensive faculties twice for it would be a bug with a cost.
        let bus = Bus::new();
        save(&bus, 900, "/home/p/report.odt", SaveShape::ClosedWrite);
        save(&bus, 900, "/home/p/report.odt", SaveShape::Replaced);

        let page = bus.since(0, Duration::ZERO);
        assert_eq!(page.observations.len(), 1, "one save should reach a reader once");
        assert_eq!(bus.counts().saved, 1);
        // Never silently: a ring that quietly drops things is the failure this service is against.
        assert_eq!(bus.counts().coalesced, 1, "the second account must be counted, not vanish");
    }

    #[test]
    fn two_documents_are_two_saves_however_close_together() {
        let bus = Bus::new();
        save(&bus, 900, "/home/p/one.odt", SaveShape::Replaced);
        save(&bus, 900, "/home/p/two.odt", SaveShape::Replaced);
        assert_eq!(bus.since(0, Duration::ZERO).observations.len(), 2);
        assert_eq!(bus.counts().coalesced, 0);
    }

    #[test]
    fn the_same_file_written_by_two_processes_is_two_saves() {
        // Two people, or a build and an editor, touching one file. Collapsing these would hide
        // exactly the collision somebody would want to know about.
        let bus = Bus::new();
        save(&bus, 900, "/home/p/shared.md", SaveShape::ClosedWrite);
        save(&bus, 901, "/home/p/shared.md", SaveShape::ClosedWrite);
        assert_eq!(bus.since(0, Duration::ZERO).observations.len(), 2);
    }

    #[test]
    fn coalescing_only_ever_applies_to_saves() {
        // A process launched twice, or the same binary run twice, is two events. The rule exists
        // for one specific double-report and must not quietly deduplicate the rest of perception.
        let bus = Bus::new();
        launch(&bus, "cargo build");
        launch(&bus, "cargo build");
        bus.push(Kind::Wrote { path: "/home/p/.x.swp".into() }, saver(900));
        bus.push(Kind::Wrote { path: "/home/p/.x.swp".into() }, saver(900));
        assert_eq!(bus.since(0, Duration::ZERO).observations.len(), 4);
    }

    #[test]
    fn a_reader_gets_only_what_it_has_not_seen() {
        let bus = Bus::new();
        launch(&bus, "one");
        launch(&bus, "two");

        let first = bus.since(0, Duration::ZERO);
        assert_eq!(first.observations.len(), 2);
        assert_eq!(first.next_seq, 2);

        launch(&bus, "three");
        let second = bus.since(first.next_seq, Duration::ZERO);
        assert_eq!(second.observations.len(), 1);
        assert_eq!(second.observations[0].summary, "started: three");
    }

    #[test]
    fn falling_behind_is_reported_not_hidden() {
        let bus = Bus::new();
        for i in 0..(CAPACITY + 10) {
            launch(&bus, &format!("p{i}"));
        }
        let page = bus.since(0, Duration::ZERO);
        assert_eq!(page.missed, 10, "a reader must learn the size of its blind spot");
        assert_eq!(page.observations.len(), CAPACITY);
    }

    #[test]
    fn a_waiting_reader_is_woken_by_a_push() {
        let bus = Bus::new();
        let writer = bus.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(80));
            launch(&writer, "late");
        });

        let started = std::time::Instant::now();
        let page = bus.since(0, Duration::from_secs(5));
        assert_eq!(page.observations.len(), 1);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the reader should wake on the push, not sit out the timeout ({:?})",
            started.elapsed()
        );
    }

    #[test]
    fn a_reader_with_nothing_to_read_returns_empty_rather_than_hanging() {
        let bus = Bus::new();
        let started = std::time::Instant::now();
        let page = bus.since(0, Duration::from_millis(120));
        assert!(page.observations.is_empty());
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn overwriting_the_ring_is_counted_in_the_snapshot() {
        let bus = Bus::new();
        for i in 0..(CAPACITY + 10) {
            launch(&bus, &format!("p{i}"));
        }
        let counts = bus.counts();
        assert_eq!(counts.overwritten, 10, "the evicted observations must be counted");
        assert_eq!(counts.held, CAPACITY as u64);
    }

    #[test]
    fn filtered_noise_is_counted_but_never_enters_the_ring() {
        let bus = Bus::new();
        bus.note_noise();
        bus.note_noise();
        let counts = bus.counts();
        assert_eq!(counts.dropped_noise, 2);
        // And nothing about them reached the ring.
        assert_eq!(bus.since(0, Duration::ZERO).observations.len(), 0);
    }

    #[test]
    fn what_the_scope_refused_is_counted_but_never_described() {
        let bus = Bus::new();
        bus.note_out_of_scope();
        bus.note_out_of_scope();
        let counts = bus.counts();
        assert_eq!(counts.dropped_out_of_scope, 2);
        // And nothing about them reached the ring.
        assert_eq!(bus.since(0, Duration::ZERO).observations.len(), 0);
    }
}
