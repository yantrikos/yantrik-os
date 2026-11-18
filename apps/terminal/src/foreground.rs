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
    /// The shell is a child of the program the session started (`bwrap`), not that program.
    wrapped: bool,
}

impl Tty {
    pub fn new(master: RawFd, wrapped: bool) -> Self {
        Self { master: Mutex::new(Some(master)), shell: AtomicI32::new(0), wrapped }
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

    /// The shell has spoken: learn its process group. A shell started directly leads its own
    /// group, which is `leader` (the pid the session started), so there is nothing to learn. One
    /// started through a wrapper makes a group of its own, so it is the first group in front that
    /// is not the wrapper's. The first byte read is no proof: when a Mind's `run` arrives before
    /// the shell's first prompt, the pty echoes it while the wrapper's group is still in front,
    /// and taking that group would show the tab busy for good (round 7, finding 6).
    pub fn note_shell(&self, leader: u32) {
        if !self.wrapped || self.shell.load(Ordering::Acquire) != 0 {
            return;
        }
        if let Some(group) = self.foreground().and_then(|g| Self::shell_group(leader, g)) {
            self.shell.store(group, Ordering::Release);
        }
    }

    /// Behind a wrapper, the group to remember as the shell's given the one in front: any but
    /// the wrapper's own.
    pub(crate) fn shell_group(leader: u32, in_front: i32) -> Option<i32> {
        (in_front != leader as i32).then_some(in_front)
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

#[cfg(test)]
mod tests {
    use super::Tty;

    /// Round 7, finding 6: behind a wrapper (leader 100), the echo of an early `run` shows the
    /// wrapper's group in front, and only a group of the shell's own is learned.
    #[test]
    fn only_a_group_other_than_the_wrappers_is_learned_as_the_shells() {
        assert_eq!(Tty::shell_group(100, 100), None);
        assert_eq!(Tty::shell_group(100, 140), Some(140));
    }
}
