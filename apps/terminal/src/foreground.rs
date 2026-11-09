//! Whether a shell's terminal has a job in front of it, asked of the pty itself.
//!
//! The terminal's controlling pty knows which process group owns it: the shell's own at the
//! prompt, the job's while a command runs. That holds however the command started (typed, pasted,
//! sent by `send_input`, or the `run` act), which a `/proc` probe on a worker only learns when
//! something marks the tab due. One `tcgetpgrp` is a single non-blocking ioctl, cheap enough for
//! the UI thread.
use std::{
    os::fd::RawFd,
    sync::{
        atomic::{AtomicI32, Ordering},
        Mutex,
    },
};

/// The pty master's descriptor, lent by the session's worker, which owns it. The worker calls
/// [`Tty::release`] before it closes the master, so a descriptor number reused later is never
/// asked; the lock is held across the ioctl for the same reason.
pub struct Tty {
    master: Mutex<Option<RawFd>>,
    /// The shell's own process group once it is known (0 until then).
    shell: AtomicI32,
}

impl Tty {
    pub fn new(master: RawFd) -> Self {
        Self { master: Mutex::new(Some(master)), shell: AtomicI32::new(0) }
    }

    /// The master is about to close: stop answering.
    pub fn release(&self) {
        *self.master.lock().unwrap() = None;
    }

    /// The process group in front of the terminal, if the master is still open.
    pub fn foreground(&self) -> Option<i32> {
        let held = self.master.lock().unwrap();
        // SAFETY: the descriptor is open while it is in the slot (the worker empties the slot
        // before dropping the master), and the lock keeps it so for the call.
        let group = unsafe { libc::tcgetpgrp((*held)?) };
        (group > 0).then_some(group)
    }

    /// The shell has spoken, so it has taken the terminal: whatever group is in front now is its
    /// own. Its group is its pid for a shell started directly, but not for one started through
    /// a wrapper such as `bwrap` (the wrapper leads the session, and the shell makes a group of its
    /// own), so it is learned here and not assumed.
    pub fn note_shell(&self) {
        if self.shell.load(Ordering::Acquire) == 0 {
            if let Some(group) = self.foreground() {
                self.shell.store(group, Ordering::Release);
            }
        }
    }

    /// Whether something other than the shell's own group is in front. Before the shell has
    /// spoken, its group is taken to be `leader`, the pid the session started.
    ///
    /// A background job (`sleep 100 &`) does not count: its group is not in front, the shell's is
    /// again, and the shell is free to take input. Only a job the shell is waiting on is work in
    /// flight.
    pub fn job_in_front(&self, leader: u32) -> bool {
        let shell = match self.shell.load(Ordering::Acquire) {
            0 => leader as i32,
            group => group,
        };
        self.foreground().is_some_and(|group| group != shell)
    }
}
