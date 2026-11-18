//! The bridge from any thread to the UI thread: a bounded queue of commands, and the `Send`
//! endpoint that posts to it (design/app-framework-2026-10-21.md, §2.3 and §4).
//!
//! The runner keeps the app's state on the UI thread. A worker, a timer or the surface's RPC
//! thread never touches it; each holds a [`Post`] and posts a command. The post wakes the UI
//! thread (through `slint::invoke_from_event_loop` in the runner; through a plain function in
//! tests), and the runner applies the command there.
//!
//! Bounded: a flood of posts gets [`Dropped::Full`] instead of unbounded memory, and the UI
//! thread never blocks waiting for space. Closed: a post after shutdown gets [`Dropped::Closed`]
//! and nothing else happens.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// How many posted commands may wait at once. Chosen above anything a working app reaches (a
/// keystroke a frame, a few jobs landing) and below anything a runaway loop would want.
pub const CAPACITY: usize = 1024;

/// Why a post was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
    /// The queue is full: the app is busier than it can be. Try again later.
    Full,
    /// The app has shut down.
    Closed,
}

/// Wakes the UI thread so it drains the queue. The runner's is `slint::invoke_from_event_loop`.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

struct Shared<C> {
    queue: Mutex<VecDeque<C>>,
    closed: AtomicBool,
    wake: Wake,
    capacity: usize,
}

/// The `Send` endpoint any thread may hold. Cloning shares the queue.
pub struct Post<C>(Arc<Shared<C>>);

impl<C> Clone for Post<C> {
    fn clone(&self) -> Self {
        Post(self.0.clone())
    }
}

/// The UI thread's end: it drains what was posted.
pub struct Inbox<C>(Arc<Shared<C>>);

/// A queue and its endpoint, waking with `wake`.
pub fn bounded<C: Send + 'static>(capacity: usize, wake: Wake) -> (Post<C>, Inbox<C>) {
    let shared = Arc::new(Shared {
        queue: Mutex::new(VecDeque::new()),
        closed: AtomicBool::new(false),
        wake,
        capacity,
    });
    (Post(shared.clone()), Inbox(shared))
}

impl<C: Send + 'static> Post<C> {
    /// Posts a command for the UI thread. Never blocks.
    pub fn post(&self, command: C) -> Result<(), Dropped> {
        if self.0.closed.load(Ordering::Acquire) {
            return Err(Dropped::Closed);
        }
        {
            let mut queue = self.0.queue.lock().unwrap_or_else(|p| p.into_inner());
            if queue.len() >= self.0.capacity {
                return Err(Dropped::Full);
            }
            queue.push_back(command);
        }
        (self.0.wake)();
        Ok(())
    }

    /// Posts on the reserved lane: never refused as full, only as closed. For what is bounded
    /// elsewhere and must not be lost to a flood of commands: a job's outcome (bounded by the
    /// pool), the core's own continuation of a drain. Crate-private, so nothing a caller can
    /// repeat reaches it.
    pub(crate) fn post_control(&self, command: C) -> Result<(), Dropped> {
        if self.0.closed.load(Ordering::Acquire) {
            return Err(Dropped::Closed);
        }
        self.0
            .queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push_back(command);
        (self.0.wake)();
        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        self.0.closed.load(Ordering::Acquire)
    }
}

impl<C> Inbox<C> {
    /// Takes everything posted so far, oldest first. Later posts wait for the next drain, so one
    /// drain is bounded by what was there when it began.
    pub fn drain(&self) -> Vec<C> {
        let mut queue = self.0.queue.lock().unwrap_or_else(|p| p.into_inner());
        queue.drain(..).collect()
    }

    /// Refuses every later post; what is already queued is dropped unread.
    pub fn close(&self) {
        self.0.closed.store(true, Ordering::Release);
        self.0
            .queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    pub fn len(&self) -> usize {
        self.0.queue.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn counting_wake() -> (Wake, Arc<AtomicUsize>) {
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        (
            Arc::new(move || {
                w.fetch_add(1, Ordering::Relaxed);
            }),
            wakes,
        )
    }

    #[test]
    fn posts_arrive_in_order_and_each_wakes_the_ui_thread() {
        let (wake, wakes) = counting_wake();
        let (post, inbox) = bounded::<u32>(8, wake);
        let other = post.clone();
        std::thread::spawn(move || {
            for i in 0..3 {
                other.post(i).unwrap();
            }
        })
        .join()
        .unwrap();
        assert_eq!(inbox.drain(), vec![0, 1, 2]);
        assert_eq!(wakes.load(Ordering::Relaxed), 3);
        assert!(inbox.is_empty());
    }

    #[test]
    fn a_full_queue_refuses_without_blocking_and_keeps_what_it_has() {
        let (wake, _) = counting_wake();
        let (post, inbox) = bounded::<u32>(2, wake);
        post.post(1).unwrap();
        post.post(2).unwrap();
        assert_eq!(post.post(3), Err(Dropped::Full));
        assert_eq!(inbox.drain(), vec![1, 2]);
        post.post(4).unwrap();
        assert_eq!(inbox.drain(), vec![4]);
    }

    #[test]
    fn a_post_after_shutdown_fails_harmlessly() {
        let (wake, wakes) = counting_wake();
        let (post, inbox) = bounded::<u32>(8, wake);
        post.post(1).unwrap();
        inbox.close();
        assert!(post.is_closed());
        assert_eq!(post.post(2), Err(Dropped::Closed));
        assert!(
            inbox.is_empty(),
            "what was queued is dropped, not applied after the end"
        );
        assert_eq!(
            wakes.load(Ordering::Relaxed),
            1,
            "a refused post does not wake anything"
        );
    }

    #[test]
    fn a_drain_is_bounded_by_what_was_there_when_it_began() {
        let (wake, _) = counting_wake();
        let (post, inbox) = bounded::<u32>(8, wake);
        post.post(1).unwrap();
        let first = inbox.drain();
        post.post(2).unwrap();
        assert_eq!(first, vec![1]);
        assert_eq!(inbox.drain(), vec![2]);
    }
}
