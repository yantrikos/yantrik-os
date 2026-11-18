//! The window closed while a save is in flight: it closes at once, and the editor runs on with no
//! window until the save lands (`fx.linger`), then ends; a failure then is kept as a draft.

use serde_json::json;
use yantrik_app::app::{ExitReason, WindowId};
use yantrik_app::documents::disk::Step;

use super::{act, failing, file, killed_and_started_again, notice, settle, state, tabs, text, typed, with_file};
use crate::command::{Command, WindowAction};

/// A saved file open, edited, and its Save started but not landed.
fn saving() -> (yantrik_testing::TestApp<crate::TextEditor>, std::path::PathBuf) {
    let (app, path) = with_file("linger", "v1\n");
    typed(&app, "v2\n");
    app.run(Command::Window(WindowAction::Save));
    assert_eq!(app.jobs_waiting(), 1, "the save is in flight");
    (app, path)
}

#[test]
fn the_window_closed_during_a_save_closes_at_once_and_the_editor_ends_when_it_lands() {
    let (app, path) = saving();
    app.close_requested(WindowId::PRIMARY);
    assert!(app.windows().is_empty(), "the window is really closed, not hidden");
    assert!(app.lingering() && app.exit_reason().is_none());
    settle(&app);
    assert_eq!(file(&path), "v2\n");
    assert_eq!(app.exit_reason(), Some(ExitReason::App));
}

#[test]
fn a_save_that_fails_while_the_window_is_gone_keeps_its_text_for_the_next_start() {
    let (app, path) = with_file("linger-fails", "v1\n");
    let disk = failing(&app);
    disk.fail(Step::Write, "notes.txt", Some(1));
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
fn a_second_launch_during_the_wait_brings_the_window_back_and_the_editor_runs_on() {
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

/// A saved file open and edited, another tab unsaved, and the first one's Save in flight.
fn saving_beside_another() -> (yantrik_testing::TestApp<crate::TextEditor>, std::path::PathBuf) {
    let (app, path) = with_file("linger-other", "x
");
    act(&app, "new", json!({ "text": "unsaved" })).unwrap();
    act(&app, "select_tab", json!({ "index": 0 })).unwrap();
    typed(&app, "y
");
    app.run(Command::Window(WindowAction::Save));
    assert_eq!(app.jobs_waiting(), 1, "the save is in flight");
    (app, path)
}

#[test]
fn the_window_closed_during_a_save_with_another_tab_unsaved_asks_about_it_once_the_save_lands() {
    let (app, path) = saving_beside_another();
    app.close_requested(WindowId::PRIMARY);
    // Taken, not refused: the window stays while the save is in flight, and says what it waits for.
    assert_eq!(notice(&app), crate::update::closing::CLOSING_WHEN_DONE);
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
    assert_eq!(state(&app)["dialog"], 0, "no question while the save is in flight");
    settle(&app);
    assert_eq!(file(&path), "y
");
    // The close as asked: the usual prompt over the tab still unsaved, the window staying for it.
    assert_eq!(state(&app)["dialog"], 3);
    assert_eq!(state(&app)["dialog_is"], "Untitled has unsaved changes.");
    assert_eq!(app.windows(), vec![WindowId::PRIMARY]);
    assert!(app.exit_reason().is_none());
    super::press(&app, WindowAction::Discard);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn the_window_closed_during_a_save_that_fails_stays_open_and_says_why() {
    let (app, path) = with_file("linger-other-fails", "x
");
    act(&app, "new", json!({ "text": "unsaved" })).unwrap();
    act(&app, "select_tab", json!({ "index": 0 })).unwrap();
    typed(&app, "y
");
    let disk = failing(&app);
    disk.fail(Step::Write, "notes.txt", Some(1));
    app.run(Command::Window(WindowAction::Save));
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    assert_eq!(file(&path), "x
");
    assert!(notice(&app).starts_with("Save failed: injected failure"), "{}", notice(&app));
    assert!(notice(&app).ends_with(crate::update::closing::NOT_CLOSED), "{}", notice(&app));
    assert_eq!((app.windows(), tabs(&app)), (vec![WindowId::PRIMARY], 2), "everything stays open");
    assert_eq!(state(&app)["dialog"], 0);
    assert!(app.exit_reason().is_none());
    assert_eq!(text(&app), "y
");
    // Nothing is left waiting: the next close is asked as any other.
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(state(&app)["dialog"], 3);
}
