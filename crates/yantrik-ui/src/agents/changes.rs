//! Telling a waiting reader the store moved (#1023).
//!
//! The Mind desk (a window of Mind View, in another process) wants to redraw when an agent
//! changes, and not on a timer. Every change to the store raises its revision; the lock the
//! store is held by wakes whoever waits on that revision when a holder lets go having raised it.
//! A reader that only looked raises nothing and wakes nobody, so a busy store costs a quiet
//! reader no more than the one wake per change.

use std::ops::{Deref, DerefMut};
use std::sync::{Condvar, MutexGuard};
use std::time::{Duration, Instant};

use super::{Agents, Store};

/// The store, held. When this is let go and the store's revision is not what it was when the lock
/// was taken, everyone waiting for a newer revision is woken.
pub struct Held<'a> {
    inner: MutexGuard<'a, Store>,
    revision: u64,
    moved: &'a Condvar,
}

impl<'a> Held<'a> {
    pub(super) fn new(inner: MutexGuard<'a, Store>, moved: &'a Condvar) -> Held<'a> {
        let revision = inner.revision();
        Held { inner, revision, moved }
    }
}

impl Deref for Held<'_> {
    type Target = Store;
    fn deref(&self) -> &Store {
        &self.inner
    }
}

impl DerefMut for Held<'_> {
    fn deref_mut(&mut self) -> &mut Store {
        &mut self.inner
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        if self.inner.revision() != self.revision {
            self.moved.notify_all();
        }
    }
}

impl Agents {
    /// The store's revision once it is newer than `seen`, or as it stands after `wait` with no
    /// change. Returns at once when it is newer already. Blocks the calling thread and holds the
    /// store's lock only while it checks: never call it on the UI thread.
    pub fn changed_since(&self, seen: u64, wait: Duration) -> u64 {
        let deadline = Instant::now() + wait;
        let mut held = self.store.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let now = held.revision();
            let left = deadline.saturating_duration_since(Instant::now());
            if now != seen || left.is_zero() {
                return now;
            }
            held = match self.moved.wait_timeout(held, left) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{store, AgentId, AgentMeta};

    /// A reader waiting on the revision is woken by a change and not by a look, and a wait with
    /// nothing happening returns at its end with the revision it had.
    #[test]
    fn a_waiting_reader_wakes_on_a_change_and_not_on_a_look() {
        let agents = store();
        let before = agents.revision();
        // Nothing changes: the wait runs out and says the revision it started with.
        let quiet = std::thread::spawn(move || store().changed_since(before, Duration::from_millis(150)));
        agents.read(|s| s.agents().len());
        let _ = quiet.join().unwrap();

        let seen = agents.revision();
        let woken = std::thread::spawn(move || {
            let began = Instant::now();
            let now = store().changed_since(seen, Duration::from_secs(20));
            (now, began.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        agents.upsert_agent(AgentMeta::new(AgentId::new("desk-wake", AgentId::MAIN), "Wake"));
        let (now, took) = woken.join().unwrap();
        assert!(now > seen, "it came back with the newer revision");
        assert!(took < Duration::from_secs(5), "and at the change, not at the end of its wait: {took:?}");
    }

    /// An answer that is newer already does not wait at all.
    #[test]
    fn a_reader_behind_the_store_is_answered_at_once() {
        let agents = store();
        agents.upsert_agent(AgentMeta::new(AgentId::new("desk-behind", AgentId::MAIN), "Behind"));
        let began = Instant::now();
        let now = agents.changed_since(0, Duration::from_secs(20));
        assert!(now > 0 && began.elapsed() < Duration::from_secs(2));
    }
}
