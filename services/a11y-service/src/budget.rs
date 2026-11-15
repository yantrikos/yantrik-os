//! How much of the watcher's time any one application may have (#1257 review, H2 and H3).
//!
//! Any local program can join the accessibility bus and say anything on it. One that announces
//! a thousand windows a second, or never answers a question, must cost the model that
//! application's facts and nothing else: the dialog another application opens meanwhile is
//! still told on time. Two small rules do that, and both are pure so they can be tested:
//!
//! - [`Patience`]: each connection gets a ration of questions per second, and one that lets a
//!   question time out is not asked again for a while.
//! - [`Relists`]: a full listing walks every application, so however many things ask for one,
//!   at most one runs per second, and one that failed is tried again with a growing pause until
//!   it succeeds.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Questions one connection may be asked in a second.
pub const PER_CONNECTION: u32 = 20;

/// Questions all connections together may be asked in a second.
pub const IN_TOTAL: u32 = 120;

/// How long a connection that let a question time out is left alone.
pub const SLOW_FOR: Duration = Duration::from_secs(10);

/// Connections remembered at most. Past this, only the ones marked slow are kept.
const MAX_TRACKED: usize = 256;

/// The shortest time between two full listings.
pub const RELIST_GAP: Duration = Duration::from_secs(1);

/// The longest pause before a failed listing is tried again.
pub const RELIST_RETRY_CAP: Duration = Duration::from_secs(30);

struct Standing {
    left: u32,
    refills_at: Instant,
    slow_until: Option<Instant>,
}

/// Who may be asked a question now.
pub struct Patience {
    connections: HashMap<String, Standing>,
    left: u32,
    refills_at: Instant,
}

impl Patience {
    pub fn new(now: Instant) -> Self {
        Self { connections: HashMap::new(), left: IN_TOTAL, refills_at: now + Duration::from_secs(1) }
    }

    /// Whether `connection` may be asked one more question now. Yes spends one from its ration.
    pub fn allows(&mut self, connection: &str, now: Instant) -> bool {
        if now >= self.refills_at {
            self.left = IN_TOTAL;
            self.refills_at = now + Duration::from_secs(1);
        }
        if self.connections.len() >= MAX_TRACKED && !self.connections.contains_key(connection) {
            self.connections.retain(|_, s| s.slow_until.is_some_and(|until| until > now));
            if self.connections.len() >= MAX_TRACKED {
                // Hundreds of connections all marked slow: nobody new is asked until they clear.
                return false;
            }
        }
        let standing = self.connections.entry(connection.to_string()).or_insert(Standing {
            left: PER_CONNECTION,
            refills_at: now + Duration::from_secs(1),
            slow_until: None,
        });
        if standing.slow_until.is_some_and(|until| until > now) {
            return false;
        }
        standing.slow_until = None;
        if now >= standing.refills_at {
            standing.left = PER_CONNECTION;
            standing.refills_at = now + Duration::from_secs(1);
        }
        if standing.left == 0 || self.left == 0 {
            return false;
        }
        standing.left -= 1;
        self.left -= 1;
        true
    }

    /// `connection` did not answer in time: leave it alone for [`SLOW_FOR`].
    pub fn timed_out(&mut self, connection: &str, now: Instant) {
        if let Some(standing) = self.connections.get_mut(connection) {
            standing.slow_until = Some(now + SLOW_FOR);
        }
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
