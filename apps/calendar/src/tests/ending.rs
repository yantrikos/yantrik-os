//! How Calendar ends when it is told to (#923): a signal or the display going is a logout, with
//! nobody there to answer. A write already sent to the store is let land first, so a Save pressed
//! a moment before a logout is not lost, and then the app ends. Closing the window is `closing.rs`.

use serde_json::json;
use yantrik_app::app::WindowId;
use yantrik_app::cx::Inbound;
use yantrik_app::quit::Signal;

use super::{a_week, open, Fixture};

/// A signal, posted from a thread of its own as the signal thread posts it, then heard.
fn signal(f: &Fixture, signal: Signal) {
    let post = f.app.post();
    std::thread::spawn(move || post.post(Inbound::Signal(signal)).is_ok())
        .join()
        .expect("the posting thread");
    f.app.tick();
}

#[test]
fn a_logout_by_signal_lets_a_save_in_flight_land_and_then_ends() {
    let f = open(a_week());
    f.app.ui(|ui| ui.invoke_add_event());
    f.app.ui(|ui| ui.invoke_save_event("Pressed just before".into(), "2026-09-24".into(), "09:00".into(), "".into()));
    assert_eq!(f.app.jobs_waiting(), 1, "the create is on its way");
    signal(&f, Signal::Terminate);
    assert_eq!(f.app.exit_reason(), None, "it waits for the write already sent");
    let e = f.act("add_event", json!({ "title": "Too late", "date": "2026-09-24", "time": "10:00" })).unwrap_err();
    assert!(e.contains("Calendar is closing"), "{e}");
    f.app.run_jobs();
    assert!(f.app.exit_reason().is_some(), "the save landed, and the app ended");
    let titles: Vec<String> = f.store.stored().into_iter().map(|e| e.title).collect();
    assert!(titles.contains(&"Pressed just before".to_string()), "{titles:?}");
    assert!(!titles.contains(&"Too late".to_string()));
}

#[test]
fn the_display_going_with_nothing_pending_ends_the_app() {
    let f = open(a_week());
    assert_eq!(f.app.jobs_waiting(), 0);
    f.app.display_gone();
    assert!(f.app.exit_reason().is_some(), "nothing to wait for");
    let _ = f.app.shutdown();
}

/// A signal while nothing is being written ends the app at once, as the framework's default did.
#[test]
fn a_logout_with_nothing_pending_ends_at_once() {
    let f = open(a_week());
    signal(&f, Signal::HangUp);
    assert!(f.app.exit_reason().is_some());
}

/// A logout's ending is not a close: the window activated meanwhile does not cancel it.
#[test]
fn a_logout_is_not_cancelled_by_the_window_coming_back() {
    let f = open(a_week());
    f.app.ui(|ui| ui.invoke_add_event());
    f.app.ui(|ui| ui.invoke_save_event("Pressed just before".into(), "2026-09-24".into(), "09:00".into(), "".into()));
    signal(&f, Signal::Terminate);
    f.app.deactivate(WindowId::PRIMARY);
    f.app.activate(WindowId::PRIMARY);
    let e = f.act("add_event", json!({ "title": "Too late", "date": "2026-09-24", "time": "10:00" }));
    assert!(e.unwrap_err().contains("Calendar is closing"));
    f.quiet();
    assert!(f.app.exit_reason().is_some(), "the save landed, and the logout ended the app");
}
