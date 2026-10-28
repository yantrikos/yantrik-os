//! The dialogs: the desktop's picker (stood in for), the typed path when it cannot be asked, and
//! the unsaved-changes question in front of Open. A dialog is shown only while something is
//! asking, and there is one question at a time.

use yantrik_app::app::WindowId;

use super::{
    enter, file, header, modified, notice, path_of, picker_says, raw, scratch, settle, shown, started, status, with_file,
};
use crate::files::picker::Choice;
use crate::SheetDialog;

fn dialog(app: &yantrik_testing::TestApp<crate::YSheets>) -> SheetDialog {
    app.ui(|ui| ui.get_dialog())
}

fn confirm(app: &yantrik_testing::TestApp<crate::YSheets>, path: &str) {
    let path = path.to_string();
    app.ui(move |ui| {
        ui.set_dialog_path(path.into());
        ui.invoke_dialog_confirm();
    });
    settle(app);
}

fn no_portal(app: &yantrik_testing::TestApp<crate::YSheets>) {
    picker_says(app, |_, _| Choice::Unavailable("no session bus in this test".into()));
}

#[test]
fn save_with_no_file_asks_for_a_typed_path_when_the_picker_cannot_be_asked() {
    let app = started();
    no_portal(&app);
    for text in ["2", "3", "", "=SUM(A1:A3)"] {
        enter(&app, text);
    }
    let dir = scratch("typed-save");
    let path = dir.join("driven.csv");
    header(&app, "save");
    assert_eq!(dialog(&app), SheetDialog::SavePath);
    assert!(app.ui(|ui| ui.get_dialog_message().to_string()).contains("file picker could not be asked"));
    assert!(app.ui(|ui| ui.get_dialog_path().to_string()).ends_with("sheet.csv"), "a name is suggested");
    confirm(&app, &path.display().to_string());
    assert_eq!(dialog(&app), SheetDialog::None);
    assert!(!modified(&app) && !app.ui(|ui| ui.get_modified()), "saved");
    assert_eq!(app.ui(|ui| ui.get_file_name().to_string()), "driven.csv");
    assert_eq!(file(&path), "2\r\n3\r\n\r\n=SUM(A1:A3)\r\n");
}

#[test]
fn a_typed_path_onto_a_file_that_is_there_asks_once_and_the_second_press_replaces_it() {
    let dir = scratch("typed-replace");
    let path = dir.join("taken.csv");
    std::fs::write(&path, "old\r\n").unwrap();
    let app = started();
    no_portal(&app);
    enter(&app, "new");
    header(&app, "save-as");
    confirm(&app, &path.display().to_string());
    assert_eq!(dialog(&app), SheetDialog::SavePath, "still asking");
    assert_eq!(
        app.ui(|ui| ui.get_dialog_message().to_string()),
        format!("{} is already there. Press Save again to replace it.", path.display())
    );
    assert_eq!(file(&path), "old\r\n");
    // Another path starts the asking over.
    let other = dir.join("other.csv");
    std::fs::write(&other, "other\r\n").unwrap();
    confirm(&app, &other.display().to_string());
    assert_eq!(file(&other), "other\r\n", "the earlier yes was for the other path");
    confirm(&app, &other.display().to_string());
    assert_eq!(file(&other), "new\r\n");
    assert_eq!(dialog(&app), SheetDialog::None);
}

#[test]
fn a_typed_path_that_cannot_be_written_stays_in_the_dialog_with_the_reason() {
    let app = started();
    no_portal(&app);
    enter(&app, "x");
    header(&app, "save-as");
    confirm(&app, "/nonexistent-folder-ysheets/x.csv");
    assert_eq!(dialog(&app), SheetDialog::SavePath);
    assert!(app.ui(|ui| ui.get_dialog_message().to_string()).contains("does not exist"));
    confirm(&app, &scratch("typed-fix").join("x.csv").display().to_string());
    assert_eq!(dialog(&app), SheetDialog::None);
    assert!(!modified(&app));
}

#[test]
fn a_save_through_the_picker_goes_straight_to_the_file_it_chose() {
    let app = started();
    enter(&app, "picked");
    let target = scratch("picked").join("picked.csv");
    let chosen = target.clone();
    picker_says(&app, move |saving, _| {
        assert!(saving);
        Choice::Chosen(chosen.clone())
    });
    header(&app, "save-as");
    assert_eq!(dialog(&app), SheetDialog::None);
    assert_eq!(file(&target), "picked\r\n");
    assert_eq!(app.ui(|ui| ui.get_file_name().to_string()), "picked.csv");
    // The picker has asked about replacing a file that is there, so the save does not.
    enter(&app, "again");
    header(&app, "save-as");
    assert_eq!(file(&target), "picked\r\nagain\r\n");
}

#[test]
fn a_picker_that_is_cancelled_changes_nothing() {
    let app = started();
    enter(&app, "kept");
    picker_says(&app, |_, _| Choice::Cancelled);
    header(&app, "save-as");
    assert_eq!(status(&app), "Nothing was chosen, so nothing changed.");
    assert!(modified(&app) && path_of(&app).is_none());
}

#[test]
fn the_picker_is_asked_with_the_sheets_file_to_suggest() {
    let (app, path) = with_file("suggest", "1\r\n");
    let (tx, rx) = std::sync::mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    picker_says(&app, move |saving, current| {
        tx.lock().unwrap().send((saving, current)).unwrap();
        Choice::Cancelled
    });
    header(&app, "save-as");
    assert_eq!(rx.recv().unwrap(), (true, Some(path)));
}

#[test]
fn a_second_press_while_the_picker_is_open_does_not_ask_again() {
    let app = started();
    // Nothing runs the picker's job until the test says, so the first press is still asking.
    picker_says(&app, |_, _| Choice::Cancelled);
    app.ui(|ui| ui.invoke_command("save-as".into()));
    app.ui(|ui| ui.invoke_command("save-as".into()));
    assert_eq!(app.jobs_waiting(), 1);
    settle(&app);
}

#[test]
fn open_over_unsaved_changes_asks_first_and_discard_goes_on_to_the_file() {
    let (app, path) = with_file("open-dirty", "1\r\n=A1+1\r\n");
    enter(&app, "scratch");
    assert!(modified(&app));
    no_portal(&app);
    header(&app, "open");
    assert_eq!(dialog(&app), SheetDialog::Unsaved);
    assert_eq!(
        app.ui(|ui| ui.get_dialog_message().to_string()),
        "budget.csv has changes that are not saved. Save them before opening another file?"
    );
    app.ui(|ui| ui.invoke_dialog_discard());
    settle(&app);
    assert_eq!(dialog(&app), SheetDialog::OpenPath, "no picker, so a path is typed");
    confirm(&app, &path.display().to_string());
    assert_eq!(shown(&app, "A2"), "2", "reopened, the formula is worked out again");
    assert_eq!(raw(&app, "A1"), "1", "and the discarded edit is gone");
    assert!(!modified(&app));
    assert_eq!(app.ui(|ui| slint::Model::row_count(&ui.get_rows())), 50, "a two-row file still opens a full grid");
}

#[test]
fn cancel_on_the_question_keeps_everything_and_nothing_is_asked_again() {
    let (app, _) = with_file("open-cancel", "1\r\n");
    enter(&app, "mine");
    header(&app, "open");
    assert_eq!(dialog(&app), SheetDialog::Unsaved);
    app.ui(|ui| ui.invoke_dialog_cancel());
    settle(&app);
    assert_eq!(dialog(&app), SheetDialog::None);
    assert_eq!(status(&app), "Nothing changed.");
    assert_eq!(raw(&app, "A1"), "mine");
    assert!(modified(&app));
}

#[test]
fn save_on_the_question_saves_then_goes_on_to_open() {
    let (app, path) = with_file("open-save", "1\r\n");
    enter(&app, "saved first");
    let other = path.with_file_name("other.csv");
    std::fs::write(&other, "from other\r\n").unwrap();
    let chosen = other.clone();
    picker_says(&app, move |saving, _| {
        assert!(!saving, "the file to open is asked for after the save");
        Choice::Chosen(chosen.clone())
    });
    header(&app, "open");
    assert_eq!(dialog(&app), SheetDialog::Unsaved);
    app.ui(|ui| ui.invoke_dialog_confirm());
    settle(&app);
    assert_eq!(file(&path), "saved first\r\n", "the sheet went to its own file");
    assert_eq!(path_of(&app), Some(other));
    assert_eq!(raw(&app, "A1"), "from other");
    assert_eq!(dialog(&app), SheetDialog::None);
}

#[test]
fn a_save_the_question_asked_for_that_fails_ends_the_question_and_says_why() {
    let (app, path) = with_file("question-fails", "1\r\n");
    let disk = super::failing(&app);
    disk.fail(yantrik_app::documents::disk::Step::Write, "budget.csv", Some(1));
    enter(&app, "unsaved");
    header(&app, "open");
    app.ui(|ui| ui.invoke_dialog_confirm());
    settle(&app);
    assert_eq!(dialog(&app), SheetDialog::None, "no question is left up with nothing behind it");
    assert!(notice(&app).starts_with("Save failed:"), "{}", notice(&app));
    assert_eq!(file(&path), "1\r\n");
    assert!(modified(&app));
}

#[test]
fn closing_over_unsaved_changes_asks_and_cancel_keeps_the_window() {
    let (app, _) = with_file("close-cancel", "1\r\n");
    enter(&app, "mine");
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(dialog(&app), SheetDialog::Unsaved);
    assert_eq!(
        app.ui(|ui| ui.get_dialog_message().to_string()),
        "budget.csv has changes that are not saved. Save them before closing?"
    );
    assert!(app.exit_reason().is_none());
    // Closing again with the question up asks nothing more: there is one question at a time.
    app.close_requested(WindowId::PRIMARY);
    assert_eq!(dialog(&app), SheetDialog::Unsaved);
    app.ui(|ui| ui.invoke_dialog_cancel());
    settle(&app);
    assert_eq!(dialog(&app), SheetDialog::None);
    assert_eq!(app.windows(), vec![WindowId::PRIMARY], "Cancel keeps the window open");
    assert!(app.exit_reason().is_none());
}

#[test]
fn closing_with_discard_ends_and_the_next_start_offers_nothing() {
    let (app, path) = with_file("close-discard", "1\r\n");
    enter(&app, "thrown away");
    super::flush_drafts(&app);
    assert!(super::draft_file(&app).exists(), "the draft was kept behind the question");
    app.close_requested(WindowId::PRIMARY);
    app.ui(|ui| ui.invoke_dialog_discard());
    settle(&app);
    assert!(app.exit_reason().is_some());
    app.run_writes();
    assert!(!super::draft_file(&app).exists(), "given up on purpose, so the draft goes");
    let again = super::killed_and_started_again(app);
    assert_eq!(notice(&again), "", "nothing is offered back");
    assert_eq!(path_of(&again), None);
    assert_eq!(file(&path), "1\r\n");
}

#[test]
fn closing_with_save_writes_the_file_then_closes() {
    let (app, path) = with_file("close-save", "1\r\n");
    enter(&app, "saved at close");
    app.close_requested(WindowId::PRIMARY);
    app.ui(|ui| ui.invoke_dialog_confirm());
    settle(&app);
    assert_eq!(file(&path), "saved at close\r\n");
    assert!(app.exit_reason().is_some());
}

#[test]
fn closing_with_save_for_a_sheet_with_no_file_asks_where_then_closes() {
    let app = started();
    enter(&app, "no home yet");
    let target = scratch("close-save-as").join("home.csv");
    let chosen = target.clone();
    picker_says(&app, move |_, _| Choice::Chosen(chosen.clone()));
    app.close_requested(WindowId::PRIMARY);
    app.ui(|ui| ui.invoke_dialog_confirm());
    settle(&app);
    assert_eq!(file(&target), "no home yet\r\n");
    assert!(app.exit_reason().is_some());
}

#[test]
fn a_person_quitting_is_asked_the_same_question_once() {
    use yantrik_app::app::{Event, ExitReason};
    let (app, _) = with_file("quit", "1\r\n");
    enter(&app, "unsaved");
    app.event(Event::ExitRequested(ExitReason::Person));
    settle(&app);
    assert_eq!(dialog(&app), SheetDialog::Unsaved);
    assert!(app.exit_reason().is_none(), "the person can still say no");
    app.event(Event::ExitRequested(ExitReason::Person));
    settle(&app);
    assert_eq!(dialog(&app), SheetDialog::Unsaved);
}

#[test]
fn text_in_the_formula_bar_at_close_is_committed_and_asked_about() {
    let app = started();
    super::type_in_bar(&app, "typed, never entered");
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    assert_eq!(raw(&app, "A1"), "typed, never entered", "the bar's text was committed first");
    assert_eq!(dialog(&app), SheetDialog::Unsaved, "so there was something to ask about");
    assert!(app.exit_reason().is_none());
}

#[test]
fn a_sheet_with_nothing_unsaved_closes_without_a_question() {
    let (app, _) = with_file("close-clean", "1\r\n");
    app.close_requested(WindowId::PRIMARY);
    settle(&app);
    assert_eq!(dialog(&app), SheetDialog::None);
    assert!(app.exit_reason().is_some());
}

/// The question is on the window exactly when the model is asking it, and a path dialog exactly
/// when the model has one: whatever the person does, in whatever order.
fn agrees(app: &yantrik_testing::TestApp<crate::YSheets>) {
    let (asking, dialog_now) = app.model(|m| (m.asking(), m.dialog));
    let want = match (dialog_now, asking) {
        (crate::model::Dialog::OpenPath, _) => SheetDialog::OpenPath,
        (crate::model::Dialog::SavePath, _) => SheetDialog::SavePath,
        (crate::model::Dialog::None, true) => SheetDialog::Unsaved,
        (crate::model::Dialog::None, false) => SheetDialog::None,
    };
    assert_eq!(dialog(app), want);
}

#[test]
fn a_dialog_is_never_up_without_a_live_question_behind_it() {
    let (app, _) = with_file("invariant", "1\r\n");
    agrees(&app);
    enter(&app, "unsaved");
    no_portal(&app);
    header(&app, "open");
    agrees(&app);
    app.ui(|ui| ui.invoke_dialog_cancel());
    settle(&app);
    agrees(&app);
    header(&app, "open");
    app.ui(|ui| ui.invoke_dialog_discard());
    settle(&app);
    agrees(&app);
    app.ui(|ui| ui.invoke_dialog_cancel());
    settle(&app);
    agrees(&app);
    assert!(app.model(|m| !m.asking() && m.dialog == crate::model::Dialog::None && !m.discarding));
    app.close_requested(WindowId::PRIMARY);
    agrees(&app);
    app.ui(|ui| ui.invoke_dialog_cancel());
    settle(&app);
    agrees(&app);
}
