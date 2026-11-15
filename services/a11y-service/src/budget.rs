//! How much of the watcher's time any one application may have (#1257 review, H2, H3 and N2).
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
//!   once; a slow connection is left alone for [`SLOW_FOR`]. A cap on the number of questions
//!   is kept as well: a question that costs nothing still costs something.
//! - [`Relists`]: a full listing walks every application, so however many things ask for one,
//!   at most one runs per second, and one that failed is tried again with a growing pause until
//!   it succeeds. Listing time is charged to each application it asked, not to everyone, because
//!   the listing is rationed already and its applications are asked together.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Questions one connection may be asked in a second.
pub const PER_CONNECTION: u32 = 20;

/// Questions all connections together may be asked in a second.
pub const IN_TOTAL: u32 = 120;

/// Time one connection may take from the watcher, per second of the watcher's life: 50 ms, 5%.
/// An honest application answers in a millisecond or two, so this is some twenty-five questions
/// a second of ordinary size, and about one of the size that makes a connection slow.
pub const CONNECTION_TIME_PER_SECOND: Duration = Duration::from_millis(50);

/// Time all connections together may take from the watcher per second: 250 ms, a quarter. The
/// rest is the watcher's own, so a dialog is told while every connection is using its share.
pub const TOTAL_TIME_PER_SECOND: Duration = Duration::from_millis(250);

/// An answer slower than this is a strike against its connection.
pub const SLOW_ANSWER: Duration = Duration::from_millis(100);

/// Strikes, within [`SLOW_FOR`] of the last, that mark a connection slow.
pub const STRIKES: u32 = 3;

/// How long a slow connection is left alone.
pub const SLOW_FOR: Duration = Duration::from_secs(10);

/// Connections remembered at most. Past this, the slow ones that are let off soonest go first.
const MAX_TRACKED: usize = 256;

/// The shortest time between two full listings.
pub const RELIST_GAP: Duration = Duration::from_secs(1);

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

    /// What is still owed at `now`, given what drains per second.
    fn at(&mut self, now: Instant, per_second: Duration) -> Duration {
        let elapsed = now.saturating_duration_since(self.at);
        let drained = per_second.mul_f64(elapsed.as_secs_f64());
        self.owed = self.owed.saturating_sub(drained);
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
}

impl Standing {
    fn new(now: Instant) -> Self {
        Self { left: PER_CONNECTION, refills_at: now + Duration::from_secs(1), time: Owed::new(now), strikes: 0, last_strike: None, slow_until: None }
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
    left: u32,
    refills_at: Instant,
    time: Owed,
    /// Times a connection was marked slow, ever. Moves when applications are slow or the watcher
    /// is late, never once per keystroke.
    marks: u64,
}

impl Patience {
    pub fn new(now: Instant) -> Self {
        Self { connections: HashMap::new(), left: IN_TOTAL, refills_at: now + Duration::from_secs(1), time: Owed::new(now), marks: 0 }
    }

    /// How many times a connection has been marked slow.
    pub fn marks(&self) -> u64 {
        self.marks
    }

    /// Whether `connection` is being left alone now. Not asking costs nothing.
    pub fn is_slow(&self, connection: &str, now: Instant) -> bool {
        self.connections.get(connection).is_some_and(|s| s.is_slow(now))
    }

    /// Make room for `connection` in the table: the slow ones are the only ones worth keeping, and
    /// if there are too many even of those, the one that is let off soonest goes.
    fn make_room(&mut self, connection: &str, now: Instant) {
        if self.connections.len() < MAX_TRACKED || self.connections.contains_key(connection) {
            return;
        }
        self.connections.retain(|_, s| s.is_slow(now));
        if self.connections.len() >= MAX_TRACKED {
            let soonest = self.connections.iter().min_by_key(|(_, s)| s.slow_until).map(|(name, _)| name.clone());
            if let Some(name) = soonest {
                self.connections.remove(&name);
            }
        }
    }

    fn standing(&mut self, connection: &str, now: Instant) -> &mut Standing {
        self.make_room(connection, now);
        self.connections.entry(connection.to_string()).or_insert_with(|| Standing::new(now))
    }

    /// Whether `connection` may be asked one more question now. Yes spends one from its ration.
    pub fn allows(&mut self, connection: &str, now: Instant) -> bool {
        if now >= self.refills_at {
            self.left = IN_TOTAL;
            self.refills_at = now + Duration::from_secs(1);
        }
        let total_owed = self.time.at(now, TOTAL_TIME_PER_SECOND);
        let total_left = self.left;
        self.make_room(connection, now);
        let standing = self.connections.entry(connection.to_string()).or_insert_with(|| Standing::new(now));
        if standing.is_slow(now) {
            return false;
        }
        standing.slow_until = None;
        if now >= standing.refills_at {
            standing.left = PER_CONNECTION;
            standing.refills_at = now + Duration::from_secs(1);
        }
        if standing.left == 0 || total_left == 0 {
            return false;
        }
        if standing.time.at(now, CONNECTION_TIME_PER_SECOND) >= CONNECTION_TIME_PER_SECOND || total_owed >= TOTAL_TIME_PER_SECOND {
            return false;
        }
        standing.left -= 1;
        self.left -= 1;
        true
    }

    /// An answer from `connection` took `took`: charge the connection and everyone together, and
    /// count a slow one as a strike. True when this marked the connection slow.
    pub fn spent(&mut self, connection: &str, took: Duration, now: Instant) -> bool {
        self.time.charge(took, now, TOTAL_TIME_PER_SECOND);
        self.listed(connection, took, now)
    }

    /// The same, charged to the connection alone: for applications asked together in a listing,
    /// which are rationed as a whole by [`Relists`].
    pub fn listed(&mut self, connection: &str, took: Duration, now: Instant) -> bool {
        let standing = self.standing(connection, now);
        standing.time.charge(took, now, CONNECTION_TIME_PER_SECOND);
        if took <= SLOW_ANSWER {
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

    /// A listing ran. One that worked settles the want; one that failed stays wanted and is
    /// tried again after a pause that doubles up to [`RELIST_RETRY_CAP`].
    pub fn ran(&mut self, worked: bool, now: Instant) {
        if worked {
            self.wanted = false;
            self.retry_in = RELIST_GAP;
            self.not_before = now + RELIST_GAP;
        } else {
            self.not_before = now + self.retry_in;
            self.retry_in = (self.retry_in * 2).min(RELIST_RETRY_CAP);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_is_asked_only_its_ration_in_a_second() {
        let start = Instant::now();
        let mut patience = Patience::new(start);
        let asked = (0..PER_CONNECTION * 3).filter(|_| patience.allows(":1.7", start)).count();
        assert_eq!(asked as u32, PER_CONNECTION);
        assert!(patience.allows(":1.7", start + Duration::from_secs(1)), "a new second, a new ration");
    }

    #[test]
    fn one_greedy_connection_does_not_spend_another_ones_ration() {
        let start = Instant::now();
        let mut patience = Patience::new(start);
        while patience.allows(":1.7", start) {}
        assert!(patience.allows(":1.8", start), "the other application is still asked");
    }

    #[test]
    fn many_connections_together_cannot_pass_the_total() {
        let start = Instant::now();
        let mut patience = Patience::new(start);
        let asked = (0..200).filter(|n| patience.allows(&format!(":1.{n}"), start)).count();
        assert_eq!(asked as u32, IN_TOTAL);
    }

    #[test]
    fn a_connection_that_timed_out_is_left_alone_for_a_while() {
        let start = Instant::now();
        let mut patience = Patience::new(start);
        assert!(patience.allows(":1.7", start));
        patience.timed_out(":1.7", start);
        assert!(!patience.allows(":1.7", start + Duration::from_secs(5)), "still slow");
        assert!(patience.allows(":1.8", start + Duration::from_secs(5)), "nobody else pays for it");
        assert!(patience.allows(":1.7", start + SLOW_FOR), "asked again once the time has passed");
    }

    #[test]
    fn the_table_of_connections_does_not_grow_without_bound() {
        let start = Instant::now();
        let mut patience = Patience::new(start);
        for n in 0..10_000u64 {
            patience.allows(&format!(":1.{n}"), start + Duration::from_secs(n));
        }
        assert!(patience.connections.len() <= MAX_TRACKED, "{}", patience.connections.len());
    }

    /// #1257 review, deviation 1: `timed_out` used to change only a connection that was already
    /// tracked, and one first met in a listing never was.
    #[test]
    fn a_connection_that_did_not_answer_a_listing_is_left_alone() {
        let start = Instant::now();
        let mut patience = Patience::new(start);
        assert!(patience.timed_out(":1.77", start), "newly marked");
        assert!(!patience.allows(":1.77", start + Duration::from_secs(1)), "never asked before, and now left alone");
        assert!(patience.is_slow(":1.77", start + Duration::from_secs(1)));
        assert!(patience.allows(":1.77", start + SLOW_FOR), "and asked again when its time is up");
    }

    /// #1257 review, deviation 1: 256 slow connections, from one concurrent listing, used to be
    /// able to refuse every connection not already tracked for ten seconds.
    #[test]
    fn a_table_full_of_slow_connections_still_has_room_for_an_honest_one() {
        let start = Instant::now();
        let mut patience = Patience::new(start);
        for n in 0..1000u64 {
            patience.timed_out(&format!(":1.{n}"), start + Duration::from_millis(n));
        }
        assert!(patience.connections.len() <= MAX_TRACKED, "{}", patience.connections.len());
        assert!(patience.allows(":1.honest", start + Duration::from_secs(1)), "the honest application is asked");
        assert!(patience.is_slow(":1.999", start + Duration::from_secs(1)), "the newest slow ones are still remembered");
        assert!(!patience.is_slow(":1.0", start + Duration::from_secs(1)), "the one let off soonest made room");
    }

    /// #1257 review, N2: one connection that answers every question in 390 ms was never marked
    /// slow, and could be asked 20 times a second: 7.8 s of the watcher's time every second.
    #[test]
    fn a_connection_that_answers_just_inside_the_limit_gets_only_its_share_of_time() {
        let start = Instant::now();
        let mut patience = Patience::new(start);
        let answer = Duration::from_millis(390);
        let (mut spent, seconds) = (Duration::ZERO, 60u64);
        for ms in (0..seconds * 1000).step_by(5) {
            let now = start + Duration::from_millis(ms);
            if patience.allows(":1.7", now) {
                patience.spent(":1.7", answer, now);
                spent += answer;
            }
        }
        // Its share, and the one overrun it may borrow.
        let share = CONNECTION_TIME_PER_SECOND * seconds as u32 + answer;
        assert!(spent <= share, "{spent:?} of the watcher's time in {seconds} s, share {share:?}");
        assert!(spent >= answer, "it was asked at all");
    }

    /// #1257 review, N2: bus connections cost nothing, so 50 of them could each spend their own
    /// share and 25 of them 400 ms each.
    #[test]
    fn fifty_connections_cannot_spend_more_than_the_total_time() {
        let start = Instant::now();
        let mut patience = Patience::new(start);
        let answer = Duration::from_millis(40);
        let (mut spent, seconds) = (Duration::ZERO, 20u64);
        for ms in (0..seconds * 1000).step_by(5) {
            let now = start + Duration::from_millis(ms);
            for n in 0..50 {
                let connection = format!(":1.{n}");
                if patience.allows(&connection, now) {
                    patience.spent(&connection, answer, now);
                    spent += answer;
                }
            }
        }
        let share = TOTAL_TIME_PER_SECOND * seconds as u32 + TOTAL_TIME_PER_SECOND + answer;
        assert!(spent <= share, "{spent:?} of the watcher's time in {seconds} s, share {share:?}");
        assert!(spent >= TOTAL_TIME_PER_SECOND * (seconds as u32 - 1), "and the time is used: {spent:?}");
    }

    #[test]
    fn answers_slower_than_a_tenth_of_a_second_count_towards_marking_slow() {
        let start = Instant::now();
        let mut patience = Patience::new(start);
        let slow = SLOW_ANSWER + Duration::from_millis(1);
        for n in 0..STRIKES - 1 {
            assert!(!patience.spent(":1.7", slow, start + Duration::from_secs(u64::from(n) * 5)));
        }
        assert!(!patience.is_slow(":1.7", start + Duration::from_secs(9)), "not yet");
        assert!(patience.spent(":1.7", slow, start + Duration::from_secs(10)), "the last strike marks it");
        assert!(patience.is_slow(":1.7", start + Duration::from_secs(11)));
        assert_eq!(patience.marks(), 1);
        // Quick answers, however many, are never a strike.
        for n in 0..1000u64 {
            patience.spent(":1.8", SLOW_ANSWER, start + Duration::from_millis(n));
        }
        assert!(!patience.is_slow(":1.8", start + Duration::from_secs(2)));
    }

    #[test]
    fn slow_answers_far_apart_do_not_add_up() {
        let start = Instant::now();
        let mut patience = Patience::new(start);
        let slow = SLOW_ANSWER * 2;
        for n in 0..10u64 {
            patience.spent(":1.7", slow, start + (SLOW_FOR + Duration::from_secs(1)) * n as u32);
        }
        assert_eq!(patience.marks(), 0, "a connection slow now and then is not a slow connection");
    }

    #[test]
    fn marking_slow_is_counted_once_a_stretch_not_once_a_question() {
        let start = Instant::now();
        let mut patience = Patience::new(start);
        for n in 0..50u64 {
            patience.timed_out(":1.7", start + Duration::from_millis(n * 100));
        }
        assert_eq!(patience.marks(), 1);
        patience.timed_out(":1.7", start + SLOW_FOR + Duration::from_secs(6));
        assert_eq!(patience.marks(), 2, "and again once it has been let off");
    }

    #[test]
    fn a_thousand_requests_for_a_listing_are_one_listing_a_second() {
        let start = Instant::now();
        let mut relists = Relists::new(start);
        let mut ran = 0;
        // A forged signal every millisecond for ten seconds, each one asking for a listing.
        for ms in 0..10_000u64 {
            let now = start + Duration::from_millis(ms);
            relists.want();
            if relists.due(now) {
                ran += 1;
                relists.ran(true, now);
            }
        }
        assert_eq!(ran, 10);
    }

    #[test]
    fn a_listing_that_failed_stays_wanted_and_is_retried_with_a_growing_pause() {
        let start = Instant::now();
        let mut relists = Relists::new(start);
        relists.want();
        assert!(relists.due(start));
        relists.ran(false, start);
        assert!(!relists.due(start + Duration::from_millis(900)), "not at once");
        assert!(relists.due(start + RELIST_GAP), "but again after the first pause");
        relists.ran(false, start + RELIST_GAP);
        assert_eq!(relists.next(), Some(start + RELIST_GAP + RELIST_GAP * 2), "the pause doubles");
        relists.ran(true, start + Duration::from_secs(5));
        assert!(relists.next().is_none(), "one that worked settles it");
        for _ in 0..20 {
            relists.want();
            relists.ran(false, start);
        }
        assert_eq!(relists.next(), Some(start + RELIST_RETRY_CAP), "and the pause has a ceiling");
    }
}
