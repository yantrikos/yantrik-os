//! Work off the UI thread (design/app-framework-2026-10-21.md, §4).
//!
//! A job is a closure that gets a [`Cancel`] and returns the command that carries its result.
//! It runs on a small named pool of threads, and its command comes back through the bridge to
//! be applied like any other. The runtime keeps a table of jobs it is waiting on; a cancelled or
//! replaced job's late result is dropped there, not drawn.
//!
//! Rust cannot kill a blocking thread safely, so cancellation is cooperative: a job polls
//! `cancel.is_set()` between its steps and returns early. At shutdown every job is cancelled,
//! the pool is given a grace period, and the process exits whether or not a job answered.
//!
//! The pool is an [`Executor`] so a test can run jobs when it chooses ([`Manual`]).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use crate::app::WindowId;

/// One job, by the number the runtime gave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(pub u64);

/// What a job is about, so two of a kind can be held to [`Concurrency`]. Never a global
/// "refresh": the feature and what it refreshes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JobKey {
    pub feature: &'static str,
    pub scope: Scope,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    App,
    Window(WindowId),
    Entity(String),
}

/// What happens when a job is spawned while one with the same key runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Concurrency {
    Parallel,
    RejectIfRunning,
    ReplacePrevious,
}

/// Set by the runtime when a job should stop. Polled by the job between its steps.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Cancel {
        Cancel::default()
    }

    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn set(&self) {
        self.0.store(true, Ordering::Release);
    }
}

/// The work: given the cancel flag, returns the command that carries the result.
pub type Job<C> = Box<dyn FnOnce(&Cancel) -> C + Send + 'static>;

/// How a finished job reaches the runtime.
pub enum Finished<C> {
    Done {
        id: JobId,
        command: C,
    },
    /// The job panicked. Its id reaches a terminal state; nothing is applied.
    Panicked {
        id: JobId,
        what: String,
    },
    /// The job was not started: the pool is at its bound. Its id reaches a terminal state.
    Refused {
        id: JobId,
        why: String,
    },
}

/// Where a finished job is delivered: the bridge, in the runtime; a channel, in a test.
pub trait Deliver<C>: Send + Sync {
    fn deliver(&self, finished: Finished<C>);
}

/// Runs jobs. The runtime's is [`Threads`]; a test's is [`Manual`].
pub trait Executor<C: Send + 'static>: Send {
    fn spawn(&mut self, id: JobId, cancel: Cancel, job: Job<C>, deliver: Arc<dyn Deliver<C>>);
    /// Jobs started and not yet finished.
    fn running(&self) -> usize;
    /// Waits up to `grace` for every job to finish. Says whether they did; a job still running
    /// is left to finish on its own, and the process does not wait for it.
    fn drain(&self, grace: Duration) -> bool;
}

fn run_job<C>(id: JobId, cancel: &Cancel, job: Job<C>, deliver: &dyn Deliver<C>) {
    // The panic is caught here and filed once by the core; the hook stays out of it, or one
    // survived panic would read as a crash and a failure both.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::problems::without_panic_records(|| job(cancel))
    }));
    match outcome {
        Ok(command) => deliver.deliver(Finished::Done { id, command }),
        Err(panic) => {
            let what = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "a job panicked".into());
            deliver.deliver(Finished::Panicked { id, what });
        }
    }
}

/// A fixed pool of named threads. Bounded in two ways: `threads` run at once, and at most
/// `waiting` more wait their turn; past that, [`Executor::spawn`] delivers a panic-shaped
/// refusal so the id still reaches a terminal state.
pub struct Threads<C: Send + 'static> {
    sender: mpsc::Sender<Task<C>>,
    pending: Arc<AtomicUsize>,
    waiting: usize,
}

type Task<C> = (JobId, Cancel, Job<C>, Arc<dyn Deliver<C>>);

impl<C: Send + 'static> Threads<C> {
    /// `name` is the app's id; the threads are `<name>-job-0..`.
    pub fn new(name: &str, threads: usize, waiting: usize) -> Threads<C> {
        let (sender, receiver) = mpsc::channel::<Task<C>>();
        let receiver = Arc::new(Mutex::new(receiver));
        let pending = Arc::new(AtomicUsize::new(0));
        for n in 0..threads.max(1) {
            let receiver = receiver.clone();
            let pending = pending.clone();
            std::thread::Builder::new()
                .name(format!("{name}-job-{n}"))
                .spawn(move || loop {
                    let task = receiver.lock().unwrap_or_else(|p| p.into_inner()).recv();
                    let Ok((id, cancel, job, deliver)) = task else {
                        return;
                    };
                    run_job(id, &cancel, job, deliver.as_ref());
                    pending.fetch_sub(1, Ordering::AcqRel);
                })
                .expect("a job thread");
        }
        Threads {
            sender,
            pending,
            waiting: waiting + threads.max(1),
        }
    }
}

impl<C: Send + 'static> Executor<C> for Threads<C> {
    fn spawn(&mut self, id: JobId, cancel: Cancel, job: Job<C>, deliver: Arc<dyn Deliver<C>>) {
        if self.pending.load(Ordering::Acquire) >= self.waiting {
            deliver.deliver(Finished::Refused {
                id,
                why: "the app is busier than it can be".into(),
            });
            return;
        }
        self.pending.fetch_add(1, Ordering::AcqRel);
        if self.sender.send((id, cancel, job, deliver)).is_err() {
            self.pending.fetch_sub(1, Ordering::AcqRel);
        }
    }

    fn running(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }

    fn drain(&self, grace: Duration) -> bool {
        let deadline = Instant::now() + grace;
        while self.running() > 0 {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }
}

/// Runs jobs only when a test says so, in order, on the calling thread.
#[derive(Default)]
pub struct Manual<C: Send + 'static> {
    queue: Arc<Mutex<VecDeque<Task<C>>>>,
}

impl<C: Send + 'static> Manual<C> {
    pub fn new() -> Manual<C> {
        Manual {
            queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Runs the oldest waiting job. Says whether there was one.
    pub fn run_one(&self) -> bool {
        let task = self
            .queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop_front();
        match task {
            Some((id, cancel, job, deliver)) => {
                run_job(id, &cancel, job, deliver.as_ref());
                true
            }
            None => false,
        }
    }

    /// Runs every waiting job, including ones queued by the ones it runs.
    pub fn run_all(&self) {
        while self.run_one() {}
    }
}

impl<C: Send + 'static> Executor<C> for Manual<C> {
    fn spawn(&mut self, id: JobId, cancel: Cancel, job: Job<C>, deliver: Arc<dyn Deliver<C>>) {
        self.queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push_back((id, cancel, job, deliver));
    }

    fn running(&self) -> usize {
        self.queue.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    /// Nothing runs on its own here: a test runs what it wants first, and what is left waits.
    fn drain(&self, _grace: Duration) -> bool {
        self.running() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sink(Mutex<Vec<String>>);
    impl Deliver<String> for Sink {
        fn deliver(&self, finished: Finished<String>) {
            let line = match finished {
                Finished::Done { id, command } => format!("{} done {command}", id.0),
                Finished::Panicked { id, what } => format!("{} panicked {what}", id.0),
                Finished::Refused { id, why } => format!("{} refused {why}", id.0),
            };
            self.0.lock().unwrap().push(line);
        }
    }

    #[test]
    fn a_pool_runs_jobs_and_a_panic_reaches_a_terminal_state() {
        let sink = Arc::new(Sink(Mutex::new(Vec::new())));
        let mut pool = Threads::<String>::new("test", 2, 8);
        pool.spawn(
            JobId(1),
            Cancel::new(),
            Box::new(|_| "one".to_string()),
            sink.clone(),
        );
        pool.spawn(
            JobId(2),
            Cancel::new(),
            Box::new(|_| panic!("boom")),
            sink.clone(),
        );
        assert!(pool.drain(Duration::from_secs(5)));
        let mut lines = sink.0.lock().unwrap().clone();
        lines.sort();
        assert_eq!(lines, vec!["1 done one", "2 panicked boom"]);
    }

    #[test]
    fn a_cancelled_job_stops_at_its_next_check_and_the_drain_has_a_deadline() {
        let sink = Arc::new(Sink(Mutex::new(Vec::new())));
        let mut pool = Threads::<String>::new("test", 1, 1);
        let cancel = Cancel::new();
        let seen = cancel.clone();
        pool.spawn(
            JobId(1),
            cancel.clone(),
            Box::new(move |c| {
                let start = Instant::now();
                while !c.is_set() && start.elapsed() < Duration::from_secs(5) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                if c.is_set() {
                    "stopped".into()
                } else {
                    "ran out".into()
                }
            }),
            sink.clone(),
        );
        assert!(
            !pool.drain(Duration::from_millis(30)),
            "a running job is not waited for past the deadline"
        );
        seen.set();
        assert!(pool.drain(Duration::from_secs(5)));
        assert_eq!(sink.0.lock().unwrap().clone(), vec!["1 done stopped"]);
    }

    #[test]
    fn an_overfull_pool_refuses_with_a_terminal_outcome() {
        let sink = Arc::new(Sink(Mutex::new(Vec::new())));
        let mut pool = Threads::<String>::new("test", 1, 0);
        let gate = Cancel::new();
        let g = gate.clone();
        pool.spawn(
            JobId(1),
            Cancel::new(),
            Box::new(move |_| {
                while !g.is_set() {
                    std::thread::sleep(Duration::from_millis(2));
                }
                "a".into()
            }),
            sink.clone(),
        );
        std::thread::sleep(Duration::from_millis(20));
        pool.spawn(
            JobId(2),
            Cancel::new(),
            Box::new(|_| "b".into()),
            sink.clone(),
        );
        gate.set();
        pool.drain(Duration::from_secs(5));
        let lines = sink.0.lock().unwrap().clone();
        assert!(
            lines.contains(&"2 refused the app is busier than it can be".to_string()),
            "{lines:?}"
        );
        assert!(lines.contains(&"1 done a".to_string()));
    }

    #[test]
    fn the_manual_executor_runs_when_told() {
        let sink = Arc::new(Sink(Mutex::new(Vec::new())));
        let mut manual = Manual::<String>::new();
        manual.spawn(
            JobId(7),
            Cancel::new(),
            Box::new(|_| "seven".into()),
            sink.clone(),
        );
        assert_eq!(manual.running(), 1);
        assert!(sink.0.lock().unwrap().is_empty());
        manual.run_all();
        assert_eq!(sink.0.lock().unwrap().clone(), vec!["7 done seven"]);
        assert!(!manual.run_one());
    }
}
