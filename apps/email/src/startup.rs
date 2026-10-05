//! Getting the window up before the mail is.
//!
//! The first look at the mail service ran on the UI thread before the window existed: start the
//! service (up to two seconds), then list the folders and the inbox, each an IMAP round trip of
//! up to ten. On the test VM the window came up eight seconds after `open_app email` was
//! accepted, and for those eight seconds nothing on screen said Email was coming — the window in
//! front stayed in front, and it looked as if the wrong app had opened.
//!
//! So the window is shown first, saying it is starting, and the look runs on a worker in two
//! halves: which account (local and quick once the service is up), then that account's mailbox.
//! The screen says "Connecting to <account>…" between them.

use std::cell::Cell;
use std::time::Instant;

use slint::ComponentHandle;

use crate::EmailApp;

/// Run `load` on a worker thread and hand what it returns to `deliver`, without waiting for
/// either. The caller is the UI thread, which must be drawing while the mail service is asked.
pub fn off_the_ui_thread<T, L, D>(load: L, deliver: D)
where
    T: Send + 'static,
    L: FnOnce() -> T + Send + 'static,
    D: FnOnce(T) + Send + 'static,
{
    std::thread::Builder::new()
        .name("email-first-load".into())
        .spawn(move || deliver(load()))
        .expect("a thread for the first look at the mail service");
}

/// Log how long the window took to draw for the first time, once.
///
/// From `started`, which `main` takes as its first act, so the number is process start to a
/// window on screen — the thing a person waits through.
pub fn log_first_frame(app: &EmailApp, started: Instant) {
    let logged = Cell::new(false);
    let notifier = app.window().set_rendering_notifier(move |state, _| {
        if matches!(state, slint::RenderingState::AfterRendering) && !logged.replace(true) {
            first_frame(started);
        }
    });
    // The software renderer has no notifier. The event loop's first turn is the window shown.
    if notifier.is_err() {
        slint::Timer::single_shot(std::time::Duration::ZERO, move || first_frame(started));
    }
}

fn first_frame(started: Instant) {
    tracing::info!(ms = started.elapsed().as_millis() as u64, "email: first frame");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// The first look at the mail service does not run on the thread that asked for it, and the
    /// asking thread is not kept waiting for it: a mailbox that takes eight seconds to open is
    /// eight seconds the window would otherwise not be drawn.
    #[test]
    fn the_first_load_runs_off_the_calling_thread_and_does_not_block_it() {
        let caller = std::thread::current().id();
        let (release, held) = mpsc::channel::<()>();
        let (done, delivered) = mpsc::channel();
        let asked = Instant::now();
        off_the_ui_thread(
            move || {
                held.recv().unwrap(); // a mailbox that has not answered yet
                std::thread::current().id()
            },
            move |loader| done.send(loader).unwrap(),
        );
        assert!(asked.elapsed() < Duration::from_millis(500), "returned while the load was held");
        assert!(delivered.try_recv().is_err(), "nothing delivered before the load finished");
        release.send(()).unwrap();
        let loader = delivered.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_ne!(loader, caller, "the load ran on a worker");
    }

    /// Before the service answers the app is starting: not unreachable, and not "no account".
    #[test]
    fn starting_is_neither_a_failure_nor_no_account() {
        let starting = crate::state::MailState::Starting;
        assert_eq!(starting.service_word(), "starting");
        assert_eq!(starting.has_account(), None);
        assert!(starting.notice().is_empty(), "nothing has gone wrong yet");
    }
}
