//! The unsaved-changes prompt in front of New, Open and Close; the window and the session ending.
//! One question at a time, and nothing the person typed is dropped without being asked about.

use serde_json::json;
use yantrik_app::app::{Event, ExitReason, WindowId};
use yantrik_app::documents::disk::Step;

use super::{act, draft_file, failing, file, killed_and_started_again, notice, opened, path_of, press, scratch, started, state, text, typed, with_file};
use crate::command::WindowAction;

fn asking(app: &yantrik_testing::TestApp<crate::YDoc>) -> bool {
    app.model(crate::prompt::asking)
}

#[test]
fn new_over_unsaved_text_asks_and_cancel_keeps_it() {
    let app = started();
    typed(&app, "keep me");
    press(&app, WindowAction::New);
    assert!(asking(&app));
    assert_eq!(app.ui(|ui| ui.get_doc_dialog()), 5, "the screen's unsaved prompt");
    assert_eq!(state(&app)["prompt_open"], true);
    press(&app, WindowAction::Cancel);
    assert!(!asking(&app));
    assert_eq!(text(&app), "keep me");
}

#[test]
fn discarding_on_the_prompt_starts_the_new_document_and_the_draft_goes() {
    let app = started();
    typed(&app, "throw away");
    app.run_writes();
    let draft = draft_file(&app);
    assert!(draft.exists());
    press(&app, WindowAction::New);
    press(&app, WindowAction::Discard);
    assert_eq!(text(&app), "");
    assert!(!asking(&app));
    app.run_writes();
    assert!(!draft.exists(), "what was given up on purpose is not offered again");
}

#[test]
fn open_over_unsaved_text_asks_first_and_the_path_rides_on_the_prompt() {
    let (app, path) = with_file("open-prompt", "# One\n");
    typed(&app, "# One\n\nedited\n");
    let other = scratch("open-prompt-2").join("two.md");
    std::fs::write(&other, "# Two\n").unwrap();
    press(&app, WindowAction::Open);
    press(&app, WindowAction::Confirm { path: other.display().to_string() });
    assert!(asking(&app), "{}", notice(&app));
    assert_eq!(path_of(&app), Some(path.clone()), "nothing opened yet");
    // Saving first writes the edit, then opens the file asked for, without retyping it.
    press(&app, WindowAction::Confirm { path: String::new() });
    assert_eq!(file(&path), "# One\n\nedited\n");
    assert_eq!(path_of(&app), Some(other));
    assert_eq!(text(&app), "# Two\n");
}

#[test]
fn discard_then_open_that_cannot_be_read_leaves_the_text_on_screen() {
    let (app, path) = with_file("open-discard", "# One\n");
    typed(&app, "# One\n\nunsaved\n");
    let missing = path.with_file_name("gone.md");
    press(&app, WindowAction::Open);
    press(&app, WindowAction::Confirm { path: missing.display().to_string() });
    press(&app, WindowAction::Discard);
    assert!(notice(&app).starts_with("Cannot open "), "{}", notice(&app));
    assert_eq!(text(&app), "# One\n\nunsaved\n", "nothing is replaced until the file has been read");
    assert!(super::modified(&app), "and it is still unsaved, with its draft");
}

#[test]
fn closing_the_window_asks_and_discard_closes_it_with_no_draft_left() {
    let app = started();
    typed(&app, "unsaved");
    app.run_writes();
    let draft = draft_file(&app);
    app.close_requested(WindowId::PRIMARY);
    assert!(asking(&app) && app.exit_reason().is_none());
    // A second close while one asks needs no words.
    app.close_requested(WindowId::PRIMARY);
    assert!(asking(&app));
    press(&app, WindowAction::Discard);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
    app.run_writes();
    assert!(!draft.exists());
}

#[test]
fn cancel_in_the_windows_close_keeps_everything() {
    let (app, _) = with_file("window-cancel", "v1\n");
    typed(&app, "v2\n");
    app.close_requested(WindowId::PRIMARY);
    press(&app, WindowAction::Cancel);
    assert!(app.exit_reason().is_none());
    assert_eq!((state(&app)["prompt_open"].clone(), text(&app)), (json!(false), "v2\n".to_string()));
}

#[test]
fn a_window_with_nothing_unsaved_closes_at_once() {
    let app = started();
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn a_document_saved_from_the_prompt_asks_where_and_closes_once_written() {
    let app = started();
    typed(&app, "untitled text");
    app.close_requested(WindowId::PRIMARY);
    press(&app, WindowAction::Confirm { path: String::new() });
    assert_eq!(app.ui(|ui| ui.get_doc_dialog()), 2, "Save As, from the prompt");
    let path = scratch("prompt-save-as").join("kept.md");
    press(&app, WindowAction::Confirm { path: path.display().to_string() });
    assert_eq!(file(&path), "untitled text");
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn cancelling_the_save_as_the_prompt_opened_calls_the_whole_thing_off() {
    let app = started();
    typed(&app, "untitled text");
    app.close_requested(WindowId::PRIMARY);
    press(&app, WindowAction::Confirm { path: String::new() });
    press(&app, WindowAction::Cancel);
    assert!(!asking(&app) && app.exit_reason().is_none());
    assert_eq!(text(&app), "untitled text");
}

#[test]
fn a_save_that_fails_in_the_prompt_keeps_the_prompt_up_with_the_reason() {
    let (app, path) = with_file("save-close-fails", "v1\n");
    typed(&app, "v2\n");
    let disk = failing(&app);
    disk.fail(Step::Write, "notes.md", Some(1));
    app.close_requested(WindowId::PRIMARY);
    press(&app, WindowAction::Confirm { path: String::new() });
    assert!(asking(&app), "the window is not closed");
    assert!(app.model(|m| m.dialog_error.starts_with("Save failed")), "the prompt says why");
    assert_eq!(file(&path), "v1\n");
    assert!(app.exit_reason().is_none());
    press(&app, WindowAction::Confirm { path: String::new() });
    assert_eq!(file(&path), "v2\n", "Save again, with the disk back, closes it");
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
}

#[test]
fn a_logout_asks_nothing_and_keeps_the_unsaved_text() {
    for signal in [true, false] {
        let (app, path) = with_file("logout", "v1\n");
        typed(&app, "unsaved at logout\n");
        if signal {
            app.event(Event::ExitRequested(ExitReason::Logout));
        } else {
            app.display_gone();
        }
        assert!(app.exit_reason().is_some(), "ended in the command that answered");
        assert_eq!(state(&app)["prompt_open"], false, "no one was asked");
        app.shutdown();
        let again = killed_and_started_again(app);
        assert_eq!(text(&again), "unsaved at logout\n", "signal={signal}");
        assert_eq!(file(&path), "v1\n");
    }
}

#[test]
fn a_person_quitting_is_the_windows_close() {
    let app = started();
    typed(&app, "x");
    app.event(Event::ExitRequested(ExitReason::Person));
    assert!(asking(&app));
    assert!(app.exit_reason().is_none());
}

/// One question at a time: the window offers only the prompt's answers while it asks, and no
/// dialog shows without a live question behind it.
#[test]
fn one_prompt_at_a_time_and_none_without_its_question() {
    let app = started();
    typed(&app, "a");
    press(&app, WindowAction::New);
    assert!(asking(&app));
    // A second New, an Open, a Save As: none takes over the question or opens another.
    press(&app, WindowAction::New);
    press(&app, WindowAction::Open);
    press(&app, WindowAction::SaveAs);
    assert_eq!(app.model(|m| m.dialog), crate::model::Dialog::None);
    assert_eq!(app.ui(|ui| ui.get_doc_dialog()), 5);
    // A close meanwhile does not stack a second prompt on the first.
    app.close_requested(WindowId::PRIMARY);
    assert!(app.model(|m| matches!(m.closing.as_ref().map(|c| &c.intent), Some(crate::prompt::Intent::New))));
    // Saved another way, the question is no longer true, so it goes.
    let path = scratch("prompt-goes").join("a.md");
    let refused = act(&app, "save_as", json!({ "path": path.display().to_string() })).unwrap_err();
    assert!(refused.contains("asking the person"), "a call waits for the answer: {refused}");
    press(&app, WindowAction::Cancel);
    act(&app, "save_as", json!({ "path": path.display().to_string() })).unwrap();
    assert!(!asking(&app));
    assert_eq!(app.ui(|ui| ui.get_doc_dialog()), 0, "no dialog without a live prompt behind it");
}

#[test]
fn opening_a_file_while_clean_just_opens_it() {
    let app = started();
    let path = scratch("clean-open").join("c.md");
    opened(&app, &path, "# C\n");
    assert!(!asking(&app));
}
