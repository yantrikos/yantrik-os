//! The window closed while a file is written: it closes at once, and yDoc runs on with no window
//! until the save lands (`fx.linger`), then ends; a failure then is kept as a draft. A second
//! launch brings the window back; a person's close is cancelled by it and a logout never is.

use serde_json::json;
use yantrik_app::app::{Event, ExitReason, WindowId};
use yantrik_app::documents::disk::Step;

use super::{act, failing, file, killed_and_started_again, none, notice, settle, state, text, typed, with_file};
use crate::command::{Command, WindowAction};

/// A saved file open, edited, and its Save started but not landed.
fn saving() -> (yantrik_testing::TestApp<crate::YDoc>, std::path::PathBuf) {
    let (app, path) = with_file("linger", "v1\n");
    typed(&app, "v2\n");
    app.run(Command::Window(WindowAction::Save));
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
    assert_eq!(file(&path), "v2\n");
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
}

#[test]
fn the_call_a_save_was_for_is_answered_before_a_linger_ends() {
    let (app, path) = with_file("linger-call", "v1\n");
    typed(&app, "v2\n");
    let pending = app.act("save", none()).unwrap();
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    let answer = pending.settle().expect("answered, not refused by the shutdown");
    assert_eq!(answer["saved"], json!(path));
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
}

#[test]
fn a_save_that_fails_while_the_window_is_gone_keeps_its_text_for_the_next_start() {
    let (app, path) = with_file("linger-fails", "v1\n");
    let disk = failing(&app);
    disk.fail(Step::Write, "notes.md", Some(1));
    typed(&app, "v2\n");
    app.run(Command::Window(WindowAction::Save));
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
    assert_eq!(file(&path), "v1\n");
    app.shutdown();
    let again = killed_and_started_again(app);
    assert_eq!(text(&again), "v2\n", "the text came back");
}

#[test]
fn a_second_launch_during_the_wait_brings_the_window_back_and_the_app_runs_on() {
    let (app, path) = saving();
    app.close_requested(WindowId::PRIMARY);
    app.launch_again().expect("the hand-over is taken");
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
    assert!(!app.lingering());
    settle(&app);
    assert_eq!(file(&path), "v2\n");
    assert!(app.exit_reason().is_none(), "the person is back");
    act(&app, "append", json!({ "text": "more\n" })).expect("and it takes calls again");
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
fn the_window_closed_while_a_file_is_read_waits_for_it_then_goes_on() {
    let (app, path) = with_file("close-reading", "v1\n");
    let other = path.with_file_name("other.md");
    std::fs::write(&other, "other\n").unwrap();
    typed(&app, "unsaved\n");
    let pending = app.act("open", json!({ "path": other.display().to_string(), "discard": true })).unwrap();
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(notice(&app), crate::update::closing::CLOSING_WHEN_DONE);
    assert_eq!(app.windows(), vec![WindowId::PRIMARY], "the window stays while the file is read");
    settle(&app);
    pending.settle().unwrap();
    // The document read is clean: the close as asked goes ahead.
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
    assert_eq!(state(&app)["prompt_open"], false);
}
