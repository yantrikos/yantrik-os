//! An account held for what the person asks (#1386). When a call somebody asked (one without the
//! companion's self mark) gets a 429 from an account, the companion's own calls to that account
//! are refused ([`crate::asked::held_for_asked`]) until the window the provider named is over, 60 s
//! when it named none. Per account, in memory, bounded, content-free: a name and one instant.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::health::Clock;

/// The window held when the provider named no wait.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60);
/// The longest window held, whatever the provider names.
pub const MAX_WINDOW: Duration = Duration::from_secs(300);
const MAX_ACCOUNTS: usize = 64;
/// The window a per-minute allowance is counted in.
const MINUTE: Duration = Duration::from_secs(60);

pub struct Holds {
    clock: Arc<dyn Clock>,
    until: Mutex<HashMap<String, Duration>>,
    /// What this gateway sent each `<account>/<model>` in the last minute: (when, estimated
    /// tokens). Counts only, bounded; what [`crate::fit`] sets against a per-minute allowance.
    spent: Mutex<HashMap<String, VecDeque<(Duration, u64)>>>,
}

impl Default for Holds {
    fn default() -> Self {
        Holds::with_clock(Arc::new(crate::health::RealClock::new()))
    }
}

impl Holds {
    pub fn with_clock(clock: Arc<dyn Clock>) -> Holds {
        Holds { clock, until: Mutex::new(HashMap::new()), spent: Mutex::new(HashMap::new()) }
    }

    /// An asked call to `account` met a 429 that named `wait` (or none): hold the account for it.
    /// A hold is only ever extended, never cut short.
    pub fn hold(&self, account: &str, wait: Option<Duration>) {
        let now = self.clock.now();
        let end = now + wait.unwrap_or(DEFAULT_WINDOW).min(MAX_WINDOW);
        let Ok(mut until) = self.until.lock() else { return };
        until.retain(|_, e| *e > now);
        if until.len() >= MAX_ACCOUNTS && !until.contains_key(account) {
            return;
        }
        let e = until.entry(account.to_string()).or_insert(end);
        *e = (*e).max(end);
    }

    /// How long `account` is still held, if it is.
    pub fn remaining(&self, account: &str) -> Option<Duration> {
        let now = self.clock.now();
        let until = self.until.lock().ok()?;
        until.get(account).and_then(|e| e.checked_sub(now)).filter(|d| !d.is_zero())
    }

    /// A request of about `tokens` was sent to `key` (`<account>/<model>`).
    pub fn spend(&self, key: &str, tokens: u64) {
        let now = self.clock.now();
        let Ok(mut spent) = self.spent.lock() else { return };
        spent.retain(|_, q| q.back().is_some_and(|(at, _)| now.saturating_sub(*at) < MINUTE));
        if spent.len() >= MAX_ACCOUNTS && !spent.contains_key(key) {
            return;
        }
        let q = spent.entry(key.to_string()).or_default();
        while q.front().is_some_and(|(at, _)| now.saturating_sub(*at) >= MINUTE) {
            q.pop_front();
        }
        q.push_back((now, tokens));
    }

    /// The real prompt tokens of the last request sent to `key`, in place of its estimate.
    pub fn correct(&self, key: &str, tokens: u64) {
        let Ok(mut spent) = self.spent.lock() else { return };
        if let Some((_, t)) = spent.get_mut(key).and_then(|q| q.back_mut()) {
            *t = tokens;
        }
    }

    /// How long until a request of `need` tokens fits what `key` takes a minute (`per_minute`)
    /// beside what was sent to it in the last minute; zero when it fits now.
    pub fn until_room(&self, key: &str, need: u64, per_minute: u64) -> Duration {
        let now = self.clock.now();
        let Ok(spent) = self.spent.lock() else { return Duration::ZERO };
        let Some(q) = spent.get(key) else { return Duration::ZERO };
        let mut used: u64 = q.iter().filter(|(at, _)| now.saturating_sub(*at) < MINUTE).map(|(_, t)| t).sum();
        for (at, t) in q.iter().filter(|(at, _)| now.saturating_sub(*at) < MINUTE) {
            if used.saturating_add(need) <= per_minute {
                break;
            }
            used = used.saturating_sub(*t);
            if used.saturating_add(need) <= per_minute {
                return (*at + MINUTE).saturating_sub(now);
            }
        }
        if used.saturating_add(need) <= per_minute { Duration::ZERO } else { MINUTE }
    }
}
