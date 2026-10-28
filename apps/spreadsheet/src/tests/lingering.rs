//! The window closed while a file is written: it closes at once, and ySheets runs on with no window
//! until the save lands (`fx.linger`), then ends; a failure then is kept as a draft. A second launch
//! brings the window back; a person's close is cancelled by it and a logout never is.

use serde_json::json;
use yantrik_app::app::{Event, ExitReason, WindowId};
use yantrik_app::documents::disk::Step;
use yantrik_testing::TestApp;

use super::{act, enter, failing, file, killed_and_started_again, none, raw, settle, with_file};
use crate::YSheets;

/// A saved sheet open, edited, and its Save started but not landed.
fn saving() -> (TestApp<YSheets>, std::path::PathBuf) {
    let (app, path) = with_file("linger", "1\r\n");
    enter(&app, "two");
    super::click(&app, "A1");
    app.ui(|ui| ui.invoke_command("save".into()));
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
    assert_eq!(file(&path), "two\r\n");
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
}

#[test]
fn the_call_a_save_was_for_is_answered_before_a_linger_ends() {
    let (app, path) = with_file("linger-call", "1\r\n");
    act(&app, "set_cell", json!({ "cell": "A1", "text": "two" })).unwrap();
    let pending = app.act("save_over", none()).unwrap();
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    let answer = pending.settle().expect("answered, not refused by the shutdown");
    assert_eq!(answer["path"], json!(path.display().to_string()));
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
}

#[test]
fn a_save_that_fails_while_the_window_is_gone_keeps_its_sheet_for_the_next_start() {
    let (app, path) = with_file("linger-fails", "1\r\n");
    let disk = failing(&app);
    disk.fail(Step::Write, "budget.csv", Some(1));
    enter(&app, "two");
    super::click(&app, "A1");
    app.ui(|ui| ui.invoke_command("save".into()));
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
    assert_eq!(file(&path), "1\r\n");
    app.shutdown();
    let again = killed_and_started_again(app);
    assert_eq!(raw(&again, "A1"), "two", "the sheet came back");
}

#[test]
fn a_second_launch_during_the_wait_brings_the_window_back_and_the_app_runs_on() {
    let (app, path) = saving();
    app.close_requested(WindowId::PRIMARY);
    app.launch_again().expect("the hand-over is taken");
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
    assert!(!app.lingering());
    settle(&app);
    assert_eq!(file(&path), "two\r\n");
    assert!(app.exit_reason().is_none(), "the person is back");
    act(&app, "set_cell", json!({ "cell": "B1", "text": "More" })).expect("and it takes calls again");
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
fn a_file_opened_while_the_app_lingers_is_not_put_in_a_window_that_is_not_there() {
    let (app, _) = saving();
    app.close_requested(WindowId::PRIMARY);
    let other = super::scratch("linger-open").join("other.csv");
    std::fs::write(&other, "other\r\n").unwrap();
    let err = act(&app, "open", json!({ "path": other.display().to_string() })).unwrap_err();
    assert!(err.contains("cannot run while ySheets is reading or writing a file"), "{err}");
    assert!(app.windows().is_empty(), "no window was made for it");
}
