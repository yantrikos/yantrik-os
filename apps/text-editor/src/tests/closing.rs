//! Closing a tab, the window and the editor: a tab with unsaved changes is asked about, one
//! question at a time; the session ending asks nothing and keeps every unsaved text in its draft.

use serde_json::json;
use yantrik_app::app::{Event, ExitReason, WindowId};
use yantrik_app::documents::disk::Step;

use super::{act, draft_file, failing, file, killed_and_started_again, notice, press, settle, state, tabs, text, typed, with_file};
use crate::command::{Command, WindowAction};

#[test]
fn closing_an_unsaved_tab_asks_and_cancel_keeps_it() {
    let app = super::started();
    act(&app, "new", json!({ "text": "keep me" })).unwrap();
    press(&app, WindowAction::Close);
    assert_eq!(state(&app)["dialog"], 3);
    assert_eq!(state(&app)["dialog_is"], "Untitled has unsaved changes.");
    assert_eq!(app.ui(|ui| ui.get_close_label().to_string()), "Untitled has unsaved changes.");
    press(&app, WindowAction::Cancel);
    assert_eq!((state(&app)["dialog"].clone(), tabs(&app)), (json!(0), 2));
    assert_eq!(text(&app), "keep me");
}

#[test]
fn discard_closes_the_tab_and_its_draft_goes_with_it() {
    let app = super::started();
    act(&app, "new", json!({ "text": "throw away" })).unwrap();
    app.run_writes();
    let draft = draft_file(&app);
    assert!(draft.exists());
    act(&app, "close", json!({})).unwrap();
    let answer = act(&app, "discard", json!({})).unwrap();
    assert_eq!(answer["discarded"], "Untitled", "{answer}");
    assert_eq!(answer["closed"], true, "{answer}");
    app.run_writes();
    assert!(!draft.exists());
    assert_eq!(tabs(&app), 1);
}

#[test]
fn save_in_the_prompt_closes_the_tab_once_it_is_written() {
    let (app, path) = with_file("save-close", "v1\n");
    act(&app, "new", json!({})).unwrap();
    act(&app, "select_tab", json!({ "index": 0 })).unwrap();
    typed(&app, "v2\n");
    act(&app, "close", json!({})).unwrap();
    let answer = act(&app, "save", json!({})).unwrap();
    assert_eq!(answer["closed_the_tab"], true, "{answer}");
    assert_eq!(answer["saved"], true, "{answer}");
    assert_eq!(answer["bytes"], 3, "{answer}");
    assert_eq!(file(&path), "v2\n");
    assert_eq!(tabs(&app), 1);
}

#[test]
fn a_save_that_fails_in_the_prompt_keeps_the_prompt_up_with_the_reason() {
    let (app, path) = with_file("save-close-fails", "v1\n");
    typed(&app, "v2\n");
    let disk = failing(&app);
    disk.fail(Step::Write, "notes.txt", Some(1));
    press(&app, WindowAction::Close);
    press(&app, WindowAction::SaveClose);
    assert_eq!(state(&app)["dialog"], 3, "the tab is not closed");
    assert!(app.model(|m| m.dialog_error.starts_with("Save failed")), "the prompt says why");
    assert_eq!(file(&path), "v1\n");
    press(&app, WindowAction::SaveClose);
    assert_eq!(file(&path), "v2\n", "Save again, with the disk back, closes it");
    assert_eq!(state(&app)["dialog"], 0);
}

#[test]
fn closing_the_window_asks_about_each_unsaved_tab_in_turn() {
    let (app, path) = with_file("window", "v1\n");
    typed(&app, "v2\n");
    act(&app, "new", json!({ "text": "second" })).unwrap();
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(state(&app)["dialog_is"], "notes.txt has unsaved changes.", "the first, brought forward");
    assert_eq!(app.model(|m| m.active), 0);
    press(&app, WindowAction::SaveClose);
    assert_eq!(file(&path), "v2\n");
    assert_eq!(state(&app)["dialog_is"], "Untitled has unsaved changes.", "then the next");
    assert!(app.exit_reason().is_none());
    // A second close while one asks needs no words.
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(state(&app)["dialog_is"], "Untitled has unsaved changes.");
    press(&app, WindowAction::Discard);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed), "nothing left to ask: the window closes");
}

#[test]
fn cancel_in_the_windows_close_keeps_everything() {
    let (app, _) = with_file("window-cancel", "v1\n");
    typed(&app, "v2\n");
    app.close_requested(WindowId::PRIMARY);
    press(&app, WindowAction::Cancel);
    assert!(app.exit_reason().is_none());
    assert_eq!((state(&app)["dialog"].clone(), text(&app)), (json!(0), "v2\n".to_string()));
    // A tab's own close while nothing asks still works.
    press(&app, WindowAction::Close);
    assert_eq!(state(&app)["dialog"], 3);
}

#[test]
fn a_window_with_nothing_unsaved_closes_at_once() {
    let app = super::started();
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn an_untitled_tab_saved_from_the_prompt_asks_where_and_closes_once_written() {
    let app = super::started();
    typed(&app, "untitled text");
    app.close_requested(WindowId::PRIMARY);
    press(&app, WindowAction::SaveClose);
    assert_eq!(state(&app)["dialog"], 2, "Save As, from the prompt");
    let path = super::scratch("prompt-save-as").join("kept.txt");
    press(&app, WindowAction::Confirm { path: path.display().to_string() });
    assert_eq!(file(&path), "untitled text");
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn a_logout_asks_nothing_and_keeps_every_unsaved_text() {
    for signal in [true, false] {
        let (app, path) = with_file("logout", "v1\n");
        typed(&app, "unsaved at logout\n");
        act(&app, "new", json!({ "text": "and this" })).unwrap();
        if signal {
            app.event(Event::ExitRequested(ExitReason::Logout));
        } else {
            app.display_gone();
        }
        assert!(app.exit_reason().is_some(), "ended in the command that answered");
        assert_eq!(state(&app)["dialog"], 0, "no one was asked");
        app.shutdown();
        let again = killed_and_started_again(app);
        assert_eq!(tabs(&again), 2, "signal={signal}");
        assert_eq!(text(&again), "unsaved at logout\n");
        assert_eq!(file(&path), "v1\n");
    }
}

#[test]
fn a_person_quitting_is_the_windows_close() {
    let app = super::started();
    typed(&app, "x");
    app.event(Event::ExitRequested(ExitReason::Person));
    assert_eq!(state(&app)["dialog"], 3);
    assert!(app.exit_reason().is_none());
}

/// One question at a time: the window offers only the prompt's answers while it asks, and no
/// dialog shows without a live question behind it.
#[test]
fn one_prompt_at_a_time_and_none_without_its_question() {
    let app = super::started();
    act(&app, "new", json!({ "text": "a" })).unwrap();
    act(&app, "new", json!({ "text": "b" })).unwrap();
    press(&app, WindowAction::Close);
    let asked = app.model(|m| m.closing.as_ref().map(|c| c.tab));
    // Another tab's close, a new tab, an open: none takes over the question.
    let first = app.model(|m| m.tabs[1].id);
    app.run(Command::CloseTab(first));
    press(&app, WindowAction::New);
    press(&app, WindowAction::Open);
    settle(&app);
    assert_eq!(app.model(|m| m.closing.as_ref().map(|c| c.tab)), asked);
    assert_eq!((state(&app)["dialog"].clone(), tabs(&app)), (json!(3), 3));
    // The tab asked about saved by another way: the question is no longer true, so it goes.
    let path = super::scratch("prompt-goes").join("b.txt");
    act(&app, "save_as", json!({ "path": path.display().to_string() })).unwrap();
    assert_eq!(state(&app)["dialog"], 0, "{}", notice(&app));
    assert!(app.model(|m| m.closing.is_none()));
}

/// A Save As the person opens over the prompt saves the tab and puts the question away, as it
/// always did; only the Save As the prompt's own *Save* opened closes the tab.
#[test]
fn a_save_as_over_the_prompt_keeps_the_tab_and_puts_the_question_away() {
    let app = super::started();
    act(&app, "new", json!({ "text": "keep open" })).unwrap();
    press(&app, WindowAction::Close);
    press(&app, WindowAction::SaveAs);
    assert_eq!(state(&app)["dialog"], 2);
    let path = super::scratch("over-prompt").join("kept.txt");
    press(&app, WindowAction::Confirm { path: path.display().to_string() });
    assert_eq!(file(&path), "keep open");
    assert_eq!((state(&app)["dialog"].clone(), tabs(&app)), (json!(0), 2), "{}", notice(&app));
}
