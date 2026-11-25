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
    /// Accounts that refused a call with a 429, whoever made it, until their window is over: no
    /// call is moved to them meanwhile (#1401). Unlike `until`, it never keeps the companion out.
    refused: Mutex<HashMap<String, Duration>>,
    /// When each `<account>/<model>` was sent a request in the last minute.
    requests: Mutex<HashMap<String, VecDeque<Duration>>>,
    /// What this gateway sent each `<account>/<model>` in the last minute: (when, estimated
    /// tokens, the call's key). Counts only, bounded; what [`crate::fit`] sets against a per-minute allowance.
    spent: Mutex<HashMap<String, VecDeque<(Duration, u64, u64)>>>,
    /// What each account has been sent today, against its day limits ([`crate::budget`]).
    pub day: crate::budget::Budget,
}

impl Default for Holds {
    fn default() -> Self {
        Holds::with_clock(Arc::new(crate::health::RealClock::new()))
    }
}

impl Holds {
    pub fn with_clock(clock: Arc<dyn Clock>) -> Holds {
        Holds::with_budget(clock, crate::budget::Budget::in_memory())
    }

    pub fn with_budget(clock: Arc<dyn Clock>, day: crate::budget::Budget) -> Holds {
        Holds { clock, until: Mutex::new(HashMap::new()), refused: Mutex::new(HashMap::new()), requests: Mutex::new(HashMap::new()), spent: Mutex::new(HashMap::new()), day }
    }

    /// Holds whose day ledger is kept in the file at `path` across a restart.
    pub fn with_state(path: std::path::PathBuf) -> Holds {
        Holds::with_budget(Arc::new(crate::health::RealClock::new()), crate::budget::Budget::at(path))
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

    /// `account` refused a call (a 429 naming `wait`, or none): nothing is moved to it until then.
    pub fn refuse(&self, account: &str, wait: Option<Duration>) {
        let now = self.clock.now();
        let end = now + wait.unwrap_or(DEFAULT_WINDOW).min(MAX_WINDOW);
        let Ok(mut refused) = self.refused.lock() else { return };
        refused.retain(|_, e| *e > now);
        if refused.len() >= MAX_ACCOUNTS && !refused.contains_key(account) {
            return;
        }
        let e = refused.entry(account.to_string()).or_insert(end);
        *e = (*e).max(end);
    }

    /// How long `account` still refuses calls, if it does.
    pub fn refusing(&self, account: &str) -> Option<Duration> {
        let now = self.clock.now();
        let refused = self.refused.lock().ok()?;
        refused.get(account).and_then(|e| e.checked_sub(now)).filter(|d| !d.is_zero())
    }

    /// A request was sent to `key` (`<account>/<model>`) now.
    pub fn note_request(&self, key: &str) {
        let now = self.clock.now();
        let Ok(mut requests) = self.requests.lock() else { return };
        requests.retain(|_, q| q.back().is_some_and(|at| now.saturating_sub(*at) < MINUTE));
        if requests.len() >= MAX_ACCOUNTS && !requests.contains_key(key) {
            return;
        }
        requests.entry(key.to_string()).or_default().push_back(now);
    }

    /// The requests sent to `key` in the last minute.
    pub fn requests_last_minute(&self, key: &str) -> u64 {
        let now = self.clock.now();
        let Ok(requests) = self.requests.lock() else { return 0 };
        requests.get(key).map_or(0, |q| q.iter().filter(|at| now.saturating_sub(**at) < MINUTE).count() as u64)
    }

    /// How long `account` is still held, if it is.
    pub fn remaining(&self, account: &str) -> Option<Duration> {
        let now = self.clock.now();
        let until = self.until.lock().ok()?;
        until.get(account).and_then(|e| e.checked_sub(now)).filter(|d| !d.is_zero())
    }

    /// Call `rid`'s request of about `tokens` was sent to `key` (`<account>/<model>`).
    pub fn spend(&self, key: &str, rid: u64, tokens: u64) {
        let now = self.clock.now();
        let Ok(mut spent) = self.spent.lock() else { return };
        spent.retain(|_, q| q.back().is_some_and(|(at, _, _)| now.saturating_sub(*at) < MINUTE));
        if spent.len() >= MAX_ACCOUNTS && !spent.contains_key(key) {
            return;
        }
        let q = spent.entry(key.to_string()).or_default();
        while q.front().is_some_and(|(at, _, _)| now.saturating_sub(*at) >= MINUTE) {
            q.pop_front();
        }
        q.push_back((now, tokens, rid));
    }

    /// The real prompt tokens of call `rid`, sent to `key`, in place of its own estimate: found by
    /// its key, so calls that overlap each correct themselves and no other.
    pub fn correct(&self, key: &str, rid: u64, tokens: u64) {
        let Ok(mut spent) = self.spent.lock() else { return };
        if let Some((_, t, _)) = spent.get_mut(key).and_then(|q| q.iter_mut().find(|(_, _, id)| *id == rid)) {
            *t = tokens;
        }
    }

    /// How long until a request of `need` tokens fits what `key` takes a minute (`per_minute`)
    /// beside what was sent to it in the last minute; zero when it fits now.
    pub fn until_room(&self, key: &str, need: u64, per_minute: u64) -> Duration {
        let now = self.clock.now();
        let Ok(spent) = self.spent.lock() else { return Duration::ZERO };
        let Some(q) = spent.get(key) else { return Duration::ZERO };
        let mut used: u64 = q.iter().filter(|(at, _, _)| now.saturating_sub(*at) < MINUTE).map(|(_, t, _)| t).sum();
        for (at, t, _) in q.iter().filter(|(at, _, _)| now.saturating_sub(*at) < MINUTE) {
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
