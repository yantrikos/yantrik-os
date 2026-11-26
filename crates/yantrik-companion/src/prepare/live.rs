//! What the running preparation job shows, and what the person can tell it (#1416, round 2).
//!
//! One job runs at a time, on a thread of its own (`hook::spawn`). This is the small shared state
//! between that thread and the shell: the job writes its title and each step as it happens, and
//! Mind View reads them; the person's **Pause**, **Resume** and **Stop** set the flags the job
//! reads before every call and every write. Steps are words made here from counts and kinds of
//! work ("searched", "read 3 sources"), never fetched text.
//!
//! Stop is checked before every call and while the job waits, and a call that was already running
//! is not trusted: its answer is dropped. After Stop the door lets nothing more through, so the
//! only thing that can still land is the one call that was already in flight when it was pressed
//! (a web fetch has its own time limit; a file write is over in a moment).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The most steps kept (the newest).
pub const MAX_STEPS: usize = 12;

/// Where the job is, in the words Mind View uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Running,
    /// Waiting: the person pressed Pause, or a turn of theirs is open.
    Paused,
    Stopping,
}

impl Phase {
    pub fn word(self) -> &'static str {
        match self {
            Phase::Running => "running",
            Phase::Paused => "paused",
            Phase::Stopping => "stopping",
        }
    }
}

/// What Mind View draws for the job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// The item, as the agenda titled it.
    pub title: String,
    pub phase: Phase,
    /// Whether the pause is the person's own button (so Resume is theirs), not an open turn.
    pub by_person: bool,
    pub steps: Vec<String>,
}

static JOB: Mutex<Option<Snapshot>> = Mutex::new(None);
static PAUSE: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);

fn job() -> std::sync::MutexGuard<'static, Option<Snapshot>> {
    JOB.lock().unwrap_or_else(|e| e.into_inner())
}

/// A job starts: flags clear, steps empty.
pub fn begin(title: &str) {
    PAUSE.store(false, Ordering::SeqCst);
    STOP.store(false, Ordering::SeqCst);
    *job() = Some(Snapshot { title: title.to_string(), phase: Phase::Running, by_person: false, steps: Vec::new() });
}

/// The job is over (done, capped, stopped or gone).
pub fn end() {
    *job() = None;
    PAUSE.store(false, Ordering::SeqCst);
    STOP.store(false, Ordering::SeqCst);
}

/// One step happened.
pub fn step(words: &str) {
    if let Some(j) = job().as_mut() {
        j.steps.push(words.to_string());
        let over = j.steps.len().saturating_sub(MAX_STEPS);
        j.steps.drain(..over);
    }
}

/// The job is waiting (`true`) or going on (`false`).
pub fn waiting(on: bool) {
    let by_person = PAUSE.load(Ordering::SeqCst);
    if let Some(j) = job().as_mut() {
        if j.phase != Phase::Stopping {
            j.phase = if on { Phase::Paused } else { Phase::Running };
            j.by_person = on && by_person;
        }
    }
}

/// The person's Pause (`true`) or Resume (`false`). Nothing when no job is running.
pub fn request_pause(on: bool) {
    if is_active() {
        PAUSE.store(on, Ordering::SeqCst);
        if let Some(j) = job().as_mut() {
            if j.phase != Phase::Stopping {
                j.phase = if on { Phase::Paused } else { Phase::Running };
                j.by_person = on;
            }
        }
    }
}

/// The person's Stop. It works whatever the job is doing, and the job is not left waiting.
pub fn request_stop() {
    if is_active() {
        STOP.store(true, Ordering::SeqCst);
        if let Some(j) = job().as_mut() {
            j.phase = Phase::Stopping;
        }
    }
}

pub fn stopped() -> bool {
    STOP.load(Ordering::SeqCst)
}

pub fn person_paused() -> bool {
    PAUSE.load(Ordering::SeqCst)
}

pub fn is_active() -> bool {
    job().is_some()
}

pub fn snapshot() -> Option<Snapshot> {
    job().clone()
}

/// The state above is process-wide: the tests that start a job take this first.
#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// One test: the state is process-wide.
    #[test]
    fn steps_pause_resume_and_stop() {
        let _one = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        request_stop();
        assert!(!stopped(), "no job: a Stop reaches nothing");
        begin("Record the 60 s demo video");
        for i in 0..15 {
            step(&format!("step {i}"));
        }
        let s = snapshot().unwrap();
        assert_eq!((s.steps.len(), s.steps[0].as_str()), (MAX_STEPS, "step 3"));
        request_pause(true);
        assert_eq!((snapshot().unwrap().phase, snapshot().unwrap().by_person, person_paused()), (Phase::Paused, true, true));
        request_pause(false);
        assert_eq!(snapshot().unwrap().phase, Phase::Running);
        waiting(true);
        assert_eq!((snapshot().unwrap().phase, snapshot().unwrap().by_person), (Phase::Paused, false), "a turn of theirs, not their button");
        request_stop();
        assert!(stopped());
        waiting(false);
        assert_eq!(snapshot().unwrap().phase, Phase::Stopping, "stopping stays");
        end();
        assert!(snapshot().is_none() && !stopped() && !person_paused());
    }
}
