//! How a shell's worker wakes the app, at most once per frame.
//!
//! Each session has a worker that sleeps on its PTY. When output arrives it calls the notifier
//! this makes. The first call after a frame posts one `Output` command to the UI thread; every
//! call after it, until that frame has been taken, does nothing. So a shell that prints without
//! pause (`yes`) costs the UI thread one command a frame, not one per read, and nothing queues
//! behind it: memory is the session's bounded scrollback and one flag.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use yantrik_app::cx::Inbound;
use yantrik_app::queue::Post;

use crate::command::Command;
use crate::Terminal;

type Door = Post<Inbound<Terminal>>;

#[derive(Clone, Default)]
pub struct Hub {
    door: Arc<Mutex<Option<Door>>>,
    pending: Arc<AtomicBool>,
}

impl Hub {
    /// Where the commands go: set once, at wire time, when the core exists.
    pub fn connect(&self, door: Door) {
        *self.door.lock().unwrap_or_else(|p| p.into_inner()) = Some(door);
    }

    /// The closure a session calls from its worker when its screen or its life changed.
    pub fn notifier(&self) -> Arc<dyn Fn() + Send + Sync> {
        let hub = self.clone();
        Arc::new(move || hub.wake())
    }

    fn wake(&self) {
        if self.pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let door = self.door.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let posted = door.is_some_and(|door| door.post(Inbound::Command(Command::Output)).is_ok());
        if !posted {
            // No door yet, or the queue is full or closed: the next wake tries again.
            self.pending.store(false, Ordering::Release);
        }
    }

    /// The frame is being taken: output after this posts again. Call before reading the sessions.
    pub fn release(&self) {
        self.pending.store(false, Ordering::Release);
    }
}
