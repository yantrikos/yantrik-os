//! The window closed while a file is written: it closes at once, and yPresent runs on with no
//! window until the save lands (`fx.linger`), then ends; a failure then is kept as a draft. A
//! second launch brings the window back; a person's close is cancelled by it and a logout never
//! is.

use serde_json::json;
use yantrik_app::app::{Event, ExitReason, WindowId};
use yantrik_app::documents::disk::Step;

use super::{act, failing, killed_and_started_again, none, outline, read_deck, settle, type_title, with_file};
use crate::command::{Command, WindowAction};

/// A saved deck open, edited, and its Save started but not landed.
fn saving() -> (yantrik_testing::TestApp<crate::YPresent>, std::path::PathBuf) {
    let (app, path) = with_file("linger", &["One"]);
    type_title(&app, "Two");
    app.run(Command::Window { action: WindowAction::Save, canvas: super::canvas(&app) });
    assert_eq!(app.jobs_waiting(), 1, "the save is in flight");
    (app, path)
}

#[test]
fn the_window_closed_during_a_save_closes_at_once_and_the_app_ends_when_it_lands() {
    let (app, path) = saving();
    app.close_requested(WindowId::PRIMARY);
    assert!(app.windows().is_empty(), "the window is really closed, not hidden");
    assert!(app.lingering() && app.exit_reason().is_none());
    settle(&app);
    assert_eq!(read_deck(&path).slides[0].title, "Two");
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
}

#[test]
fn the_call_a_save_was_for_is_answered_before_a_linger_ends() {
    let (app, path) = with_file("linger-call", &["One"]);
    act(&app, "set_slide", json!({ "index": 0, "title": "Two" })).unwrap();
    let pending = app.act("save", none()).unwrap();
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    let answer = pending.settle().expect("answered, not refused by the shutdown");
    assert_eq!(answer["path"], json!(path.display().to_string()));
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
}

#[test]
fn a_save_that_fails_while_the_window_is_gone_keeps_its_deck_for_the_next_start() {
    let (app, path) = with_file("linger-fails", &["One"]);
    let disk = failing(&app);
    disk.fail(Step::Write, "talk.ydeck", Some(1));
    type_title(&app, "Two");
    press_save_without_landing(&app);
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
    assert_eq!(read_deck(&path).slides[0].title, "One");
    app.shutdown();
    let again = killed_and_started_again(app);
    assert_eq!(outline(&again), vec!["Two"], "the deck came back");
}

fn press_save_without_landing(app: &yantrik_testing::TestApp<crate::YPresent>) {
    app.run(Command::Window { action: WindowAction::Save, canvas: super::canvas(app) });
}

#[test]
fn a_second_launch_during_the_wait_brings_the_window_back_and_the_app_runs_on() {
    let (app, path) = saving();
    app.close_requested(WindowId::PRIMARY);
    app.launch_again().expect("the hand-over is taken");
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
    assert!(!app.lingering());
    settle(&app);
    assert_eq!(read_deck(&path).slides[0].title, "Two");
    assert!(app.exit_reason().is_none(), "the person is back");
    act(&app, "add_slide", json!({ "title": "More" })).expect("and it takes calls again");
    app.advance(std::time::Duration::from_secs(11));
    assert!(app.exit_reason().is_none(), "the backstop is the framework's, and was cancelled");
}

#[test]
fn a_logout_during_the_wait_is_never_cancelled_by_a_launch() {
    let (app, _) = saving();
    app.close_requested(WindowId::PRIMARY);
    app.launch_again().expect("the hand-over is taken");
    app.event(Event::ExitRequested(ExitReason::Logout));
    assert!(app.exit_reason().is_some(), "a logout ends it whatever the person did a moment ago");
}

#[test]
fn a_call_that_needs_a_window_is_told_there_is_none_while_it_lingers() {
    let (app, _) = saving();
    app.close_requested(WindowId::PRIMARY);
    let err = act(&app, "show", none()).unwrap_err();
    assert!(err.contains("window is closed while it finishes a save"), "{err}");
}
