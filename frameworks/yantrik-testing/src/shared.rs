//! What the core is given under test, shared with the test on the one UI thread: the manual
//! executor, the virtual clock and the manual persistence lane.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use yantrik_app::jobs::{Cancel, Deliver, Executor, Job, JobId, Manual};
use yantrik_app::persist::{Lane, ManualLane, Write};
use yantrik_app::timers::{Fire, Fired, ManualTimers, TimerId, Timers};

/// The manual executor, shared between the core and the test (both on the UI thread).
#[derive(Clone)]
pub(crate) struct SharedJobs<C: Send + 'static>(pub(crate) Arc<Mutex<Manual<C>>>);

impl<C: Send + 'static> Executor<C> for SharedJobs<C> {
    fn spawn(&mut self, id: JobId, cancel: Cancel, job: Job<C>, deliver: Arc<dyn Deliver<C>>) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .spawn(id, cancel, job, deliver);
    }

    fn running(&self) -> usize {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).running()
    }

    fn drain(&self, grace: Duration) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain(grace)
    }
}

/// The virtual clock, shared between the core and the test.
pub(crate) type Clock<C> = Rc<RefCell<ManualTimers<C>>>;
/// The clock an app's core runs on: its commands' timers, and the framework's own (`Fired`).
pub(crate) type AppClock<C> = Clock<Fired<C>>;

pub(crate) struct SharedTimers<C: 'static>(pub(crate) Clock<C>);

impl<C: 'static> Timers<C> for SharedTimers<C> {
    fn every(&mut self, id: TimerId, period: Duration, fire: Fire<C>) {
        self.0.borrow_mut().every(id, period, fire);
    }

    fn after(&mut self, id: TimerId, delay: Duration, fire: Fire<C>) {
        self.0.borrow_mut().after(id, delay, fire);
    }

    fn stop(&mut self, id: TimerId) {
        self.0.borrow_mut().stop(id);
    }

    fn stop_all(&mut self) {
        self.0.borrow_mut().stop_all();
    }

    fn armed(&self) -> usize {
        self.0.borrow().armed()
    }
}

/// The manual persistence lane, shared between the core and the test.
#[derive(Clone)]
pub(crate) struct SharedLane(pub(crate) Arc<ManualLane>);

impl Lane for SharedLane {
    fn write(&mut self, write: Write) {
        // `ManualLane::write` takes `&mut`, but its queue is behind a lock: the shared handle
        // may push through it.
        self.0.push(write);
    }

    fn waiting(&self) -> usize {
        self.0.waiting()
    }

    fn drain(&self, grace: Duration) -> bool {
        self.0.drain(grace)
    }
}
