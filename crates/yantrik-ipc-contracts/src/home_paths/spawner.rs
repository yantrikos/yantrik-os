//! The one thread that forks every program started in the outside view (#1124, round 14).
//!
//! `bwrap --die-with-parent` arms `PR_SET_PDEATHSIG(SIGKILL)` on itself, and Linux sends that
//! signal when the *thread* that forked it exits, not only when the process does. A tokio
//! blocking-pool thread exits after ten idle seconds, so a program forked there and meant to
//! outlive the call (a job, a background task, an MCP server) was killed with the thread. Every
//! such program is forked here instead: one thread that lives as long as the process, so the
//! flag keeps its meaning, "end when the shell ends".

use std::process::{Child, Command};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

type Task = Box<dyn FnOnce() + Send>;

fn sender() -> &'static Mutex<Sender<Task>> {
    static SPAWNER: OnceLock<Mutex<Sender<Task>>> = OnceLock::new();
    SPAWNER.get_or_init(|| {
        let (tx, rx) = channel::<Task>();
        std::thread::Builder::new()
            .name("mind-spawner".into())
            .spawn(move || {
                // A task that panics must not take the thread, and with it every child, down.
                rx.into_iter().for_each(|task| {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
                });
            })
            .expect("the spawner thread must start");
        Mutex::new(tx)
    })
}

/// Run `fork` (which must do the fork and return) on the spawner thread, and give back what it
/// returned. `None` only if it panicked.
pub fn on_spawner<R: Send + 'static>(fork: impl FnOnce() -> R + Send + 'static) -> Option<R> {
    let (back, answer) = channel();
    let task: Task = Box::new(move || {
        let _ = back.send(fork());
    });
    sender().lock().unwrap_or_else(|e| e.into_inner()).send(task).ok()?;
    answer.recv().ok()
}

/// `command.spawn()`, forked on the spawner thread.
pub fn spawn(mut command: Command) -> std::io::Result<Child> {
    on_spawner(move || command.spawn()).unwrap_or_else(|| Err(std::io::Error::other("the spawner thread failed")))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::home_paths::workshops;
    use std::time::Duration;

    /// A program with `--die-with-parent`, forked by a thread that then ends, as a tokio
    /// blocking-pool thread does after its idle seconds.
    fn forked_by_a_thread_that_ends(through_spawner: bool) -> Option<bool> {
        let bwrap = workshops::find_bwrap()?;
        let mut args = workshops::outside_args(&[], true);
        args.extend(["sleep", "30"].map(String::from));
        let mut command = Command::new(bwrap);
        command.args(args);
        let probe = std::thread::spawn(move || {
            let child = if through_spawner { spawn(command) } else { command.spawn() };
            // Long enough for bwrap to arm its parent-death signal before this thread ends.
            std::thread::sleep(Duration::from_secs(1));
            child
        });
        let mut child = probe.join().unwrap().ok()?;
        std::thread::sleep(Duration::from_millis(1500));
        let alive = child.try_wait().ok()?.is_none();
        let _ = child.kill();
        let _ = child.wait();
        Some(alive)
    }

    #[test]
    fn a_program_forked_here_outlives_the_thread_that_asked() {
        // Without bwrap (or a user namespace to run it in) there is nothing to observe.
        if let Some(alive) = forked_by_a_thread_that_ends(true) {
            assert!(alive, "the spawner thread outlives the caller, so the program does too");
        }
    }
}
