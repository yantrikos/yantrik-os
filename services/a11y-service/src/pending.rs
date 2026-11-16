//! What the watcher was not able to do yet, and must not forget (#1257 review, H1 and L4).
//!
//! A window announces itself once. If the question that would learn it is refused (its
//! connection has used its share, or everyone has used the total), the announcement is not
//! repeated, and without this it would be lost: the dialog stays out of the model until
//! something unrelated lists the windows. So a refused learn is kept, in a slot of the
//! connection's own, the newest announcement winning, and asked again as soon as the connection
//! may be asked. Whoever else fills their slots, an honest application's is its own.
//!
//! The same slots hold a window that took the front before the model knew it: it is made the
//! front once a listing has found it, if nothing has taken or lost the front since and it is not
//! too old (a person who switched windows meanwhile is not switched back).
//!
//! Bounded: at most [`MAX_SLOTS`] connections have one, and the oldest goes first.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::screen::Key;

/// Connections that may have a slot at the same time.
pub const MAX_SLOTS: usize = 64;

/// How long a window that took the front waits for a listing to find it. Past this the person
/// has probably moved on, and the listing is not a reason to bring it to the front.
pub const FRONT_MAX_AGE: Duration = Duration::from_secs(3);

/// How long a refused question is kept to be asked again.
pub const ASK_MAX_AGE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
struct Slot {
    key: Key,
    /// Take it as the front once it is known.
    activate: bool,
    /// Ask the window what it is, once its connection may be asked.
    ask: bool,
    at: Instant,
}

#[derive(Debug, Default)]
pub struct Pending {
    slots: HashMap<String, Slot>,
}

impl Pending {
    fn put(&mut self, slot: Slot) {
        if self.slots.len() >= MAX_SLOTS && !self.slots.contains_key(&slot.key.0) {
            let oldest = self.slots.iter().min_by_key(|(_, s)| s.at).map(|(name, _)| name.clone());
            if let Some(name) = oldest {
                self.slots.remove(&name);
            }
        }
        self.slots.insert(slot.key.0.clone(), slot);
    }

    /// A window's question was refused: keep it, and whether it is to take the front.
    pub fn refused(&mut self, key: Key, activate: bool, now: Instant) {
        self.put(Slot { key, activate, ask: true, at: now });
    }

    /// A window that is not known took the front: take it as the front once a listing finds it.
    pub fn front_when_found(&mut self, key: Key, now: Instant) {
        self.put(Slot { key, activate: true, ask: false, at: now });
    }

    /// The windows to ask about again, with whether each is to take the front. Those that were
    /// refused too long ago are forgotten.
    pub fn asks_due(&mut self, now: Instant) -> Vec<(Key, bool)> {
        self.slots.retain(|_, s| !s.ask || now.saturating_duration_since(s.at) <= ASK_MAX_AGE);
        let mut asks: Vec<&Slot> = self.slots.values().filter(|s| s.ask).collect();
        asks.sort_by_key(|s| s.at);
        asks.into_iter().map(|s| (s.key.clone(), s.activate)).collect()
    }

    /// The question was answered, or does not need asking: the slot is empty.
    pub fn answered(&mut self, key: &Key) {
        if self.slots.get(&key.0).is_some_and(|s| s.key == *key) {
            self.slots.remove(&key.0);
        }
    }

    pub fn is_asking(&self) -> bool {
        self.slots.values().any(|s| s.ask)
    }

    /// A window took the front: whatever was waiting to take it is older, and is not wanted.
    pub fn front_taken(&mut self) {
        self.slots.retain(|_, s| {
            s.activate = false;
            s.ask
        });
    }

    /// `key` is no longer the front: it is not to be made so.
    pub fn deactivated(&mut self, key: &Key) {
        self.slots.retain(|_, s| {
            if s.key == *key {
                s.activate = false;
            }
            s.ask || s.activate
        });
    }

    /// A listing ran. What to make the front now, oldest first so the newest wins: the windows
    /// that were waiting for it and that it found, and are not too old. What waited and was not
    /// found is forgotten; what is to be asked about stays.
    pub fn after_listing(&mut self, now: Instant, known: impl Fn(&Key) -> bool) -> Vec<Key> {
        let mut front: Vec<(Instant, Key)> = Vec::new();
        self.slots.retain(|_, s| {
            let found = known(&s.key);
            if found && s.activate && now.saturating_duration_since(s.at) <= FRONT_MAX_AGE {
                front.push((s.at, s.key.clone()));
            }
            s.ask && !found
        });
        front.sort_by_key(|(at, _)| *at);
        front.into_iter().map(|(_, key)| key).collect()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.slots.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(connection: &str, n: u32) -> Key {
        (connection.to_string(), format!("/org/a11y/atspi/accessible/{n}"))
    }

    #[test]
    fn a_refused_window_is_kept_and_asked_again_oldest_first() {
        let t = Instant::now();
        let mut pending = Pending::default();
        pending.refused(key(":1.2", 1), true, t + Duration::from_millis(5));
        pending.refused(key(":1.1", 1), false, t);
        assert_eq!(pending.asks_due(t + Duration::from_millis(10)), [(key(":1.1", 1), false), (key(":1.2", 1), true)]);
        pending.answered(&key(":1.1", 1));
        assert_eq!(pending.asks_due(t + Duration::from_millis(10)), [(key(":1.2", 1), true)]);
        assert!(pending.is_asking());
    }

    /// #1257 review, L4: one slot for everyone let a throwaway connection take an honest one's.
    #[test]
    fn a_slot_is_a_connections_own_and_its_newest_announcement_wins() {
        let t = Instant::now();
        let mut pending = Pending::default();
        pending.refused(key(":1.honest", 1), true, t);
        for n in 0..10 {
            pending.refused(key(":1.throwaway", n), true, t + Duration::from_millis(u64::from(n)));
        }
        let asks = pending.asks_due(t + Duration::from_millis(20));
        assert_eq!(asks.len(), 2);
        assert!(asks.contains(&(key(":1.honest", 1), true)));
        assert!(asks.contains(&(key(":1.throwaway", 9), true)), "the newest of its own");
    }

    #[test]
    fn slots_are_bounded_and_the_oldest_goes_first() {
        let t = Instant::now();
        let mut pending = Pending::default();
        for n in 0..MAX_SLOTS as u64 + 10 {
            pending.refused(key(&format!(":1.{n}"), 1), false, t + Duration::from_millis(n));
        }
        assert_eq!(pending.len(), MAX_SLOTS);
        let asks = pending.asks_due(t + Duration::from_secs(1));
        assert!(!asks.iter().any(|(k, _)| k.0 == ":1.0") && asks.iter().any(|(k, _)| k.0 == ":1.73"));
    }

    #[test]
    fn a_refused_question_is_not_kept_for_ever() {
        let t = Instant::now();
        let mut pending = Pending::default();
        pending.refused(key(":1.1", 1), false, t);
        assert!(pending.asks_due(t + ASK_MAX_AGE + Duration::from_millis(1)).is_empty());
        assert!(!pending.is_asking());
    }

    /// #1257 review, L4: a window made front after the listing even if the person had switched.
    #[test]
    fn a_window_waiting_for_the_front_is_not_made_front_after_the_front_moved() {
        let t = Instant::now();
        let mut pending = Pending::default();
        pending.front_when_found(key(":1.1", 1), t);
        pending.front_taken();
        assert!(pending.after_listing(t, |_| true).is_empty(), "the person switched meanwhile");
        pending.front_when_found(key(":1.1", 1), t);
        pending.deactivated(&key(":1.1", 1));
        assert!(pending.after_listing(t, |_| true).is_empty(), "and a window that deactivated is not brought back");
        pending.front_when_found(key(":1.1", 1), t);
        assert!(pending.after_listing(t + FRONT_MAX_AGE + Duration::from_millis(1), |_| true).is_empty(), "and one that waited too long");
    }

    #[test]
    fn what_a_listing_found_takes_the_front_newest_last_and_what_it_did_not_is_forgotten() {
        let t = Instant::now();
        let mut pending = Pending::default();
        pending.front_when_found(key(":1.2", 1), t + Duration::from_millis(5));
        pending.front_when_found(key(":1.1", 1), t);
        pending.front_when_found(key(":1.3", 1), t + Duration::from_millis(9));
        pending.refused(key(":1.4", 1), false, t);
        let front = pending.after_listing(t + Duration::from_millis(20), |k| k.0 != ":1.3" && k.0 != ":1.4");
        assert_eq!(front, [key(":1.1", 1), key(":1.2", 1)]);
        assert_eq!(pending.len(), 1, "only the refused question that is still to be asked stays");
    }
}
