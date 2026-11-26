//! The person's open turns, told to the gateway's "now" (#1407 step 2).
//!
//! The shell opens a turn for a harness when the person sends to it, and closes it where
//! `answered_turns` already learns the turn's end (`Agents::close_turn`). The gateway reads class 0
//! only from an open turn; the companion holds its own work while one is open
//! (`yantrik_ml::asked`), and Mind View says so ([`paused_words`]).
//!
//! A turn that never reports its end is closed after [`MAX_OPEN`], so held work cannot wait forever.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use yantrik_gateway::now::{shared, Turn};

/// The longest a turn is held open without its end being reported.
pub const MAX_OPEN: Duration = Duration::from_secs(15 * 60);

static OPEN: Mutex<Option<HashMap<String, (Turn, Instant)>>> = Mutex::new(None);

/// The person sent to `agent`: its harness has a turn open.
pub fn begin(agent: &str, harness: &str) {
    begin_at(agent, harness, Instant::now());
}

fn begin_at(agent: &str, harness: &str, at: Instant) {
    let expired: Vec<Turn> = {
        let mut open = OPEN.lock().unwrap_or_else(|e| e.into_inner());
        let map = open.get_or_insert_with(HashMap::new);
        let stale: Vec<String> = map.iter().filter(|(_, (_, since))| at.saturating_duration_since(*since) >= MAX_OPEN).map(|(k, _)| k.clone()).collect();
        let mut gone: Vec<Turn> = stale.iter().filter_map(|k| map.remove(k).map(|(t, _)| t)).collect();
        // A second send before the first ended replaces it: still one turn open for the agent.
        gone.extend(map.insert(agent.to_string(), (shared().open_turn(harness), at)).map(|(t, _)| t));
        gone
    };
    // Dropped outside the lock: closing wakes held work.
    drop(expired);
}

/// The turn of `agent` ended (answered, stopped, or the agent was removed).
pub fn end(agent: &str) {
    let ended = OPEN.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.remove(agent));
    drop(ended);
}

/// What Mind View says of work held for an open turn: `None` when nothing is held.
pub fn paused_words() -> Option<String> {
    let held = yantrik_ml::asked::paused();
    (!held.is_empty()).then(|| format!("paused while you're being answered: {}", held.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_that_never_ends_is_closed_by_the_next_begin_after_the_limit() {
        let t0 = Instant::now();
        begin_at("turn-now-a:1", "turn-now-h1", t0);
        assert!(shared().is_open_for("turn-now-h1"));
        begin_at("turn-now-b:1", "turn-now-h2", t0 + MAX_OPEN);
        assert!(!shared().is_open_for("turn-now-h1"), "the stale turn was closed");
        assert!(shared().is_open_for("turn-now-h2"));
        end("turn-now-b:1");
        assert!(!shared().is_open_for("turn-now-h2"));
    }
}
