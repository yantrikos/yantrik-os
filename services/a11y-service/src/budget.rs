//! How much of the watcher's time any one application may have (#1257 review, H2, H3, N2, H1 and
//! M1 of round 3).
//!
//! Any local program can join the accessibility bus and say anything on it. One that announces
//! a thousand windows a second, or answers every question just inside its limit, or never
//! answers, must cost the model that application's facts and nothing else: the dialog another
//! application opens meanwhile is still told on time. Two small rules do that, and both are pure
//! so they can be tested:
//!
//! - [`Patience`]: the watcher's *time* is rationed, not its questions. Every answer is charged
//!   for how long it took, to the connection and to everyone together, and a connection that has
//!   used up its share is not asked until it has drained. Answers slower than [`SLOW_ANSWER`]
//!   count towards marking the connection slow, and one that did not answer at all is marked at
//!   once; a slow connection is left alone for [`SLOW_FOR`]. A cap of [`PER_CONNECTION`]
//!   questions a second is kept as well, because a question that costs nothing still costs
//!   something. There is no cap on all connections together: one that answers at once would use
//!   it up, and an honest application would be refused for it.
//! - [`Relists`]: a full listing walks every application, so however many things ask for one,
//!   at most one runs per second, and one that failed is tried again with a growing pause until
//!   it succeeds. A listing blocks the watcher for as long as it takes, so it is also spaced by
//!   three times its own duration: listings take at most a quarter of the watcher's time.
//!
//! # The total is shared fairly
//!
//! Everyone together may take [`TOTAL_TIME_PER_SECOND`] a second. When that is spent, a first
//! come, first served gate would shut out whoever asks next, however little they have used. So
//! a connection that owes less than a tenth of its own share ([`LIGHT`]) is still admitted, ahead
//! of those that owe more, as long as the watcher is not more than [`TOTAL_CEILING`] behind with
//! what it is expected to cost: what its last answer cost, or [`EXPECTED_ANSWER`] if it has not
//! been asked. That is a loan, not a larger total: the total drains at the same rate, so what
//! these connections can take over a long time is the total and no more. And the one that takes
//! 49 ms needs more room than the one that takes 1 ms, so ten connections that each keep within
//! their own share cannot keep a quiet one out.
//!
//! What is refused is not lost: a window whose question was refused is kept by the watcher
//! (`pending`) and asked again, and a listing, which asks for nothing from this budget, finds it
//! in the end. Nothing is told late for want of budget for longer than about a second.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Questions one connection may be asked in a second.
pub const PER_CONNECTION: u32 = 20;

/// Time one connection may take from the watcher, per second of the watcher's life: 50 ms, 5%.
/// An honest application answers in a millisecond or two, so this is some twenty-five questions
/// a second of ordinary size, and about one of the size that makes a connection slow.
pub const CONNECTION_TIME_PER_SECOND: Duration = Duration::from_millis(50);

/// Time all connections together may take from the watcher per second: 250 ms, a quarter. The
/// rest is the watcher's own.
pub const TOTAL_TIME_PER_SECOND: Duration = Duration::from_millis(250);

/// What a connection owes at most, with the total spent, to be admitted all the same: a tenth of
/// its own share.
pub const LIGHT: Duration = Duration::from_millis(5);

/// How far behind, in time owed to the watcher, a connection that owes next to nothing is still
/// let in: four times the total, a second. Past this nobody is asked until it has drained.
pub const TOTAL_CEILING: Duration = Duration::from_millis(1000);

/// What a connection that has not been asked yet is expected to cost: as much as an answer can
/// take and still be quick for a listing.
pub const EXPECTED_ANSWER: Duration = Duration::from_millis(25);

/// An answer slower than this is a strike against its connection.
pub const SLOW_ANSWER: Duration = Duration::from_millis(100);

/// A listing answer slower than this is a strike. An honest `GetChildren` takes a few
/// milliseconds, and a listing asks many applications, so it gets a quarter of the time.
pub const LISTED_SLOW_ANSWER: Duration = Duration::from_millis(25);

/// Strikes, within [`SLOW_FOR`] of the last, that mark a connection slow.
pub const STRIKES: u32 = 3;

/// How long a slow connection is left alone.
pub const SLOW_FOR: Duration = Duration::from_secs(10);

/// Connections remembered at most. Past this, one is forgotten at a time: the one that owes
/// least and was used longest ago.
pub const MAX_TRACKED: usize = 256;

/// The shortest time between two full listings.
pub const RELIST_GAP: Duration = Duration::from_secs(1);

/// A listing is followed by a pause of at least this many times its duration, so that listings
/// take a quarter of the watcher's time at most.
pub const RELIST_SPACING: u32 = 3;

/// The longest pause before a failed listing is tried again.
pub const RELIST_RETRY_CAP: Duration = Duration::from_secs(30);

/// Time owed to the watcher that drains at a fixed rate. Asking is allowed while less is owed
/// than the rate gives in one second, so the average is the rate and one overrun is the most
/// that can be borrowed.
#[derive(Debug, Clone, Copy)]
struct Owed {
    owed: Duration,
    at: Instant,
}

impl Owed {
    fn new(now: Instant) -> Self {
        Self { owed: Duration::ZERO, at: now }
    }

    /// What would be owed at `now`, given what drains per second, changing nothing.
    fn peek(&self, now: Instant, per_second: Duration) -> Duration {
        let elapsed = now.saturating_duration_since(self.at);
        self.owed.saturating_sub(per_second.mul_f64(elapsed.as_secs_f64()))
    }

    /// What is still owed at `now`, given what drains per second.
    fn at(&mut self, now: Instant, per_second: Duration) -> Duration {
        self.owed = self.peek(now, per_second);
        self.at = self.at.max(now);
        self.owed
    }

    fn charge(&mut self, took: Duration, now: Instant, per_second: Duration) {
        self.at(now, per_second);
        self.owed += took;
    }
}

struct Standing {
    left: u32,
    refills_at: Instant,
    time: Owed,
    strikes: u32,
    last_strike: Option<Instant>,
    slow_until: Option<Instant>,
    last_used: Instant,
    /// What its last answer cost: what the next is expected to.
    predicted: Duration,
}

impl Standing {
    fn new(now: Instant) -> Self {
        Self {
            left: PER_CONNECTION,
            refills_at: now + Duration::from_secs(1),
            time: Owed::new(now),
            strikes: 0,
            last_strike: None,
            slow_until: None,
            last_used: now,
            predicted: EXPECTED_ANSWER,
        }
    }

    fn is_slow(&self, now: Instant) -> bool {
        self.slow_until.is_some_and(|until| until > now)
    }

    /// Mark slow. True when it was not marked already.
    fn mark(&mut self, now: Instant) -> bool {
        let newly = !self.is_slow(now);
        self.slow_until = Some(now + SLOW_FOR);
        self.strikes = 0;
        self.last_strike = None;
        newly
    }
}

/// Who may be asked a question now.
pub struct Patience {
    connections: HashMap<String, Standing>,
    /// Time every connection together owes.
    time: Owed,
    /// Times a connection was marked slow, ever. Moves when applications are slow or the watcher
    /// is late, never once per keystroke.
    marks: u64,
}

impl Patience {
    pub fn new(now: Instant) -> Self {
        Self { connections: HashMap::new(), time: Owed::new(now), marks: 0 }
    }

    /// How many times a connection has been marked slow.
    pub fn marks(&self) -> u64 {
        self.marks
    }

    /// Whether `connection` is being left alone now. Not asking costs nothing.
    #[cfg(test)]
    pub fn is_slow(&self, connection: &str, now: Instant) -> bool {
        self.connections.get(connection).is_some_and(|s| s.is_slow(now))
    }

    /// Whether a listing should leave `connection` out: it is slow, or it owes more than its
    /// share of time. A listing asks every application, so one that was slow to answer the last
    /// listing is asked again only once it has drained (#1257 review, M1).
    pub fn is_busy(&self, connection: &str, now: Instant) -> bool {
        self.connections.get(connection).is_some_and(|s| s.is_slow(now) || s.time.peek(now, CONNECTION_TIME_PER_SECOND) >= CONNECTION_TIME_PER_SECOND)
    }

    /// Make room for `connection` in the table by forgetting one entry: the one that is not
    /// slow, owes least and was used longest ago. If every entry is slow, the one let off
    /// soonest. Never all of them: what the others owe, and their strikes, are kept (#1257
    /// review, M1).
    fn make_room(&mut self, connection: &str, now: Instant) {
        if self.connections.len() < MAX_TRACKED || self.connections.contains_key(connection) {
            return;
        }
        let victim = self
            .connections
            .iter()
            .filter(|(_, s)| !s.is_slow(now))
            .min_by_key(|(_, s)| (s.time.peek(now, CONNECTION_TIME_PER_SECOND), s.last_used))
            .or_else(|| self.connections.iter().min_by_key(|(_, s)| s.slow_until))
            .map(|(name, _)| name.clone());
        if let Some(name) = victim {
            self.connections.remove(&name);
        }
    }

    fn standing(&mut self, connection: &str, now: Instant) -> &mut Standing {
        self.make_room(connection, now);
        let standing = self.connections.entry(connection.to_string()).or_insert_with(|| Standing::new(now));
        standing.last_used = now;
        standing
    }

    /// Whether `connection` may be asked one more question now. Yes spends one from its ration.
    pub fn allows(&mut self, connection: &str, now: Instant) -> bool {
        let total_owed = self.time.at(now, TOTAL_TIME_PER_SECOND);
        let standing = self.standing(connection, now);
        if standing.is_slow(now) {
            return false;
        }
        standing.slow_until = None;
        if now >= standing.refills_at {
            standing.left = PER_CONNECTION;
            standing.refills_at = now + Duration::from_secs(1);
        }
        let own = standing.time.at(now, CONNECTION_TIME_PER_SECOND);
        if standing.left == 0 || own >= CONNECTION_TIME_PER_SECOND {
            return false;
        }
        // The total is spent: only one that owes next to nothing is let in, and only if the
        // watcher is not so far behind that what it is expected to cost would pass the ceiling.
        if total_owed >= TOTAL_TIME_PER_SECOND && (own >= LIGHT || total_owed + standing.predicted >= TOTAL_CEILING) {
            return false;
        }
        standing.left -= 1;
        true
    }

    /// An answer from `connection` took `took`: charge the connection and everyone together, and
    /// count a slow one as a strike. True when this marked the connection slow.
    pub fn spent(&mut self, connection: &str, took: Duration, now: Instant) -> bool {
        self.time.charge(took, now, TOTAL_TIME_PER_SECOND);
        self.struck(connection, took, now, SLOW_ANSWER)
    }

    /// The same for an application asked in a listing, charged to the connection alone: the
    /// listing's own time is charged to everyone by [`Patience::listing_took`]. An answer over
    /// [`LISTED_SLOW_ANSWER`] is a strike.
    pub fn listed(&mut self, connection: &str, took: Duration, now: Instant) -> bool {
        self.struck(connection, took, now, LISTED_SLOW_ANSWER)
    }

    /// A listing blocked the watcher for `took`: charged to everyone together.
    pub fn listing_took(&mut self, took: Duration, now: Instant) {
        self.time.charge(took, now, TOTAL_TIME_PER_SECOND);
    }

    fn struck(&mut self, connection: &str, took: Duration, now: Instant, slow: Duration) -> bool {
        let standing = self.standing(connection, now);
        standing.time.charge(took, now, CONNECTION_TIME_PER_SECOND);
        standing.predicted = took;
        if took <= slow {
            return false;
        }
        if standing.last_strike.is_some_and(|at| now.saturating_duration_since(at) > SLOW_FOR) {
            standing.strikes = 0;
        }
        standing.strikes += 1;
        standing.last_strike = Some(now);
        if standing.strikes < STRIKES {
            return false;
        }
        self.marked(connection, now)
    }

    /// `connection` did not answer in time: leave it alone for [`SLOW_FOR`], whether or not it
    /// was asked before. True when this marked it.
    pub fn timed_out(&mut self, connection: &str, now: Instant) -> bool {
        self.marked(connection, now)
    }

    fn marked(&mut self, connection: &str, now: Instant) -> bool {
        let newly = self.standing(connection, now).mark(now);
        self.marks += u64::from(newly);
        newly
    }

    #[cfg(test)]
    pub fn tracked(&self) -> usize {
        self.connections.len()
    }
}

/// When the next full listing may run.
pub struct Relists {
    wanted: bool,
    not_before: Instant,
    retry_in: Duration,
}

impl Relists {
    pub fn new(now: Instant) -> Self {
        Self { wanted: false, not_before: now, retry_in: RELIST_GAP }
    }

    /// Something needs the model reconciled. Asking again before it has run changes nothing.
    pub fn want(&mut self) {
        self.wanted = true;
    }

    pub fn due(&self, now: Instant) -> bool {
        self.wanted && now >= self.not_before
    }

    /// When a wanted listing may run, for the watcher's sleep.
    pub fn next(&self) -> Option<Instant> {
        self.wanted.then_some(self.not_before)
    }

    /// A listing that took `took` ran. One that worked settles the want; one that failed stays
    /// wanted and is tried again after a pause that doubles up to [`RELIST_RETRY_CAP`]. Either
    /// way the next one waits at least [`RELIST_SPACING`] times as long as this one took.
    pub fn ran(&mut self, worked: bool, took: Duration, now: Instant) {
        let spacing = took * RELIST_SPACING;
        if worked {
            self.wanted = false;
            self.retry_in = RELIST_GAP;
            self.not_before = now + RELIST_GAP.max(spacing);
        } else {
            self.not_before = now + self.retry_in.max(spacing);
            self.retry_in = (self.retry_in * 2).min(RELIST_RETRY_CAP);
        }
    }
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;
