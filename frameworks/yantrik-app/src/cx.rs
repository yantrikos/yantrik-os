//! The UI-thread core: the one place the model lives, and how commands reach it
//! (design/app-framework-2026-10-21.md, §§2.3, 3, 4).
//!
//! [`Core`] owns the model, the window, the timers, the job table, the open settlements and the
//! settings store. It applies a command by calling `update`, committing the effects it asked
//! for, and calling `render` for the parts that changed. Nothing else touches the model. [`Cx`]
//! is the handle a view uses to bind callbacks, and [`Core::post`] is the `Send` endpoint every
//! other thread uses.
//!
//! Re-entrancy: a command raised while one is being applied (from `then`, or a Slint callback
//! fired by `render`) is queued and applied after the current one's render, never nested. One
//! drain applies at most [`PENDING_PER_DRAIN`] queued commands before yielding to the event
//! loop, so a `then` cycle cannot freeze drawing.
//!
//! Capacity: commands share the bounded inbox and a flood of them is refused; a job's outcome
//! does not. It goes through the inbox's reserved lane, bounded by the pool itself, so saturation
//! never loses a result and never leaves a key marked running (§4).
//!
//! Invocations: a command that came through the surface runs under an [`Invocation`]; the jobs
//! and `then` chains it starts carry it, so a late result, a panic or a replacement is known to
//! belong to it, and an intent that settles explicitly is answered exactly once, and only when
//! nothing of it is still running or queued.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::{App, Changed, Event, ExitReason};
use crate::effects::{Effects, Request};
use crate::intents::{Invocation, InvocationId, Origin};
use crate::jobs::{Cancel, Deliver, Executor, Finished, JobId, JobKey};
use crate::persist::{Lane, Report, Written};
use crate::problems;
use crate::queue::{self, Inbox, Post, Wake};
use crate::refused::Refused;
use crate::settings::Store;
use crate::timers::{Fired, Timers};
use crate::rail::{self, RailCommand};
use crate::watch::Seen;
use yantrik_surface::LocalRegistry;

mod events;
mod handle;
mod jobs_table;
mod linger;
mod persisted;
mod quitting;
mod rail_hooks;
mod settlement;
mod windows;
pub use handle::{Cx, UiRef};
pub use linger::MAX_LINGER;
pub(crate) use windows::next_window_id;
use settlement::Open;

/// How many queued (`then`) commands one drain applies before the rest wait for the next wake.
pub const PENDING_PER_DRAIN: usize = queue::CAPACITY;

/// What reaches the core from any thread.
pub enum Inbound<A: App> {
    Command(A::Command),
    Finished(Finished<A::Command>),
    Event(Event<A::Settings>),
    /// The persistence lane finished a write.
    Written(Written),
    /// The housekeeping thread saw a file change.
    Seen(Seen),
    /// The signal thread heard a signal that asks the process to end.
    Signal(crate::quit::Signal),
    /// `quit::SIGNAL_DEADLINE` passed since the first signal.
    Overdue(crate::quit::Signal),
    /// The rest of a drain that reached [`PENDING_PER_DRAIN`]: posted by the core to itself, and
    /// by nothing else (the token cannot be made outside this crate).
    Continue(Continuation),
}

/// The core's own token for [`Inbound::Continue`].
pub struct Continuation(());

/// A job the core is waiting on.
struct Waiting<C> {
    cancel: Cancel,
    key: Option<JobKey>,
    invocation: Option<Invocation>,
    /// Applied if the job ends without a result (a panic, a refusal), so the app hears of it.
    failed: Option<C>,
}

/// The core, on the UI thread. One per app process; a test makes its own.
pub struct Core<A: App> {
    id: &'static str,
    model: RefCell<A::Model>,
    ui: A::Ui,
    timers: RefCell<Box<dyn Timers<Fired<A::Command>>>>,
    executor: RefCell<Box<dyn Executor<A::Command>>>,
    settings: RefCell<Store<A::Settings>>,
    lane: RefCell<Box<dyn Lane>>,
    paths: crate::paths::Paths,
    /// Each draft's latest revision, so a write's outcome is known to be the latest or not.
    drafts: RefCell<HashMap<String, u64>>,
    jobs: RefCell<HashMap<JobId, Waiting<A::Command>>>,
    by_key: RefCell<HashMap<JobKey, JobId>>,
    next_job: Cell<u64>,
    next_invocation: Cell<u64>,
    open: RefCell<HashMap<InvocationId, Open<A>>>,
    current: Cell<Option<Invocation>>,
    post: Post<Inbound<A>>,
    inbox: Inbox<Inbound<A>>,
    /// Queued commands, each with the invocation it runs under and whether the rail's apply
    /// queued it (its outcome is then the rail's `Applied`).
    pending: RefCell<VecDeque<(Option<Invocation>, A::Command, bool)>>,
    /// Set while the rail's apply runs through the surface: what is queued meanwhile is its.
    rail_applying: Cell<bool>,
    rail_queued: Cell<bool>,
    applying: Cell<bool>,
    /// Queued commands this wake may still apply: one [`PENDING_PER_DRAIN`] per wake, shared by
    /// every drain within it, so a flood is bounded per wake and not per call.
    budget: Cell<usize>,
    /// Inside [`Core::drain`]: a `run` from within it draws on the wake's budget, not a fresh one.
    in_wake: Cell<bool>,
    /// Whether a `Continue` is already in the inbox: at most one is ever outstanding.
    continue_posted: Cell<bool>,
    last_refusal: RefCell<Option<Refused>>,
    exit: Cell<Option<ExitReason>>,
    /// The logout raised, once: what asked for it and when (`quitting`).
    logout: Cell<Option<crate::quit::LoggedOut>>,
    /// Running on with no window since the last one closed, by `fx.linger` (`cx/linger.rs`).
    linger: Cell<Option<linger::Lingering>>,
    /// The windows beyond the first, and whether the first is open (`cx/windows.rs`).
    windows: RefCell<windows::Windows<A::Ui>>,
    /// This core, weakly: a window opened later is wired through a `Cx` made from it.
    me: Weak<Core<A>>,
    /// The app's served surface, once it is: the rail applies a proposal through it.
    registry: RefCell<Option<Rc<LocalRegistry>>>,
    /// The documents' files the watch thread looks at (`fx.watch`).
    watched: crate::documents::Watched,
}

/// Delivers a finished job to the core's queue.
struct PostDeliver<A: App>(Post<Inbound<A>>);

impl<A: App> Deliver<A::Command> for PostDeliver<A> {
    fn deliver(&self, finished: Finished<A::Command>) {
        // The reserved lane: a full inbox of commands cannot lose a result. A late result after
        // shutdown is dropped, as the design says it must be.
        let _ = self.0.post_control(Inbound::Finished(finished));
    }
}

/// Makes the timers over the sink that applies a fired command.
pub type MakeTimers<C> = Box<dyn FnOnce(crate::timers::Sink<C>) -> Box<dyn Timers<C>>>;
/// Makes the persistence lane over where its outcomes go.
pub type MakeLane = Box<dyn FnOnce(Arc<dyn Report>) -> Box<dyn Lane>>;

/// What the core is built over, besides the model and the window.
pub struct Parts<A: App> {
    /// The job pool.
    pub executor: Box<dyn Executor<A::Command>>,
    pub timers: MakeTimers<Fired<A::Command>>,
    pub lane: MakeLane,
    /// The settings as opened for the boot.
    pub settings: Store<A::Settings>,
    /// Where the app's files are: the drafts go under the state dir.
    pub paths: crate::paths::Paths,
    /// How a post from another thread gets the UI thread to call [`Core::drain`].
    pub wake: Wake,
}

impl<A: App> Core<A> {
    /// A core over `model` and `ui`, with the parts it is given.
    pub fn new(model: A::Model, ui: A::Ui, parts: Parts<A>) -> Rc<Core<A>> {
        let (post, inbox) = queue::bounded(queue::CAPACITY, parts.wake);
        let report: Arc<dyn Report> = Arc::new(persisted::PostWritten(post.clone()));
        Rc::new_cyclic(|weak: &Weak<Core<A>>| {
            let sink_core = weak.clone();
            let sink: crate::timers::Sink<Fired<A::Command>> = Rc::new(move |fired| {
                if let Some(core) = sink_core.upgrade() {
                    core.fired(fired);
                }
            });
            Core {
                id: A::ID,
                model: RefCell::new(model),
                ui,
                timers: RefCell::new((parts.timers)(sink)),
                executor: RefCell::new(parts.executor),
                settings: RefCell::new(parts.settings),
                lane: RefCell::new((parts.lane)(report)),
                paths: parts.paths,
                drafts: RefCell::new(HashMap::new()),
                jobs: RefCell::new(HashMap::new()),
                by_key: RefCell::new(HashMap::new()),
                next_job: Cell::new(1),
                next_invocation: Cell::new(1),
                open: RefCell::new(HashMap::new()),
                current: Cell::new(None),
                post,
                inbox,
                pending: RefCell::new(VecDeque::new()),
                rail_applying: Cell::new(false),
                rail_queued: Cell::new(false),
                applying: Cell::new(false),
                budget: Cell::new(PENDING_PER_DRAIN),
                in_wake: Cell::new(false),
                continue_posted: Cell::new(false),
                last_refusal: RefCell::new(None),
                exit: Cell::new(None),
                logout: Cell::new(None),
                linger: Cell::new(None),
                windows: RefCell::new(windows::Windows::new()),
                me: weak.clone(),
                registry: RefCell::new(None),
                watched: crate::documents::Watched::new(),
            }
        })
    }

    /// The documents' files watched, shared with the watch thread the runner starts.
    pub fn watched(&self) -> crate::documents::Watched {
        self.watched.clone()
    }

    pub fn id(&self) -> &'static str {
        self.id
    }

    pub fn ui(&self) -> &A::Ui {
        &self.ui
    }

    /// Where the app's files are.
    pub fn paths(&self) -> &crate::paths::Paths {
        &self.paths
    }

    /// The `Send` endpoint for other threads.
    pub fn post(&self) -> Post<Inbound<A>> {
        self.post.clone()
    }

    /// Read the model.
    pub fn read<R>(&self, f: impl FnOnce(&A::Model) -> R) -> R {
        f(&self.model.borrow())
    }

    /// The last command's refusal, if it was refused.
    pub fn last_refusal(&self) -> Option<Refused> {
        self.last_refusal.borrow().clone()
    }

    /// Draw everything: the first render. A callback a binding fires during it is queued, as
    /// one fired during any other render is.
    pub fn render_all(&self) {
        self.fresh_wake();
        self.applying.set(true);
        self.render_windows(&Changed::all());
        self.applying.set(false);
        self.drain_pending();
    }

    /// `App::start`, and its effects. `start` cannot change the model (it reads it), so nothing
    /// is redrawn unless it says so; the first render has just happened.
    pub fn start(&self) {
        self.fresh_wake();
        let mut fx = Effects::starting(self.next_job.get());
        A::start(&self.model.borrow(), &mut fx);
        let (requests, changed, next) = fx.into_parts();
        self.next_job.set(next);
        self.applying.set(true);
        self.current.set(None);
        self.commit(requests);
        if !changed.is_empty() {
            self.render_windows(&changed);
        }
        // An app with a rail: the first refresh now, and the fallback tick from here on.
        if let Some(refresh) = A::rail(RailCommand::Refresh) {
            let tick = refresh.clone();
            let fire = Box::new(move || Fired::Command(tick.clone()));
            self.timers.borrow_mut().every(rail::TICK, rail::TICK_EVERY, fire);
            self.queue(None, refresh);
        }
        self.applying.set(false);
        self.drain_pending();
    }

    /// Apply a local command now (on the UI thread), or queue it if one is being applied.
    pub fn run(&self, command: A::Command) {
        if self.applying.get() {
            self.queue(None, command);
            return;
        }
        self.fresh_wake();
        let _ = self.apply(None, command);
        self.drain_pending();
    }

    /// A call from outside [`Core::drain`] (a Slint callback, a timer, the surface's hop, the
    /// first render) is a wake of its own, with the whole budget.
    fn fresh_wake(&self) {
        if !self.in_wake.get() {
            self.budget.set(PENDING_PER_DRAIN);
        }
    }

    /// Apply a command that came through the surface, under an invocation. Returns the first
    /// command's refusal, if it was refused. An explicit settlement opened for `id` is settled
    /// here with the reply when the invocation started no job and queued nothing more.
    pub fn run_invocation(
        &self,
        origin: Origin,
        command: A::Command,
        id: Option<InvocationId>,
    ) -> Result<(), Refused> {
        if self.post.is_closed() {
            return Err(Refused::new("closing", "The app is closing."));
        }
        let id = id.unwrap_or_else(|| {
            let id = InvocationId(self.next_invocation.get());
            self.next_invocation.set(id.0 + 1);
            id
        });
        let invocation = Invocation { id, origin };
        if self.applying.get() {
            // A surface call while a command is applied: the rail's apply, which runs through
            // the surface from `commit`. The command is queued and runs when this one is done.
            self.queue(Some(invocation), command);
            return Ok(());
        }
        self.fresh_wake();
        let result = self.apply(Some(invocation), command);
        self.drain_pending();
        if result.is_ok() {
            self.settle_if_idle(id);
        }
        result
    }

    /// Queues a command behind the one being applied, under `invocation` if it has one; an open
    /// settlement counts it, so the invocation is not settled while it waits.
    fn queue(&self, invocation: Option<Invocation>, command: A::Command) {
        if let Some(invocation) = invocation {
            self.count_queued(invocation.id, 1);
        }
        let from_rail = self.rail_applying.get();
        if from_rail {
            self.rail_queued.set(true);
        }
        self.pending.borrow_mut().push_back((invocation, command, from_rail));
    }

    /// Applies what is queued while this wake's budget lasts; the rest wait for the next wake,
    /// so the event loop draws between floods, and one `Continue` at most is left in the inbox
    /// to bring that wake about. A queued command's refusal is its invocation's terminal
    /// outcome.
    fn drain_pending(&self) {
        while self.budget.get() > 0 {
            let next = self.pending.borrow_mut().pop_front();
            let Some((invocation, command, from_rail)) = next else {
                return;
            };
            self.budget.set(self.budget.get() - 1);
            if let Some(invocation) = invocation {
                self.count_queued(invocation.id, -1);
            }
            let result = self.apply(invocation, command);
            // The rail's apply is answered by what its command did, not by its admission.
            if from_rail {
                let outcome = result.as_ref().map(|_| ()).map_err(|r| r.sentence.clone());
                if let Some(applied) = A::rail(RailCommand::Applied(outcome)) {
                    self.queue(None, applied);
                }
            }
            if let Some(invocation) = invocation {
                match result {
                    Ok(()) => self.settle_if_idle(invocation.id),
                    Err(refused) => self.settle(invocation.id, Err(refused.sentence)),
                }
            }
        }
        if !self.pending.borrow().is_empty() && !self.continue_posted.replace(true) {
            tracing::warn!(
                app = self.id,
                "{PENDING_PER_DRAIN} queued commands applied in one wake; the rest wait for the next"
            );
            if self.post.post_control(Inbound::Continue(Continuation(()))).is_err() {
                self.continue_posted.set(false);
            }
        }
    }

    /// Commands queued and not yet applied.
    pub fn pending_commands(&self) -> usize {
        self.pending.borrow().len()
    }

    /// One command: update, commit its effects if it was accepted, render what changed.
    fn apply(&self, invocation: Option<Invocation>, command: A::Command) -> Result<(), Refused> {
        self.applying.set(true);
        self.current.set(invocation);
        let mut fx = Effects::new(self.next_job.get(), invocation);
        let result = A::update(&mut self.model.borrow_mut(), command, &mut fx);
        let (requests, changed, next) = fx.into_parts();
        match &result {
            Ok(()) => {
                self.next_job.set(next);
                *self.last_refusal.borrow_mut() = None;
                self.commit(requests);
                if !changed.is_empty() {
                    self.render_windows(&changed);
                }
                self.rail_changed(&changed);
            }
            Err(refused) => {
                // Nothing it asked for runs; the sentence is the person's and the Mind's.
                tracing::warn!(app = self.id, kind = refused.kind, "{}", refused.sentence);
                if refused.is_internal() {
                    problems::record_failure(self.id, refused.kind, &refused.sentence);
                }
                *self.last_refusal.borrow_mut() = Some(refused.clone());
            }
        }
        self.current.set(None);
        self.applying.set(false);
        result
    }

    /// The effects of an accepted command, in order. `self.current` is the invocation it ran
    /// under. Always under `applying`, so a command an effect raises is queued.
    fn commit(&self, requests: Vec<Request<A>>) {
        let invocation = self.current.get();
        // Closed after everything else this command asked for, so a window it replaces is on
        // screen before the old one goes.
        let mut closing = Vec::new();
        let mut linger = None;
        let mut quit = false;
        for request in requests {
            match request {
                Request::Then(command) => self.queue(invocation, command),
                Request::Spawn {
                    id,
                    key,
                    mode,
                    failed,
                    job,
                } => self.spawn(id, key, mode, failed, job, invocation),
                Request::Cancel(id) => self.cancel(id),
                Request::Every {
                    id,
                    period,
                    command,
                } => {
                    let fire = Box::new(move || Fired::Command(command.clone()));
                    self.timers.borrow_mut().every(id, period, fire);
                }
                Request::After { id, delay, command } => {
                    let fire = Box::new(move || Fired::Command(command.clone()));
                    self.timers.borrow_mut().after(id, delay, fire);
                }
                Request::StopTimer(id) => self.timers.borrow_mut().stop(id),
                Request::Report { what, error } => {
                    tracing::error!(app = self.id, what, error, "a failure the app survived");
                    problems::record_failure(self.id, what, &error);
                }
                Request::Settle(result) => {
                    if let Some(invocation) = invocation {
                        self.settle(invocation.id, result.map_err(|r| r.sentence));
                    }
                }
                Request::SaveSettings {
                    value,
                    replace_unusable,
                } => self.save_settings(value, replace_unusable),
                Request::SaveDraft { name, bytes, delay } => self.save_draft(name, bytes, delay),
                Request::DiscardDraft(name) => self.discard_draft(name),
                Request::Watch { key, path, seen } => self.watched.add(&key, path, seen),
                Request::Unwatch(key) => self.watched.remove(&key),
                // Never blocks: the send is a thread of its own, bounded by the crate.
                Request::Notify(notification) => {
                    // Always this app's: the name comes from the framework, not the caller (§5).
                    yantrik_notifications::send(notification.as_app(self.id));
                }
                Request::ApplyAsAgent { apply, authority } => {
                    // The surface queues the intent's command (a command is being applied);
                    // what it queues is tagged and reports `Applied` once it has run. A call
                    // the surface refused reports now.
                    self.rail_queued.set(false);
                    self.rail_applying.set(true);
                    let admitted = self.act_as_agent(apply, authority);
                    self.rail_applying.set(false);
                    let report = match admitted {
                        Err(why) => Some(Err(why)),
                        Ok(()) if !self.rail_queued.get() => Some(Ok(())),
                        Ok(()) => None,
                    };
                    if let Some(report) = report {
                        if let Some(applied) = A::rail(RailCommand::Applied(report)) {
                            self.queue(None, applied);
                        }
                    }
                }
                Request::OpenWindow(window) => self.open_window(window),
                Request::Close(window) => closing.push(window),
                Request::Linger(backstop) => linger = Some(backstop),
                Request::Exit => self.request_exit(ExitReason::App),
                Request::RequestQuit => quit = true,
            }
        }
        // Once every effect is applied: a close and an open in one command replace a window,
        // and a close of the last one ends the process, or lingers if the command asked.
        self.closed(closing, linger);
        if quit {
            self.quit_requested();
        }
    }

    /// Everything posted since the last drain, in order: one wake, with one budget for what is
    /// queued. The wake calls this.
    pub fn drain(&self) {
        self.in_wake.set(true);
        self.budget.set(PENDING_PER_DRAIN);
        for inbound in self.inbox.drain() {
            match inbound {
                Inbound::Command(command) => self.run(command),
                Inbound::Finished(finished) => self.finished(finished),
                Inbound::Event(event) => self.event(event),
                Inbound::Written(written) => self.written(written),
                Inbound::Seen(seen) => self.seen(seen),
                Inbound::Signal(signal) => self.signalled(signal),
                Inbound::Overdue(signal) => self.overdue(signal),
                Inbound::Continue(_) => {
                    self.continue_posted.set(false);
                    self.drain_pending();
                }
            }
        }
        self.in_wake.set(false);
    }

    /// The end, in order: a signal still queued heard (its logout raised and applied), no more
    /// posts, no more timers, the writes owed handed to the lane, every open answer refused,
    /// every job cancelled, then one bounded wait shared by the lane and the pool (§4: the grace
    /// is the whole of it, not one per drain); after a signal, the lane may use what is left of
    /// `quit::SIGNAL_DEADLINE`. Nothing else in the model is touched here.
    pub fn shutdown(&self, grace: Duration) -> bool {
        self.hear_last_signals();
        let deadline = Instant::now() + grace;
        let lane_deadline = Instant::now() + self.write_window(grace);
        self.inbox.close();
        self.timers.borrow_mut().stop_all();
        // A save asked for is a save made, before the process ends.
        self.flush_settings();
        // Every owed answer is refused as a closing, before a cancelled job can call it replaced.
        let owed: Vec<InvocationId> = self.open.borrow().keys().copied().collect();
        for id in owed {
            self.settle(id, Err("the app is closing".into()));
        }
        let ids: Vec<JobId> = self.jobs.borrow().keys().copied().collect();
        for id in ids {
            self.cancel(id);
        }
        let written = self
            .lane
            .borrow()
            .drain(lane_deadline.saturating_duration_since(Instant::now()));
        if !written {
            tracing::warn!(
                app = self.id,
                "a write did not finish within the grace period; it is not durable"
            );
        }
        let drained = self
            .executor
            .borrow()
            .drain(deadline.saturating_duration_since(Instant::now()));
        if !drained {
            tracing::warn!(
                app = self.id,
                "a job did not stop within the grace period; exiting without it"
            );
        }
        written && drained
    }

    /// Jobs the core is waiting on.
    pub fn jobs_waiting(&self) -> usize {
        self.jobs.borrow().len()
    }

    pub fn timers_armed(&self) -> usize {
        self.timers.borrow().armed()
    }

    /// A refusal raised before a command existed (a converter in `try_cmd1` could not make one).
    fn refuse(&self, refused: Refused) {
        tracing::warn!(app = self.id, kind = refused.kind, "{}", refused.sentence);
        *self.last_refusal.borrow_mut() = Some(refused);
    }
}
