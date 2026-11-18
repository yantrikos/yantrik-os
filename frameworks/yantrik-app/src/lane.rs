//! An ordered line of blocking commands to one thing, worked through by one thread.
//!
//! The framework's jobs run on a pool, in any order. An app whose commands must reach a resource
//! in the order they were asked for (a library on disk, a download engine) cannot hand each to a
//! job: a `build` asked after an `update` could reach the library first. So `update` hands each
//! command to the lane, in the order it applies them, and the job it spawns waits for the lane's
//! [`Reply`]. The channel is the order; the lane's thread is the only one that ever calls the
//! resource's commands, so the resource needs no lock of its own.
//!
//! A command that panics ends that command, not the lane: its job finds no reply and says so.
//! Dropping the `Lane` lets its thread finish what is queued and end.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;

type Task<S> = Box<dyn FnOnce(&S) + Send>;

/// The line of commands to a `S`, and the thread that works through it.
pub struct Lane<S> {
    line: mpsc::Sender<Task<S>>,
}

/// What one command will have come to, once the lane gets to it.
pub struct Reply<T>(mpsc::Receiver<T>);

impl<T> Reply<T> {
    /// Waits for the lane. `None` if the command panicked or the lane is gone.
    pub fn wait(self) -> Option<T> {
        self.0.recv().ok()
    }
}

impl<S: Send + 'static> Lane<S> {
    /// Starts the lane's thread, named `name`, holding `state`.
    pub fn start(name: &str, state: S) -> Lane<S> {
        let (line, tasks) = mpsc::channel::<Task<S>>();
        let started = std::thread::Builder::new().name(name.into()).spawn(move || {
            while let Ok(task) = tasks.recv() {
                if catch_unwind(AssertUnwindSafe(|| task(&state))).is_err() {
                    tracing::error!("a command to the lane panicked");
                }
            }
        });
        if let Err(e) = started {
            tracing::error!(error = %e, lane = name, "the lane could not be started; every command will fail");
        }
        Lane { line }
    }

    /// Queue `command` behind everything asked before it. Called from `update`: it only sends.
    pub fn ask<T: Send + 'static>(&self, command: impl FnOnce(&S) -> T + Send + 'static) -> Reply<T> {
        let (answer, reply) = mpsc::sync_channel(1);
        let task: Task<S> = Box::new(move |state| {
            let _ = answer.send(command(state));
        });
        // A lane that never started leaves the task unqueued: its sender drops, and the reply is
        // `None`.
        let _ = self.line.send(task);
        Reply(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn commands_run_in_the_order_they_were_asked_whichever_thread_waits_first() {
        let lane = Lane::start("test-lane", ());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let replies: Vec<_> = (0..20)
            .map(|n| {
                let seen = seen.clone();
                lane.ask(move |_| {
                    std::thread::sleep(std::time::Duration::from_millis(if n % 2 == 0 { 3 } else { 0 }));
                    seen.lock().unwrap().push(n);
                    n
                })
            })
            .collect();
        // Waited on last-first, as a pool might run the jobs.
        let got: Vec<_> = replies.into_iter().rev().map(|r| r.wait()).collect();
        assert_eq!(got.len(), 20);
        assert_eq!(*seen.lock().unwrap(), (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn a_command_that_panics_costs_only_itself() {
        let lane = Lane::start("test-lane", 5u8);
        let bad = lane.ask(|_| -> u8 { panic!("a stand-in fault") });
        let good = lane.ask(|n| *n + 2);
        assert_eq!(bad.wait(), None);
        assert_eq!(good.wait(), Some(7));
    }
}
