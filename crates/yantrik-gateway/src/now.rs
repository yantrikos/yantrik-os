//! The gateway's "now" (#1407, step 2): one read-only picture of what is happening, kept where
//! every call is seen, read by the companion, Mind View and the Lens. Counts only, never content.
//!
//! - the open turns, by harness: the shell opens one when the person sends to a harness and closes
//!   it when the answer is shown ([`Now::open_turn`]);
//! - each `<account>/<model>`'s minute: tokens used, room, and when room comes back
//!   ([`Now::snapshot`]).
//!
//! A call's class is derived here, never claimed by a caller ([`Now::class_of`]): `Interactive`
//! only from a harness with an open turn, `Own` for the companion's mark, `Asked` for the rest. A
//! header can only lower a class.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::hold::Holds;

/// Who a call is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// A call inside a turn the person has open with this harness.
    Interactive,
    /// Anything else: somebody asked for it. The default.
    Asked,
    /// The companion started it by itself.
    Own,
}

impl Class {
    /// The word the call log keeps.
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Interactive => "interactive",
            Class::Asked => "asked",
            Class::Own => "own",
        }
    }
}

/// One `<account>/<model>`'s minute, in counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyMinute {
    pub key: String,
    pub used: u64,
    /// Tokens left this minute; `None` when the model has no per-minute allowance.
    pub room: Option<u64>,
    /// Seconds until a request of the model's whole minute's worth of room could be taken; zero when there is room.
    pub room_back_in: u64,
}

/// What [`Now::snapshot`] returns.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Snapshot {
    /// Harness id and how many turns it has open.
    pub turns: Vec<(String, u32)>,
    pub keys: Vec<KeyMinute>,
}

type Wake = Box<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub struct Now {
    turns: Mutex<HashMap<String, u32>>,
    on_close: Mutex<Vec<Wake>>,
}

/// An open turn. Dropping it closes the turn and wakes what was waiting, once.
#[must_use = "the turn closes when this is dropped"]
pub struct Turn {
    now: Arc<Now>,
    harness: String,
}

impl Drop for Turn {
    fn drop(&mut self) {
        self.now.close(&self.harness);
    }
}

impl Now {
    /// The harness has a turn open until the guard drops.
    pub fn open_turn(self: &Arc<Now>, harness: &str) -> Turn {
        *self.turns.lock().unwrap_or_else(|e| e.into_inner()).entry(harness.to_string()).or_insert(0) += 1;
        Turn { now: self.clone(), harness: harness.to_string() }
    }

    fn close(&self, harness: &str) {
        let last = {
            let mut turns = self.turns.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(n) = turns.get_mut(harness) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    turns.remove(harness);
                }
            }
            turns.is_empty()
        };
        // One wake-up when the last open turn closes; no polling.
        if last {
            for wake in self.on_close.lock().unwrap_or_else(|e| e.into_inner()).iter() {
                wake();
            }
        }
    }

    /// Call `wake` each time the last open turn closes.
    pub fn on_close(&self, wake: impl Fn() + Send + Sync + 'static) {
        self.on_close.lock().unwrap_or_else(|e| e.into_inner()).push(Box::new(wake));
    }

    /// The harnesses with a turn open, and how many each, sorted by name.
    pub fn turns(&self) -> Vec<(String, u32)> {
        let mut turns: Vec<(String, u32)> = self.turns.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(h, n)| (h.clone(), *n)).collect();
        turns.sort();
        turns
    }

    pub fn is_open_for(&self, harness: &str) -> bool {
        self.turns.lock().unwrap_or_else(|e| e.into_inner()).contains_key(harness)
    }

    pub fn any_open(&self) -> bool {
        !self.turns.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    /// Whether a turn of some harness other than `harness` is open.
    pub fn any_open_besides(&self, harness: &str) -> bool {
        self.turns.lock().unwrap_or_else(|e| e.into_inner()).keys().any(|h| h != harness)
    }

    /// The class of a call from `harness`. `marked` is whether it carries the companion's mark.
    /// `Interactive` needs an open turn for that harness and no mark; a mark only lowers.
    pub fn class_of(&self, harness: &str, companion: bool, marked: bool) -> Class {
        match (marked, companion) {
            (true, true) => Class::Own,
            (true, false) => Class::Asked,
            (false, _) if self.is_open_for(harness) => Class::Interactive,
            _ => Class::Asked,
        }
    }

    /// The open turns, and each of `keys` (`<account>/<model>` and its tokens a minute, where it
    /// has one) as `holds` sees it.
    pub fn snapshot(&self, holds: &Holds, keys: &[(String, Option<u64>)]) -> Snapshot {
        let mut turns: Vec<(String, u32)> = self.turns.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(h, n)| (h.clone(), *n)).collect();
        turns.sort();
        let keys = keys
            .iter()
            .map(|(key, per_minute)| {
                let used = holds.used_last_minute(key);
                KeyMinute {
                    key: key.clone(),
                    used,
                    room: per_minute.map(|p| p.saturating_sub(used)),
                    room_back_in: per_minute.map_or(0, |p| holds.until_room(key, p, p).as_secs()),
                }
            })
            .collect();
        Snapshot { turns, keys }
    }
}

/// The one `Now` of this process: the shell's turns and the gateway it hosts share it.
pub fn shared() -> Arc<Now> {
    static NOW: OnceLock<Arc<Now>> = OnceLock::new();
    NOW.get_or_init(Default::default).clone()
}

/// How long a minute of room takes to come back, as words for a surface: "back in 12 s".
pub fn back_in_words(d: Duration) -> String {
    if d.is_zero() { "now".into() } else { format!("in {} s", d.as_secs().max(1)) }
}
