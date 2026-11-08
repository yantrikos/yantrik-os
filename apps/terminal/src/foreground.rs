//! Whether a shell's terminal has a job in front of it, asked of the pty itself.
//!
//! The terminal's controlling pty knows which process group owns it: the shell's own at the
//! prompt, the job's while a command runs. That holds however the command started (typed, pasted,
//! sent by `send_input`, or the `run` act), which a `/proc` probe on a worker only learns when
//! something marks the tab due. One `tcgetpgrp` is a single non-blocking ioctl, cheap enough for
//! the UI thread.
use std::{os::fd::RawFd, sync::Mutex};

/// The pty master's descriptor, lent by the session's worker, which owns it. The worker calls
/// [`Tty::release`] before it closes the master, so a descriptor number reused later is never
/// asked; the lock is held across the ioctl for the same reason.
pub struct Tty(Mutex<Option<RawFd>>);

impl Tty {
    pub fn new(master: RawFd) -> Self {
        Self(Mutex::new(Some(master)))
    }

    /// The master is about to close: stop answering.
    pub fn release(&self) {
        *self.0.lock().unwrap() = None;
    }

    /// The process group in front of the terminal, if the master is still open.
    pub fn foreground(&self) -> Option<i32> {
        let held = self.0.lock().unwrap();
        // SAFETY: the descriptor is open while it is in the slot (the worker empties the slot
        // before dropping the master), and the lock keeps it so for the call.
        let group = unsafe { libc::tcgetpgrp((*held)?) };
        (group > 0).then_some(group)
    }

    /// Whether something other than the shell's own group is in front. The shell leads its
    /// session (`setsid`), so its group id is its pid.
    ///
    /// A background job (`sleep 100 &`) does not count: its group is not in front, the shell's is
    /// again, and the shell is free to take input. Only a job the shell is waiting on is work in
    /// flight.
    pub fn job_in_front(&self, shell: u32) -> bool {
        self.foreground().is_some_and(|group| group != shell as i32)
    }
}
