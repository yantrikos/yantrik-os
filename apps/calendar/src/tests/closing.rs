//! Closing the window while writes are in flight: the window closes at once, and the app runs on
//! without one (`fx.linger`) until the store has answered every write already sent, then ends. A
//! second launch meanwhile opens the window again and cancels the close. Before the framework this
//! was implicit: writes ran on the UI thread, so the close could not be handled before them.

use std::time::Duration;

use serde_json::json;
use yantrik_app::app::{Event, ExitReason, WindowId};

use super::{a_week, open, Fixture};

/// The form's Save pressed, its create queued and not yet run.
fn a_save_queued(f: &Fixture, title: &str) {
    f.app.ui(|ui| ui.invoke_add_event());
    let title = title.to_string();
    f.app.ui(move |ui| ui.invoke_save_event(title.into(), "2026-09-24".into(), "09:00".into(), "".into()));
    assert_eq!(f.app.jobs_waiting(), 1, "the create is on its way");
}

fn titles(f: &Fixture) -> Vec<String> {
    f.store.stored().into_iter().map(|e| e.title).collect()
}

#[test]
fn closing_the_window_with_a_save_queued_stores_it_and_then_ends() {
    let f = open(a_week());
    a_save_queued(&f, "Before closing");
    f.app.close_requested(WindowId::PRIMARY);
    assert!(f.app.windows().is_empty(), "the window is closed at once");
    assert!(f.app.lingering(), "and the app runs on without it");
    assert_eq!(f.app.exit_reason(), None, "it waits for the write already sent");
    let e = f.act("add_event", json!({ "title": "After closing", "date": "2026-09-24", "time": "10:00" })).unwrap_err();
    assert!(e.contains("Calendar is closing, so nothing more is written."), "{e}");
    f.app.run_jobs();
    assert_eq!(f.app.exit_reason(), Some(ExitReason::App), "the save landed, and the app ended itself");
    assert!(titles(&f).contains(&"Before closing".to_string()));
    assert!(!titles(&f).contains(&"After closing".to_string()));
}

/// A store that never answers does not keep a closed app alive: the linger's backstop ends it.
#[test]
fn a_close_waiting_on_a_store_that_never_answers_ends_at_the_backstop() {
    let f = open(a_week());
    f.app.ui(|ui| ui.invoke_delete_event(0));
    f.app.close_requested(WindowId::PRIMARY);
    f.app.advance(crate::update::CLOSE_BACKSTOP - Duration::from_secs(1));
    assert_eq!(f.app.exit_reason(), None);
    f.app.advance(Duration::from_secs(1));
    assert_eq!(f.app.exit_reason(), Some(ExitReason::LastWindowClosed), "ended without the store's answer");
}

/// With nothing in flight, closing the window ends the app at once, as the framework's default.
#[test]
fn closing_the_window_with_nothing_pending_ends_at_once() {
    let f = open(a_week());
    f.app.close_requested(WindowId::PRIMARY);
    assert_eq!(f.app.exit_reason(), Some(ExitReason::LastWindowClosed));
    assert!(!f.app.lingering());
}

/// Closed with a save queued, then opened again by the desktop's `present`: the close is
/// cancelled. The person can write again, and the app keeps running after the first write lands
/// and past the backstop (review of #921 at c17eaa7f, Q1).
#[test]
fn a_window_presented_during_the_close_wait_cancels_the_close() {
    let f = open(a_week());
    a_save_queued(&f, "First");
    f.app.close_requested(WindowId::PRIMARY);
    f.app.present().unwrap();
    assert_eq!(f.app.windows(), [WindowId::PRIMARY], "the window is open again");
    assert!(!f.app.lingering());
    let added = f.app.act("add_event", json!({ "title": "Second", "date": "2026-09-24", "time": "10:00" })).unwrap();
    f.app.run_jobs();
    assert_eq!(f.app.exit_reason(), None, "the app keeps running after the first write lands");
    assert_eq!(f.settled(added).unwrap()["added"], json!("Second"));
    f.app.advance(crate::update::CLOSE_BACKSTOP + Duration::from_secs(1));
    assert_eq!(f.app.exit_reason(), None, "no backstop is left armed");
    assert!(titles(&f).contains(&"First".to_string()) && titles(&f).contains(&"Second".to_string()));
}

/// A second launch handed over (`launch`) during the close wait cancels it too.
#[test]
fn a_second_launch_during_the_close_wait_cancels_the_close() {
    let f = open(a_week());
    f.app.ui(|ui| ui.invoke_delete_event(0));
    f.app.close_requested(WindowId::PRIMARY);
    f.app.launch_again().unwrap();
    f.app.run_jobs();
    assert_eq!(f.app.exit_reason(), None);
    assert_eq!(f.app.windows(), [WindowId::PRIMARY]);
    assert!(f.act("add_event", json!({ "title": "Back", "date": "2026-09-24", "time": "10:00" })).is_ok());
}

/// The review of #921 at 4d3b0ebd, Low 1: once the last write has landed and the app has ended
/// itself, a second launch does not bring it back.
#[test]
fn a_launch_after_the_app_has_ended_does_not_bring_it_back() {
    let f = open(a_week());
    a_save_queued(&f, "Last");
    f.app.close_requested(WindowId::PRIMARY);
    f.app.run_jobs();
    assert_eq!(f.app.exit_reason(), Some(ExitReason::App));
    let _ = f.app.launch_again();
    assert!(f.app.present().is_err());
    assert!(f.app.windows().is_empty());
    assert_eq!(f.app.exit_reason(), Some(ExitReason::App), "and still ends");
}

/// The review of #921 at 4d3b0ebd, Low 2: an activation that was already on its way when the
/// window closed cannot bring a closed window back, nor cancel the close.
#[test]
fn a_stale_activation_does_not_bring_back_a_closed_window() {
    let f = open(a_week());
    a_save_queued(&f, "Stays closed");
    f.app.close_requested(WindowId::PRIMARY);
    f.app.event(Event::Activated(WindowId::PRIMARY));
    assert!(f.app.windows().is_empty(), "still closed");
    assert!(f.app.lingering());
    let e = f.act("add_event", json!({ "title": "No", "date": "2026-09-24", "time": "10:00" })).unwrap_err();
    assert!(e.contains("Calendar is closing"), "{e}");
    f.app.run_jobs();
    assert_eq!(f.app.exit_reason(), Some(ExitReason::App), "the close went on to its end");
}
