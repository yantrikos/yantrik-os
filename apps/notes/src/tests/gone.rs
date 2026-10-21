//! An open note whose file is deleted or moved outside Notes (the 520 run of #918, defect 2). Its
//! text is then on no disk at all, so it is unsaved by the one rule (`Model::unsaved_doc`) even
//! before anything is typed: the footer says the file is gone, in amber; `describe` says unsaved;
//! its draft is kept, so a crash offers it back; and a close asks. The file coming back before
//! anything is typed leaves the note as clean as it was.

use std::path::PathBuf;

use serde_json::json;
use yantrik_app::app::{ExitReason, WindowId};
use yantrik_app::documents::OutsideState;
use yantrik_testing::TestApp;

use super::windows::two_notes_two_windows;
use super::{bar, draft_file, file, killed_and_started_again, outside, settle, state, unsaved, with_note};
use crate::command::Command;
use crate::update::closing::CLOSE_DIALOG;
use crate::update::said;
use crate::Notes;

/// A saved note, open and untouched, whose file another program then deleted, as the watch saw it.
pub(super) fn deleted_untouched() -> (TestApp<Notes>, PathBuf) {
    let (app, path) = with_note("Gone");
    std::fs::remove_file(&path).unwrap();
    app.look_at_files();
    assert_eq!(outside(&app), OutsideState::Deleted);
    (app, path)
}

fn footer(app: &TestApp<Notes>, window: WindowId) -> (String, bool) {
    app.window(window, |ui| (ui.get_status().to_string(), ui.get_modified())).expect("the window is open")
}

#[test]
fn a_note_whose_file_is_gone_is_unsaved_before_any_edit() {
    let (app, path) = deleted_untouched();
    assert_eq!(footer(&app, WindowId::PRIMARY), (said::FILE_GONE.to_string(), true), "said, in amber");
    assert!(unsaved(&app));
    let described = state(&app);
    assert_eq!((described["unsaved"].clone(), described["windows"][0]["unsaved"].clone()), (json!(true), json!(true)));
    let summary = app.describe()["summary"].as_str().unwrap().to_string();
    assert!(summary.contains("unsaved, its file was moved or deleted"), "{summary}");
    assert_eq!(bar(&app), 2, "Save as new is offered");
    app.run_writes();
    assert!(draft_file(&app, &path).exists(), "its text is kept for a crash: it is on no disk");
    assert!(!path.exists(), "and the file is not made again");
    // The footer stays so through the autosave, which may not write the file again.
    super::autosave(&app);
    assert_eq!(footer(&app, WindowId::PRIMARY).0, said::FILE_GONE);
    assert!(!path.exists());
}

#[test]
fn closing_a_note_whose_file_is_gone_asks_and_dont_save_lets_it_go() {
    let (app, path) = deleted_untouched();
    app.close_requested(WindowId::PRIMARY);
    let heading = app.ui(|ui| (ui.get_dialog(), ui.get_close_heading().to_string()));
    assert_eq!(heading, (CLOSE_DIALOG, "Save \"Gone\" as a new note before closing?".to_string()));
    assert_eq!(app.ui(|ui| ui.get_close_save_label().to_string()), said::SAVE_AS_NEW);
    app.ui(|ui| ui.invoke_action("close-discard".into()));
    assert_eq!(app.exit_reason(), Some(ExitReason::LastWindowClosed));
    app.run_writes();
    assert!(!draft_file(&app, &path).exists(), "the text was let go, with its draft");
}

/// With two windows, *Don't save* closes one; the gone note leaves the library with its text,
/// and nothing of it is offered at the next start.
#[test]
fn dont_save_on_a_gone_note_takes_it_out_of_the_library() {
    let (app, first, left, second, _) = two_notes_two_windows();
    std::fs::remove_file(&left).unwrap();
    app.look_at_files();
    assert_eq!(footer(&app, first).0, said::FILE_GONE);
    assert_eq!(footer(&app, second).0, "All changes saved", "the other window's note is on disk");
    app.close_requested(first);
    assert_eq!(app.window(first, |ui| ui.get_dialog()), Some(CLOSE_DIALOG));
    app.window(first, |ui| ui.invoke_action("close-discard".into()));
    settle(&app);
    assert_eq!(app.windows(), vec![second]);
    let titles: Vec<String> = app.model(|m| m.library.iter().map(|n| n.title()).collect());
    assert_eq!(titles, vec!["Right".to_string()], "nothing of it is listed");
    app.run_writes();
    let again = killed_and_started_again(app);
    assert!(state(&again)["recovery"].as_array().unwrap().is_empty());
}

#[test]
fn a_crash_after_the_file_went_offers_the_text_back_as_a_note_whose_file_is_gone() {
    let (app, path) = deleted_untouched();
    app.run_writes();
    let again = killed_and_started_again(app);
    let offered = state(&again)["recovery"].clone();
    assert_eq!(offered[0]["title"], "Gone", "the only copy of the text is offered back");
    super::act(&again, "restore_draft", json!({})).expect("restored");
    assert_eq!(super::notice(&again), said::RESTORED_GONE);
    assert!(!path.exists(), "and never made again unasked");
}

/// The file back with other text before anything was typed: it is read again, as any outside
/// change to a note with nothing unsaved is, not held as a conflict over edits no one made.
#[test]
fn a_file_back_before_any_edit_is_read_again_not_a_conflict() {
    let (app, path) = deleted_untouched();
    std::fs::write(&path, "# Gone\n\nback, from elsewhere\n").unwrap();
    app.look_at_files();
    settle(&app);
    assert_eq!(outside(&app), OutsideState::Clean);
    assert_eq!(app.model(|m| m.current().unwrap().text().to_string()), "# Gone\n\nback, from elsewhere\n");
    assert!(!unsaved(&app) && bar(&app) == 0);
    assert_eq!(footer(&app, WindowId::PRIMARY), ("All changes saved".to_string(), false));
    app.run_writes();
    assert!(!draft_file(&app, &path).exists(), "the draft kept while it was gone is gone too");
}

/// And the same file back: the note is as it was, saved.
#[test]
fn the_same_file_back_before_any_edit_leaves_the_note_saved() {
    let (app, path) = with_note("Gone");
    let saved = file(&path);
    std::fs::remove_file(&path).unwrap();
    app.look_at_files();
    assert!(unsaved(&app));
    std::fs::write(&path, &saved).unwrap();
    app.look_at_files();
    settle(&app);
    assert_eq!(outside(&app), OutsideState::Clean);
    assert!(!unsaved(&app));
    assert!(!app.model(|m| m.current().unwrap().doc.dirty()), "clean to the framework as well");
}

/// Typed in after the file went: the file coming back is the conflict it now is.
#[test]
fn a_file_back_after_an_edit_is_a_conflict() {
    let (app, path) = deleted_untouched();
    app.run(Command::Edit("# Gone\n\ntyped while it was gone".into()));
    std::fs::write(&path, "# Gone\n\nback\n").unwrap();
    app.look_at_files();
    assert!(matches!(outside(&app), OutsideState::Conflict { .. }));
    assert_eq!(bar(&app), 1);
}
