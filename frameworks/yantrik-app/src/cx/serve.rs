//! The start of the instance, and what keeps it running with no window (`crate::service`, design
//! #978 step 1). Part of [`Core`]; here so the core's own file stays readable.
//!
//! An instance starts one of two ways. The person's launch runs `App::start_service` and then
//! `App::start`, and shows the first window. An agent's service start runs `start_service` alone
//! and shows nothing: the app's store is loaded and its intents and `describe` answer, and the
//! person's start (`App::start`: recovery, anything they see) waits for their first window, which
//! the person's launch opens through `present` or `launch`.
//!
//! An instance with no window runs on while agents use it and ends [`crate::service::IDLE`]
//! after the last time one did. That holds when it was started for an agent, and when the person
//! closed the last window after an agent had used it: never does a window of the person's become
//! a window anywhere else, and the process never moves display.

use std::cell::{Cell, RefCell};

use super::Core;
use crate::app::{App, ExitReason};
use crate::rail::RailCommand;
use crate::report::Reporter;
use crate::timers::{Fired, IDLE};

/// What the core keeps of how the instance started and who has used it.
#[derive(Default)]
pub(super) struct Serve {
    /// Started for an agent with no window: it has none until the person's launch asks for one.
    windowless: Cell<bool>,
    /// An agent has used the instance: it outlives its last window while they do.
    used: Cell<bool>,
    /// The person's start (`App::start`) has run.
    person_started: Cell<bool>,
    /// Where the count of the person's windows is reported to the shell, under the runner.
    reporter: RefCell<Option<Reporter>>,
}

impl<A: App> Core<A> {
    /// The person's start: `App::start_service`, then `App::start`.
    pub fn start(&self) {
        self.run_start(A::start_service, false);
        self.person_arrives();
        self.report_windows();
    }

    /// A service start, for an agent: `App::start_service` alone, with no window shown. The
    /// instance ends [`crate::service::IDLE`] after this unless an agent uses it.
    pub fn start_service(&self) {
        self.serve.windowless.set(true);
        // The first window exists but is not shown, and the core does not count it open.
        self.windows.borrow_mut().first_open = false;
        self.run_start(A::start_service, false);
        self.arm_idle();
        self.report_windows();
    }

    /// The person's window is coming (their launch, handed over, or a start of their own): what
    /// waited for it runs, once.
    pub fn person_arrives(&self) {
        if !self.serve.person_started.replace(true) {
            self.run_start(A::start, true);
        }
    }

    /// Whether the person's start has run.
    pub fn person_has_started(&self) -> bool {
        self.serve.person_started.get()
    }

    fn run_start(&self, start: fn(&A::Model, &mut crate::effects::Effects<A>), rail: bool) {
        self.fresh_wake();
        let mut fx = self.effects(None).starting();
        start(&self.model.borrow(), &mut fx);
        let (requests, changed, next) = fx.into_parts();
        self.next_job.set(next);
        self.applying.set(true);
        self.current.set(None);
        self.commit(requests);
        if !changed.is_empty() {
            self.render_windows(&changed);
        }
        // An app with a rail: the first refresh now. Later ones follow the model's changes and
        // the Mind's reach; there is no tick (see `rail`). It asks the Mind, so it is the
        // person's.
        if rail {
            if let Some(refresh) = A::rail(RailCommand::Refresh) {
                self.queue(None, refresh);
            }
        }
        self.applying.set(false);
        self.drain_pending();
    }

    /// An agent called the app (`describe` or an intent). While no window is open the idle
    /// wait starts over, and the instance outlives its last window while agents use it.
    pub fn agent_used_if(&self, agent: bool) {
        if !agent {
            return;
        }
        self.serve.used.set(true);
        if self.windows_open().is_empty() {
            self.arm_idle();
        }
    }

    /// Whether the process runs on with no window: it was started for an agent, or one has used
    /// it. The last window closing does not end it then; the idle wait does.
    pub(super) fn keeps_serving(&self) -> bool {
        self.exit.get().is_none() && (self.serve.windowless.get() || self.serve.used.get())
    }

    /// The last window closed and the instance keeps serving: nothing ends it but the idle wait.
    pub(super) fn serve_windowless(&self) {
        tracing::info!(app = self.id, "the last window closed; running on without one while agents use it");
        self.arm_idle();
    }

    /// The idle wait, started over.
    pub(super) fn arm_idle(&self) {
        self.timers.borrow_mut().after(IDLE, crate::service::IDLE, Box::new(|| Fired::IdleOver));
    }

    /// A window of the person's is open: the idle wait is over.
    pub(super) fn stop_idle(&self) {
        self.timers.borrow_mut().stop(IDLE);
    }

    /// The idle wait passed. A late fire, with a window open or the app ending, does nothing.
    pub(super) fn idle_over(&self) {
        if !self.windows_open().is_empty() || self.exit.get().is_some() {
            return;
        }
        tracing::info!(app = self.id, "no agent used the app for a while and no window is open; ending");
        self.request_exit(ExitReason::LastWindowClosed);
    }

    /// Where whose windows are open is reported to the shell, under the runner. Written at once.
    pub fn report_windows_to(&self, reporter: Reporter) {
        *self.serve.reporter.borrow_mut() = Some(reporter);
        self.report_windows();
    }

    /// The count of the person's windows, to the shell, when it changed.
    pub(super) fn report_windows(&self) {
        if let Some(reporter) = self.serve.reporter.borrow().as_ref() {
            reporter.write(self.windows_open().len() as u32);
        }
    }
}
