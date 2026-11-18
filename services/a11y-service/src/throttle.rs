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
//!   "quiet". Text is therefore never told at once, and not at a time of its own either: it falls
//!   due on a grid, at the multiples of [`TEXT_COALESCE`] counted from the start of the process,
//!   at the first one at least that long after the change (#1257 review, M2). Every target's
//!   text then falls due at the same instants, so when a reader is woken, and where the event
//!   lies among others, says which step it was and nothing about when in it, or how fast, anyone
//!   typed. Text is told at most once per interval whatever the speed of the typing.
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

/// The step of the grid text falls due on, and the shortest a text change is held.
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
    /// When the first text change not yet told falls due: a point of the grid.
    text_due: Option<Instant>,
}

#[derive(Debug)]
pub struct Throttles {
    targets: HashMap<Target, Throttle>,
    /// Where the grid text falls due on starts: when the process did, so nothing a person types
    /// can have set it.
    epoch: Instant,
    /// A change began to be held since this was last asked.
    started: bool,
}

impl Default for Throttles {
    fn default() -> Self {
        Self { targets: HashMap::new(), epoch: Instant::now(), started: false }
    }
}

impl Throttles {
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.targets.len()
    }

    #[cfg(test)]
    pub fn starting_at(epoch: Instant) -> Self {
        Self { epoch, ..Self::default() }
    }

    /// Whether a change began to be held since this was last asked. The watcher reads whether the
    /// person is private afresh when one does: a change held through a private time that was
    /// shorter than the interval between looks must not be told afterwards (#1257 review, L1).
    pub fn take_started(&mut self) -> bool {
        std::mem::take(&mut self.started)
    }

    /// The first point of the grid at least [`TEXT_COALESCE`] after `now`.
    fn grid_after(&self, now: Instant) -> Instant {
        let wanted = (now + TEXT_COALESCE).saturating_duration_since(self.epoch);
        let step = TEXT_COALESCE.as_nanos();
        let steps = wanted.as_nanos().div_ceil(step);
        self.epoch + Duration::from_nanos((steps * step) as u64)
    }

    /// Whether `target` can be tracked, dropping the idle targets if the table is full.
    fn make_room(&mut self, target: &Target) -> bool {
        if self.targets.len() >= MAX_THROTTLED && !self.targets.contains_key(target) {
            self.targets.retain(|_, t| t.held.is_some() || t.text_due.is_some());
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
            let text_due = self.targets.get(&target).and_then(|t| t.text_due);
            self.targets.insert(target, Throttle { last_told: now, held: None, text_due });
            return Some(held);
        }
        let throttle = self.targets.entry(target).or_insert(Throttle { last_told: now, held: None, text_due: None });
        self.started |= throttle.held.is_none();
        mark(throttle.held.get_or_insert_with(Held::default));
        None
    }

    /// One text change for `target`. Never told at once: the first one fixes when it falls due,
    /// on the grid.
    pub fn text(&mut self, target: Target, now: Instant) {
        if !self.make_room(&target) {
            return;
        }
        let due = self.grid_after(now);
        let throttle = self.targets.entry(target).or_insert(Throttle { last_told: now, held: None, text_due: None });
        if throttle.text_due.is_none() {
            throttle.text_due = Some(due);
            self.started = true;
        }
    }

    /// What became due by `now`, per target. The lanes of one target due together are one entry.
    pub fn due(&mut self, now: Instant) -> Vec<(Target, Held)> {
        let mut out = Vec::new();
        for (target, throttle) in &mut self.targets {
            let quick_due = throttle.held.is_some() && now.duration_since(throttle.last_told) >= COALESCE;
            let text_due = throttle.text_due.is_some_and(|due| now >= due);
            if !quick_due && !text_due {
                continue;
            }
            let mut held = if quick_due { throttle.held.take().unwrap_or_default() } else { Held::default() };
            if quick_due {
                throttle.last_told = now;
            }
            if text_due {
                throttle.text_due = None;
                held.text = true;
            }
            out.push((target.clone(), held));
        }
        out
    }

    /// When `due` next has something, if anything is waiting.
    pub fn next_due(&self) -> Option<Instant> {
        let quick = self.targets.values().filter(|t| t.held.is_some()).map(|t| t.last_told + COALESCE);
        let text = self.targets.values().filter_map(|t| t.text_due);
        quick.chain(text).min()
    }

    /// Nothing waiting is ever told: the person went private.
    pub fn forget_held(&mut self) {
        for throttle in self.targets.values_mut() {
            throttle.held = None;
            throttle.text_due = None;
        }
    }

    pub fn remove(&mut self, target: &Target) {
        self.targets.remove(target);
    }
}
