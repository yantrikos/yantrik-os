//! Timers the runtime owns (design/app-framework-2026-10-21.md, §4).
//!
//! An app never holds a `slint::Timer`, keeps one alive through a `_binding`, or forgets one with
//! `mem::forget`. It asks for a timer by [`TimerId`] and gets a command when it fires; arming the
//! same id again restarts it, so a debounce is one line. The runtime stops and drops every timer
//! at shutdown.
//!
//! [`Timers`] is what the runtime needs; [`SlintTimers`] is the real one, and [`ManualTimers`]
//! is a virtual clock for tests, so no test sleeps through a four-second tick.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

/// A timer's name within the app: `TimerId("forecast.tick")`. Stable, so re-arming finds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimerId(pub &'static str);

/// What fires: a command, made fresh each time for a repeating timer.
pub type Fire<C> = Box<dyn Fn() -> C + 'static>;

/// Where a fired command goes: the runtime's queue.
pub type Sink<C> = std::rc::Rc<dyn Fn(C)>;

/// What a timer the core arms fires: one of the app's commands, or a deadline of the framework's
/// own that no command of the app's stands for. The core's timers carry these, so the
/// framework's deadline runs on the same clock as the app's, and a test moves both with
/// `TestApp::advance`.
#[derive(Debug, Clone, PartialEq)]
pub enum Fired<C> {
    Command(C),
    /// A linger's backstop (`fx.linger`): the app has run on without a window for as long as it
    /// asked, and the framework ends it.
    LingerOver,
}

/// The names of the framework's own timers start with this. An app's `fx.every`, `fx.after` or
/// `fx.stop_timer` under one is dropped, with a warning (`Effects`), so no command of the app's
/// can stop, replace or re-arm a deadline the framework keeps.
const FRAMEWORK: &str = "yantrik-app.";

/// The timer a linger's backstop is armed under: the framework's own.
pub(crate) const LINGER: TimerId = TimerId("yantrik-app.linger");

impl TimerId {
    /// Whether this is one of the framework's own timers, which the app may not touch.
    pub fn is_framework(self) -> bool {
        self.0.starts_with(FRAMEWORK)
    }
}

pub trait Timers<C: 'static> {
    fn every(&mut self, id: TimerId, period: Duration, fire: Fire<C>);
    fn after(&mut self, id: TimerId, delay: Duration, fire: Fire<C>);
    fn stop(&mut self, id: TimerId);
    fn stop_all(&mut self);
    fn armed(&self) -> usize;
    /// What this clock reads: the machine's time for real timers, the virtual time for a test's.
    fn instant(&self) -> Instant;
}

/// The runtime's timers: `slint::Timer`s on the UI thread, which stop when dropped.
pub struct SlintTimers<C: 'static> {
    timers: HashMap<TimerId, slint::Timer>,
    sink: Sink<C>,
}

impl<C: 'static> SlintTimers<C> {
    pub fn new(sink: Sink<C>) -> SlintTimers<C> {
        SlintTimers {
            timers: HashMap::new(),
            sink,
        }
    }

    fn arm(&mut self, id: TimerId, mode: slint::TimerMode, interval: Duration, fire: Fire<C>) {
        let timer = slint::Timer::default();
        let sink = self.sink.clone();
        timer.start(mode, interval, move || sink(fire()));
        // Inserting drops the old timer of this id, which stops it: a re-arm is a restart.
        self.timers.insert(id, timer);
    }
}

impl<C: 'static> Timers<C> for SlintTimers<C> {
    fn every(&mut self, id: TimerId, period: Duration, fire: Fire<C>) {
        self.arm(id, slint::TimerMode::Repeated, period, fire);
    }

    fn after(&mut self, id: TimerId, delay: Duration, fire: Fire<C>) {
        self.arm(id, slint::TimerMode::SingleShot, delay, fire);
    }

    fn stop(&mut self, id: TimerId) {
        self.timers.remove(&id);
    }

    fn stop_all(&mut self) {
        self.timers.clear();
    }

    fn armed(&self) -> usize {
        self.timers.len()
    }

    fn instant(&self) -> Instant {
        Instant::now()
    }
}

/// A virtual clock: timers fire when a test advances it, in deadline order.
pub struct ManualTimers<C: 'static> {
    /// The instant the clock reads at zero: when it was made.
    epoch: Instant,
    now: Duration,
    due: BTreeMap<(Duration, TimerId), (Option<Duration>, Fire<C>)>,
    sink: Sink<C>,
}

impl<C: 'static> ManualTimers<C> {
    pub fn new(sink: Sink<C>) -> ManualTimers<C> {
        ManualTimers {
            epoch: Instant::now(),
            now: Duration::ZERO,
            due: BTreeMap::new(),
            sink,
        }
    }

    pub fn now(&self) -> Duration {
        self.now
    }

    /// Moves the clock by `by`, firing every timer that comes due, in order, through the sink; a
    /// repeating timer is re-armed from the time it fired. For a clock behind a `RefCell`, use
    /// [`ManualTimers::fire_next`] so the borrow is released before the command runs.
    pub fn advance(&mut self, by: Duration) {
        let until = self.now + by;
        while let Some(command) = self.fire_next(until) {
            (self.sink)(command);
        }
        self.settle(until);
    }

    /// The next timer due by `until`, fired: the clock moves to its deadline, a repeating one is
    /// re-armed, and its command is returned for the caller to apply, outside any borrow of
    /// this clock, so a command that arms or stops a timer is fine. `None` when nothing is due.
    pub fn fire_next(&mut self, until: Duration) -> Option<C> {
        let ((at, id), _) = self.due.iter().next().map(|(k, v)| (*k, v.0))?;
        if at > until {
            return None;
        }
        let (period, fire) = self.due.remove(&(at, id)).expect("the entry just seen");
        self.now = at;
        let command = fire();
        if let Some(period) = period {
            self.due.insert((at + period, id), (Some(period), fire));
        }
        Some(command)
    }

    /// Moves the clock to `until` once everything due has fired.
    pub fn settle(&mut self, until: Duration) {
        if until > self.now {
            self.now = until;
        }
    }

    fn arm(&mut self, id: TimerId, interval: Duration, period: Option<Duration>, fire: Fire<C>) {
        self.stop(id);
        self.due.insert((self.now + interval, id), (period, fire));
    }
}

impl<C: 'static> Timers<C> for ManualTimers<C> {
    fn every(&mut self, id: TimerId, period: Duration, fire: Fire<C>) {
        self.arm(id, period, Some(period), fire);
    }

    fn after(&mut self, id: TimerId, delay: Duration, fire: Fire<C>) {
        self.arm(id, delay, None, fire);
    }

    fn stop(&mut self, id: TimerId) {
        self.due.retain(|(_, tid), _| *tid != id);
    }

    fn stop_all(&mut self) {
        self.due.clear();
    }

    fn armed(&self) -> usize {
        self.due.len()
    }

    fn instant(&self) -> Instant {
        self.epoch + self.now
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    type Log = Rc<RefCell<Vec<(&'static str, Duration)>>>;

    fn fired() -> (Sink<&'static str>, Log) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let l = log.clone();
        // The time is read back from the test's own clock, through the closure's argument.
        (
            Rc::new(move |c| l.borrow_mut().push((c, Duration::ZERO))),
            log,
        )
    }

    #[test]
    fn a_virtual_clock_fires_in_order_and_a_rearm_is_a_restart() {
        let (sink, log) = fired();
        let mut t = ManualTimers::new(sink);
        t.every(TimerId("tick"), Duration::from_secs(4), Box::new(|| "tick"));
        t.after(
            TimerId("save"),
            Duration::from_millis(750),
            Box::new(|| "save"),
        );
        t.advance(Duration::from_millis(500));
        t.after(
            TimerId("save"),
            Duration::from_millis(750),
            Box::new(|| "save"),
        ); // typed again
        t.advance(Duration::from_millis(500));
        assert!(
            log.borrow().is_empty(),
            "the debounce restarted, so nothing has fired at 1.0 s"
        );
        t.advance(Duration::from_secs(8));
        let names: Vec<_> = log.borrow().iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            vec!["save", "tick", "tick"],
            "save at 1.25 s, ticks at 4 s and 8 s"
        );
        assert_eq!(
            t.armed(),
            1,
            "the one-shot is gone, the repeating one stays"
        );
        t.stop(TimerId("tick"));
        t.advance(Duration::from_secs(10));
        assert_eq!(log.borrow().len(), 3);
    }
}
