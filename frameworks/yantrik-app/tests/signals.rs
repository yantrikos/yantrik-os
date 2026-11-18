//! A real SIGTERM, raised at this test process, through the handler the runner installs
//! (`yantrik_app::quit::listen`): heard on the signal thread, posted, and raised on the app's UI
//! thread as one logout however many arrive; then the deadline ends an app that has not ended.
//! Its own test binary, so no other test shares the process the signal is raised at.

#![cfg(unix)]

mod common;

use std::time::{Duration, Instant};

use yantrik_app::app::ExitReason;
use yantrik_testing::TestApp;

use common::quitter::{ui_thread, Command, Quitter};

/// Ticks the app until `done` holds, or panics after a few seconds.
fn until(app: &TestApp<Quitter>, what: &str, done: impl Fn(&TestApp<Quitter>) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.tick();
        if done(app) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: not within 10 s");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_sigterm_at_this_process_reaches_the_app_once_and_the_deadline_ends_it() {
    let app = TestApp::<Quitter>::start();
    app.run(Command::Edit);
    let deadline = Duration::from_secs(2);
    yantrik_app::quit::listen::<Quitter>(app.post(), deadline).expect("the handler is installed");
    let raised = Instant::now();
    signal_hook::low_level::raise(signal_hook::consts::SIGTERM).expect("SIGTERM raised");
    signal_hook::low_level::raise(signal_hook::consts::SIGTERM).expect("SIGTERM raised again");

    until(&app, "the logout", |app| app.model(|m| m.logouts) == 1);
    assert_eq!(app.model(|m| m.logout_thread), Some(ui_thread(&app)), "on the UI thread");
    assert_eq!(app.model(|m| m.asked), 0, "no one is asked at a logout");
    // The second signal has been heard by now or will be; either way, not raised again.
    std::thread::sleep(Duration::from_millis(50));
    app.tick();
    assert_eq!(app.model(|m| m.logouts), 1, "two signals, one logout");
    assert_eq!(app.exit_reason(), None, "the app is saving");

    // The save never lands: the deadline ends the app.
    until(&app, "the deadline", |app| app.exit_reason().is_some());
    assert_eq!(app.exit_reason(), Some(ExitReason::Logout));
    assert!(raised.elapsed() >= deadline, "not before the deadline");
}
