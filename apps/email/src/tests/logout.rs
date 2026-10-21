//! The app ending (#923): a logout, a signal, the display going, a person's quit, or the window
//! closing. The composer's text is kept without asking, a send in flight leaves its text with the
//! warning that whether it went is not known, and nothing is sent.

use serde_json::json;
use yantrik_app::app::{Event, ExitReason, WindowId};
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::{App, Paths};
use yantrik_testing::TestApp;

use super::compose::{draft, type_into_composer};
use super::{notice, quiet, start_at, with_inbox};
use crate::Email;

/// How the session ends the app in each test.
#[derive(Clone, Copy)]
enum Ending {
    /// SIGTERM or SIGHUP: the runner raises the logout.
    Signal,
    /// The compositor went: the runner's loop ended with no one having asked.
    DisplayGone,
    /// The person closed the window.
    WindowClosed,
    /// The person quit (`fx.request_quit()`).
    Quit,
}

fn end(app: TestApp<Email>, how: Ending) {
    match how {
        Ending::Signal => app.event(Event::ExitRequested(ExitReason::Logout)),
        Ending::DisplayGone => app.display_gone(),
        Ending::WindowClosed => app.close_requested(WindowId::PRIMARY),
        Ending::Quit => app.event(Event::ExitRequested(ExitReason::Person)),
    }
    let _ = app.shutdown();
    drop(app);
}

fn started_at(paths: &Paths) -> TestApp<Email> {
    let mail = with_inbox().answering("email.send_message", json!({}));
    let (app, _) = start_at(paths.clone(), &mail, FakeMind::new());
    quiet(&app);
    app
}

/// What Compose opens after a restart at `paths`: its body and its status line.
fn compose_after_restart(paths: &Paths) -> (String, String, TestApp<Email>) {
    let app = started_at(paths);
    app.ui(|ui| ui.invoke_compose_new());
    let (body, status) = app.ui(|ui| (ui.get_compose_body().to_string(), ui.get_email_draft_status().to_string()));
    (body, status, app)
}

fn unsaved_composer_is_kept(how: Ending) {
    let paths = Paths::temporary(Email::ID);
    let app = started_at(&paths);
    app.ui(|ui| ui.invoke_compose_new());
    type_into_composer(&app, &draft("a@example.com", "half written, never saved"));
    end(app, how);
    let (body, status, app) = compose_after_restart(&paths);
    assert_eq!(body, "half written, never saved");
    assert_eq!(status, "Draft restored", "an ordinary draft: nothing was sent");
    assert_eq!(notice(&app), "The message you were writing when Email closed is kept as a draft; Compose reopens it.");
}

fn send_in_flight_is_offered_with_the_warning(how: Ending) {
    let paths = Paths::temporary(Email::ID);
    let app = started_at(&paths);
    app.ui(|ui| ui.invoke_compose_new());
    type_into_composer(&app, &draft("a@example.com", "maybe went"));
    app.ui(|ui| ui.invoke_send_email("a@example.com".into(), "".into(), "".into(), "Plans".into(), "maybe went".into()));
    // An edit made while it is out: the newest text is what is kept, with the warning.
    type_into_composer(&app, &draft("a@example.com", "maybe went, and an edit"));
    end(app, how);
    let (body, status, app) = compose_after_restart(&paths);
    assert_eq!(body, "maybe went, and an edit");
    assert_eq!(status, "Not known whether this was sent");
    assert_eq!(notice(&app), "Whether this message was already sent is not known. Look in Sent before sending it again.");
}

#[test]
fn a_logout_with_an_unsaved_composer_keeps_the_text_for_the_next_start() {
    unsaved_composer_is_kept(Ending::Signal);
}

#[test]
fn a_logout_with_a_send_in_flight_offers_its_text_with_the_warning() {
    send_in_flight_is_offered_with_the_warning(Ending::Signal);
}

#[test]
fn the_display_going_is_the_same_logout() {
    unsaved_composer_is_kept(Ending::DisplayGone);
    send_in_flight_is_offered_with_the_warning(Ending::DisplayGone);
}

/// A logout with nothing in the composer leaves nothing behind.
#[test]
fn a_logout_with_the_composer_closed_keeps_nothing() {
    let paths = Paths::temporary(Email::ID);
    end(started_at(&paths), Ending::Signal);
    let (body, status, _app) = compose_after_restart(&paths);
    assert_eq!((body.as_str(), status.as_str()), ("", ""));
}

/// Review M5 (#920): closing the window with a half-written message keeps it, as a logout does,
/// and so does a person's quit. Nothing is sent.
#[test]
fn closing_the_window_or_quitting_keeps_the_composer() {
    unsaved_composer_is_kept(Ending::WindowClosed);
    unsaved_composer_is_kept(Ending::Quit);
    send_in_flight_is_offered_with_the_warning(Ending::WindowClosed);
}

/// Review L11 (#920): a draft save still queued behind a write in flight when the app ends is
/// kept, though the composer is closed.
#[test]
fn a_draft_save_still_queued_at_the_end_is_kept() {
    for how in [Ending::Signal, Ending::WindowClosed] {
        let paths = Paths::temporary(Email::ID);
        let app = started_at(&paths);
        app.ui(|ui| ui.invoke_compose_new());
        type_into_composer(&app, &draft("a@example.com", "v1 saved"));
        app.ui(|ui| ui.invoke_save_draft());
        type_into_composer(&app, &draft("a@example.com", "v2 queued behind it"));
        app.ui(|ui| ui.invoke_cancel_compose());
        assert!(app.model(|m| m.composer.queued.is_some()), "the edit waits behind the first write");
        end(app, how);
        let (body, _, _app) = compose_after_restart(&paths);
        assert_eq!(body, "v2 queued behind it");
    }
}
