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

/// When the person's last turn ended (or was given up at [`MAX_OPEN`]): the idle lock counts from here.
static LAST_END: Mutex<Option<Instant>> = Mutex::new(None);

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
    if ended.is_some() {
        *LAST_END.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    }
    drop(ended);
}

/// What the idle lock may count (#1430): a person who asked and waits is idle, not away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    /// A turn is open (and younger than [`MAX_OPEN`]): the idle lock does not fire.
    Open,
    /// No turn holds it; the idle count starts again at this instant (`None`: no turn since start).
    Since(Option<Instant>),
}

/// Whether an open turn holds off the idle lock at `now`, else where its count restarts.
pub fn hold_at(now: Instant) -> Hold {
    let mut since_end = *LAST_END.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = OPEN.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        for (_, began) in map.values() {
            if now.saturating_duration_since(*began) < MAX_OPEN {
                return Hold::Open;
            }
            // Never held past MAX_OPEN: the count restarts where the hold gave out.
            let gave_out = *began + MAX_OPEN;
            since_end = Some(since_end.map_or(gave_out, |t| t.max(gave_out)));
        }
    }
    Hold::Since(since_end)
}

/// [`counted_idle`] at this moment.
pub fn idle_for_lock(idle_secs: u64) -> Option<u64> {
    let now = Instant::now();
    counted_idle(idle_secs, hold_at(now), now)
}

/// The idle seconds the auto-lock may count: `None` while a turn holds it, else the compositor's
/// `idle_secs` bounded by the time since the turn's end. A manual lock does not ask this.
pub fn counted_idle(idle_secs: u64, hold: Hold, now: Instant) -> Option<u64> {
    match hold {
        Hold::Open => None,
        Hold::Since(None) => Some(idle_secs),
        Hold::Since(Some(t)) => Some(idle_secs.min(now.saturating_duration_since(t).as_secs())),
    }
}

/// The names of the work held for an open turn (names only), sorted and without repeats.
pub fn paused_names() -> Vec<String> {
    let mut held = yantrik_ml::asked::paused();
    held.sort();
    held.dedup();
    held
}

/// What Mind View says of work held for an open turn: `None` when nothing is held.
pub fn paused_words() -> Option<String> {
    words_of(&paused_names())
}

/// The one line for `names`, `None` for none. The desk draws the same words.
pub fn words_of(names: &[String]) -> Option<String> {
    (!names.is_empty()).then(|| format!("Paused while you're being answered: {}", names.join(", ")))
}

/// `describe shell`'s `now`: the harnesses with a turn open and the work paused, counts and names
/// only, never content.
pub fn for_describe() -> serde_json::Value {
    let open: Vec<serde_json::Value> = shared().turns().iter().map(|(h, n)| serde_json::json!({ "harness": h, "turns": n })).collect();
    serde_json::json!({ "open_turns": open, "paused": paused_names() })
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

    #[test]
    fn an_open_turn_holds_the_idle_lock_and_the_count_restarts_at_its_end() {
        let t0 = Instant::now();
        begin_at("turn-now-i:1", "turn-now-hi", t0);
        let held = hold_at(t0 + Duration::from_secs(400));
        assert_eq!(held, Hold::Open);
        assert_eq!(counted_idle(400, held, t0), None, "300 s idle, and no lock while the turn is open");
        end("turn-now-i:1");
        let now = Instant::now();
        let after = hold_at(now);
        assert!(matches!(after, Hold::Since(Some(_))));
        // The person was idle 400 s, but the count began at the turn's end.
        assert_eq!(counted_idle(400, after, now), Some(0));
        assert_eq!(counted_idle(400, after, now + Duration::from_secs(300)), Some(300));
    }

    #[test]
    fn the_hold_never_outlasts_max_open() {
        let t0 = Instant::now();
        begin_at("turn-now-m:1", "turn-now-hm", t0);
        assert_eq!(hold_at(t0 + MAX_OPEN - Duration::from_secs(1)), Hold::Open);
        let late = t0 + MAX_OPEN + Duration::from_secs(10);
        let h = hold_at(late);
        assert!(matches!(h, Hold::Since(Some(_))), "{h:?}");
        assert!(counted_idle(10_000, h, late).unwrap() <= 10, "counts from where the hold gave out");
        end("turn-now-m:1");
    }

    #[test]
    fn describe_names_open_harnesses_and_paused_work_and_nothing_else() {
        begin("turn-now-d:1", "turn-now-hd");
        let d = for_describe();
        assert!(d["open_turns"].as_array().unwrap().iter().any(|t| t["harness"] == "turn-now-hd" && t["turns"] == 1), "{d}");
        assert!(d["paused"].is_array());
        end("turn-now-d:1");
        let d = for_describe();
        assert!(!d["open_turns"].as_array().unwrap().iter().any(|t| t["harness"] == "turn-now-hd"), "{d}");
    }

    #[test]
    fn the_line_is_one_sentence_of_names_and_goes_when_nothing_is_held() {
        assert_eq!(words_of(&[]), None);
        assert_eq!(words_of(&["learning".into(), "narrative".into()]).as_deref(), Some("Paused while you're being answered: learning, narrative"));
    }
}
