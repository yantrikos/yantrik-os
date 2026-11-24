//! An account held for what the person asks (#1386). When a call somebody asked (one without the
//! companion's self mark) gets a 429 from an account, the companion's own calls to that account
//! are refused ([`crate::asked::held_for_asked`]) until the window the provider named is over, 60 s
//! when it named none. Per account, in memory, bounded, content-free: a name and one instant.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::health::Clock;

/// The window held when the provider named no wait.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60);
/// The longest window held, whatever the provider names.
pub const MAX_WINDOW: Duration = Duration::from_secs(300);
const MAX_ACCOUNTS: usize = 64;

pub struct Holds {
    clock: Arc<dyn Clock>,
    until: Mutex<HashMap<String, Duration>>,
}

impl Default for Holds {
    fn default() -> Self {
        Holds::with_clock(Arc::new(crate::health::RealClock::new()))
    }
}

impl Holds {
    pub fn with_clock(clock: Arc<dyn Clock>) -> Holds {
        Holds { clock, until: Mutex::new(HashMap::new()) }
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
}
