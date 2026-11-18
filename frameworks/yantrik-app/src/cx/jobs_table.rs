//! The job table: what the core is waiting on, and what it does when a job lands, ends without a
//! result, or is cancelled. Part of [`Core`]; here so the core's own file stays readable.

use std::sync::Arc;

use super::{Core, PostDeliver, Waiting};
use crate::app::App;
use crate::intents::Invocation;
use crate::jobs::{Cancel, Concurrency, Deliver, Finished, JobId, JobKey};
use crate::problems;

impl<A: App> Core<A> {
    pub(super) fn spawn(
        &self,
        id: JobId,
        key: Option<JobKey>,
        mode: Concurrency,
        failed: Option<A::Command>,
        job: crate::jobs::Job<A::Command>,
        invocation: Option<Invocation>,
    ) {
        let mut replaces = None;
        if let Some(key) = &key {
            let running = self.by_key.borrow().get(key).copied();
            match (running, mode) {
                (Some(_), Concurrency::RejectIfRunning) => {
                    tracing::debug!(
                        app = self.id,
                        ?key,
                        "a job of this kind is running; this one is not started"
                    );
                    // The work was refused: an invocation waiting on it is told so, not told
                    // "done" from a model the job never touched; and the app hears of it through
                    // its `failed` command, as it would of a panic.
                    if let Some(invocation) = invocation {
                        self.settle(
                            invocation.id,
                            Err("a job of this kind is already running".into()),
                        );
                    }
                    if let Some(failed) = failed {
                        self.queue(invocation, failed);
                    }
                    return;
                }
                (Some(old), Concurrency::ReplacePrevious) => replaces = Some(old),
                _ => {}
            }
            self.by_key.borrow_mut().insert(key.clone(), id);
        }
        let cancel = Cancel::new();
        self.jobs.borrow_mut().insert(
            id,
            Waiting {
                cancel: cancel.clone(),
                key,
                invocation,
                failed,
            },
        );
        if let Some(invocation) = invocation {
            if let Some(open) = self.open.borrow_mut().get_mut(&invocation.id) {
                open.jobs += 1;
            }
        }
        // The new job is counted before the old one is cancelled, so a replacement within one
        // invocation never leaves it with nothing running for an instant and settles it.
        if let Some(old) = replaces {
            self.cancel(old);
        }
        let deliver: Arc<dyn Deliver<A::Command>> = Arc::new(PostDeliver(self.post.clone()));
        self.executor.borrow_mut().spawn(id, cancel, job, deliver);
    }

    /// Forgets a job's key, if it still names this job.
    pub(super) fn forget_key(&self, id: JobId, key: Option<JobKey>) {
        if let Some(key) = key {
            let mut by_key = self.by_key.borrow_mut();
            if by_key.get(&key) == Some(&id) {
                by_key.remove(&key);
            }
        }
    }

    /// Stops waiting on a job: its cancel flag is set and its late result is dropped. An
    /// invocation whose last job this was is settled as replaced.
    pub(super) fn cancel(&self, id: JobId) {
        let Some(waiting) = self.jobs.borrow_mut().remove(&id) else {
            return;
        };
        waiting.cancel.set();
        self.forget_key(id, waiting.key);
        if let Some(invocation) = waiting.invocation {
            if self.job_landed(invocation.id) {
                self.settle(
                    invocation.id,
                    Err("the work was replaced by a newer request".into()),
                );
            }
        }
    }

    /// A finished job: its result is applied only if the core is still waiting on it, under the
    /// invocation it started from. A job that ended without a result (a panic, a refusal) runs
    /// the `failed` command it was spawned with, and an invocation waiting on it is told.
    pub(super) fn finished(&self, finished: Finished<A::Command>) {
        match finished {
            Finished::Done { id, command } => {
                let waited = self.jobs.borrow_mut().remove(&id);
                let Some(waiting) = waited else {
                    tracing::debug!(app = self.id, job = id.0, "a late result, dropped");
                    return;
                };
                self.forget_key(id, waiting.key);
                if let Some(invocation) = waiting.invocation {
                    self.job_landed(invocation.id);
                }
                if self.applying.get() {
                    self.queue(waiting.invocation, command);
                } else {
                    let result = self.apply(waiting.invocation, command);
                    self.drain_pending();
                    if let Some(invocation) = waiting.invocation {
                        match result {
                            Ok(()) => self.settle_if_idle(invocation.id),
                            Err(refused) => self.settle(invocation.id, Err(refused.sentence)),
                        }
                    }
                }
            }
            Finished::Panicked { id, what } => {
                tracing::error!(app = self.id, job = id.0, what, "a job panicked");
                problems::record_failure(self.id, "job", &what);
                self.ended_without_result(id, format!("the work failed: {what}"));
            }
            Finished::Refused { id, why } => {
                tracing::warn!(app = self.id, job = id.0, why, "a job was not started");
                problems::record_failure(self.id, "job_refused", &why);
                self.ended_without_result(id, format!("the work could not start: {why}"));
            }
        }
    }

    pub(super) fn ended_without_result(&self, id: JobId, outcome: String) {
        let waited = self.jobs.borrow_mut().remove(&id);
        let Some(waiting) = waited else {
            return;
        };
        self.forget_key(id, waiting.key);
        if let Some(invocation) = waiting.invocation {
            if self.job_landed(invocation.id) {
                self.settle(invocation.id, Err(outcome));
            }
        }
        if let Some(failed) = waiting.failed {
            if self.applying.get() {
                self.queue(waiting.invocation, failed);
            } else {
                let _ = self.apply(waiting.invocation, failed);
                self.drain_pending();
            }
        }
    }
}
