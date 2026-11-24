//! Which accounts are known to be down, so a call for one goes straight to the private lane
//! instead of paying the connect timeout again (#1304).
//!
//! Per account, in memory, bounded, and holding no content: a name and three instants. An account
//! enters only after a failure that made a call fall to the lane; it leaves when one real attempt
//! through it gets any answer. While it is down, one ordinary call is let through per interval
//! (30 s, doubling to [`MAX_INTERVAL`] each time it fails again): no timer spends calls with
//! nobody asking.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The first wait before an account is tried again.
pub const FIRST_INTERVAL: Duration = Duration::from_secs(30);
/// The longest wait between two tries.
pub const MAX_INTERVAL: Duration = Duration::from_secs(300);
/// The most accounts remembered: more than a person has, so a full table is a fault, not a state.
const MAX_ACCOUNTS: usize = 64;

/// What time it is, as an offset from any fixed start. A test moves it by hand.
pub trait Clock: Send + Sync {
    fn now(&self) -> Duration;
}

/// The real clock, for the records that keep time ([`Health`], [`crate::hold::Holds`]).
pub struct RealClock(Instant);

impl RealClock {
    pub fn new() -> RealClock {
        RealClock(Instant::now())
    }
}

impl Clock for RealClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

/// What the gateway does with a call for an account, as far as health goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admit {
    /// Not known to be down, or due a try: send it, and say how it went.
    Send,
    /// Known to be down and not due: answer from the lane without trying.
    Skip,
}

struct Down {
    since: Duration,
    next_try: Duration,
    interval: Duration,
    /// The call let through by [`Health::admit`] is in flight: its failure, and only its, backs
    /// the interval off. Calls that were already in flight when the account went down fail too,
    /// and must not.
    probing: bool,
}

pub struct Health {
    clock: Arc<dyn Clock>,
    down: Mutex<HashMap<String, Down>>,
}

impl Default for Health {
    fn default() -> Self {
        Health::with_clock(Arc::new(RealClock::new()))
    }
}

impl Health {
    pub fn with_clock(clock: Arc<dyn Clock>) -> Health {
        Health { clock, down: Mutex::new(HashMap::new()) }
    }

    /// Whether a call for `account` is sent or skipped. A down account that is due is let through
    /// once: the next try is set a full interval ahead before this returns, so concurrent calls
    /// are skipped and at most one real attempt is made per interval.
    pub fn admit(&self, account: &str) -> Admit {
        let now = self.clock.now();
        let Ok(mut down) = self.down.lock() else { return Admit::Send };
        match down.get_mut(account) {
            Some(d) if now < d.next_try => Admit::Skip,
            Some(d) => {
                d.next_try = now + d.interval;
                d.probing = true;
                Admit::Send
            }
            None => Admit::Send,
        }
    }

    /// A call for `account` failed in a way that made it fall: remember it as down. `retry_after`
    /// is the wait the provider named, used as the first interval (at least a second, at most
    /// [`MAX_INTERVAL`]). When it already was down, only the failure of the call let through by
    /// [`Health::admit`] pushes the next try out (the interval doubles, to [`MAX_INTERVAL`]); a
    /// call that was already in flight when it went down leaves the interval alone.
    pub fn failed(&self, account: &str, retry_after: Option<Duration>) {
        let now = self.clock.now();
        let Ok(mut down) = self.down.lock() else { return };
        if let Some(d) = down.get_mut(account) {
            if d.probing {
                d.probing = false;
                d.interval = (d.interval * 2).min(MAX_INTERVAL);
                d.next_try = now + d.interval;
            }
            return;
        }
        let first = retry_after.map_or(FIRST_INTERVAL, |d| d.clamp(Duration::from_secs(1), MAX_INTERVAL));
        if down.len() >= MAX_ACCOUNTS {
            if let Some(oldest) = down.iter().min_by_key(|(_, d)| d.since).map(|(k, _)| k.clone()) {
                down.remove(&oldest);
            }
        }
        down.insert(account.to_string(), Down { since: now, next_try: now + first, interval: first, probing: false });
    }

    /// `account` answered: it is back.
    pub fn recovered(&self, account: &str) {
        if let Ok(mut down) = self.down.lock() {
            down.remove(account);
        }
    }

    /// Whether `account` is remembered as down.
    pub fn is_down(&self, account: &str) -> bool {
        self.down.lock().is_ok_and(|d| d.contains_key(account))
    }
}
