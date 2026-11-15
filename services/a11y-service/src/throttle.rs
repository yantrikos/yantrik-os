//! How often changes inside one window are told (#1256, #1257 review, H5 and N1).
//!
//! Accessibility is a firehose, and a reader wants to hear that a window is changing, not every
//! change. Each target (a window, or an application whose window is not known) has two lanes:
//!
//! - **Quick**: children, names and titles. The first change in a quiet target is told at once, so
//!   a reader learns within milliseconds that something changed, and whatever follows inside
//!   [`COALESCE`] is one more event.
//! - **Text**: one text-changed signal is one keystroke, so the quick lane would tell a person's
//!   typing key by key, with the time between keys: at 300 ms a key, every key finds the window
//!   "quiet". Text is therefore never told at once. The first change is held for
//!   [`TEXT_COALESCE`], and the next one starts a new wait, so text is told at most once per
//!   interval whatever the speed of the typing.
//!
//! Bounded: at most [`MAX_THROTTLED`] targets are tracked. Past that, and with none of them idle, a
//! quick change is told at once and untracked (a little early, never withheld) and a text change
//! for a target not yet tracked is dropped; that takes more than [`MAX_THROTTLED`] targets with
//! text waiting inside one interval, which is more than the model holds.
//!
//! Pure: no clock of its own, so every rule has a test.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::screen::Key;

/// The longest children, name and title changes are held back to be told as one.
pub const COALESCE: Duration = Duration::from_millis(150);

/// How long a text change is held, and so the shortest time between two "text is changing".
pub const TEXT_COALESCE: Duration = Duration::from_secs(2);

/// Targets remembered at most. Past this the idle ones are dropped: the cost is one change told a
/// little early, never one withheld.
pub const MAX_THROTTLED: usize = 512;

/// Who a held change belongs to: a window, or an application whose window is not known.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    Window(Key),
    App(String),
}

/// Changes in one target not yet told.
#[derive(Debug, Default)]
pub struct Held {
    pub text: bool,
    pub children: bool,
    pub name: bool,
    pub renamed: bool,
}

#[derive(Debug)]
struct Throttle {
    /// When the quick lane last spoke.
    last_told: Instant,
    held: Option<Held>,
    /// When the first text change not yet told was seen.
    text_since: Option<Instant>,
}

#[derive(Debug, Default)]
pub struct Throttles {
    targets: HashMap<Target, Throttle>,
}

impl Throttles {
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.targets.len()
    }

    /// Whether `target` can be tracked, dropping the idle targets if the table is full.
    fn make_room(&mut self, target: &Target) -> bool {
        if self.targets.len() >= MAX_THROTTLED && !self.targets.contains_key(target) {
            self.targets.retain(|_, t| t.held.is_some() || t.text_since.is_some());
        }
        self.targets.len() < MAX_THROTTLED || self.targets.contains_key(target)
    }

    /// One quick change for `target`: handed back to be told at once when the target has been
    /// quiet, held otherwise.
    pub fn quick(&mut self, target: Target, now: Instant, mark: impl FnOnce(&mut Held)) -> Option<Held> {
        let room = self.make_room(&target);
        let quiet = self.targets.get(&target).is_none_or(|t| t.held.is_none() && now.duration_since(t.last_told) >= COALESCE);
        if quiet || !room {
            let mut held = Held::default();
            mark(&mut held);
            if !room {
                return Some(held);
            }
            let text_since = self.targets.get(&target).and_then(|t| t.text_since);
            self.targets.insert(target, Throttle { last_told: now, held: None, text_since });
            return Some(held);
        }
        let throttle = self.targets.entry(target).or_insert(Throttle { last_told: now, held: None, text_since: None });
        mark(throttle.held.get_or_insert_with(Held::default));
        None
    }

    /// One text change for `target`. Never told at once: the first one starts the wait.
    pub fn text(&mut self, target: Target, now: Instant) {
        if !self.make_room(&target) {
            return;
        }
        let throttle = self.targets.entry(target).or_insert(Throttle { last_told: now, held: None, text_since: None });
        throttle.text_since.get_or_insert(now);
    }

    /// What became due by `now`, per target. The lanes of one target due together are one entry.
    pub fn due(&mut self, now: Instant) -> Vec<(Target, Held)> {
        let mut out = Vec::new();
        for (target, throttle) in &mut self.targets {
            let quick_due = throttle.held.is_some() && now.duration_since(throttle.last_told) >= COALESCE;
            let text_due = throttle.text_since.is_some_and(|since| now.duration_since(since) >= TEXT_COALESCE);
            if !quick_due && !text_due {
                continue;
            }
            let mut held = if quick_due { throttle.held.take().unwrap_or_default() } else { Held::default() };
            if quick_due {
                throttle.last_told = now;
            }
            if text_due {
                throttle.text_since = None;
                held.text = true;
            }
            out.push((target.clone(), held));
        }
        out
    }

    /// When `due` next has something, if anything is waiting.
    pub fn next_due(&self) -> Option<Instant> {
        let quick = self.targets.values().filter(|t| t.held.is_some()).map(|t| t.last_told + COALESCE);
        let text = self.targets.values().filter_map(|t| t.text_since).map(|since| since + TEXT_COALESCE);
        quick.chain(text).min()
    }

    /// Nothing waiting is ever told: the person went private.
    pub fn forget_held(&mut self) {
        for throttle in self.targets.values_mut() {
            throttle.held = None;
            throttle.text_since = None;
        }
    }

    pub fn remove(&mut self, target: &Target) {
        self.targets.remove(target);
    }
}
