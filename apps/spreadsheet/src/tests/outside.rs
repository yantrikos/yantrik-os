//! The sheet's file changed outside ySheets: followed when nothing is unsaved, a conflict that is
//! never written over when something is, a moved file followed and a deleted one said.

use serde_json::json;

use super::{act, enter, file, modified, notice, outside_writes, path_of, raw, settle, shown, with_file};
use crate::command::Command;
use crate::files::open::open_sheet;

#[test]
fn a_clean_sheet_follows_another_programs_edit() {
    let (app, path) = with_file("follow", "1\r\n=A1+1\r\n");
    assert_eq!(shown(&app, "A2"), "2");
    outside_writes(&app, &path, "10\r\n=A1+1\r\nnew row\r\n");
    assert_eq!(raw(&app, "A1"), "10");
    assert_eq!(shown(&app, "A2"), "11", "and worked out again");
    assert_eq!(shown(&app, "A3"), "new row");
    assert_eq!(notice(&app), "budget.csv was changed outside ySheets; the sheet now shows the file as it is.");
    assert!(!modified(&app));
    assert!(!app.model(|m| m.book().history.can_undo()), "what was typed before is not undoable into theirs");
}

#[test]
fn an_edit_under_unsaved_changes_is_a_conflict_that_is_never_written_over() {
    let (app, path) = with_file("conflict", "1\r\n");
    act(&app, "set_cell", json!({ "cell": "A1", "text": "mine" })).unwrap();
    outside_writes(&app, &path, "theirs\r\n");
    assert!(notice(&app).contains("was changed outside ySheets while this sheet has unsaved changes"), "{}", notice(&app));
    assert_eq!(raw(&app, "A1"), "mine", "the sheet keeps what was typed");
    assert!(modified(&app));
    let err = act(&app, "save_over", json!({})).unwrap_err();
    assert!(err.contains("changed on disk since it was opened"), "{err}");
    assert_eq!(file(&path), "theirs\r\n");
}

#[test]
fn a_deleted_file_is_said_and_the_sheet_stays_unsaved_until_it_is_saved_somewhere() {
    let (app, path) = with_file("deleted", "1\r\n");
    std::fs::remove_file(&path).unwrap();
    app.look_at_files();
    settle(&app);
    assert!(notice(&app).contains("is not there any more"), "{}", notice(&app));
    assert!(modified(&app), "its content is on no disk at all");
    // Saving it somewhere is what clears that.
    let elsewhere = path.with_file_name("again.csv");
    act(&app, "save", json!({ "path": elsewhere.display().to_string() })).unwrap();
    assert!(!modified(&app));
    assert_eq!(path_of(&app), Some(elsewhere));
}

#[test]
fn a_moved_file_is_followed() {
    let (app, path) = with_file("moved", "1\r\nsecond\r\n");
    let moved = path.with_file_name("renamed.csv");
    std::fs::rename(&path, &moved).unwrap();
    app.look_at_files();
    settle(&app);
    assert_eq!(path_of(&app), Some(moved.clone()));
    assert_eq!(notice(&app), format!("This sheet moved. It is {} now.", moved.display()));
    assert!(!modified(&app));
    enter(&app, "edited");
    act(&app, "save_over", json!({})).unwrap();
    assert_eq!(file(&moved), "edited\r\nsecond\r\n");
}

#[test]
fn a_file_that_is_not_csv_any_more_is_said_and_the_sheet_keeps_its_content() {
    let (app, path) = with_file("not-csv", "1\r\n");
    outside_writes(&app, &path, "a,\"never closed\n");
    assert!(notice(&app).contains("cannot be shown here"), "{}", notice(&app));
    assert_eq!(raw(&app, "A1"), "1");
    assert!(modified(&app), "a conflict is unsaved: Save as keeps the sheet");
}

#[test]
fn a_late_re_read_of_a_file_no_longer_open_is_dropped() {
    let (app, path) = with_file("late", "1\r\n");
    let other = super::scratch("late-2").join("other.csv");
    std::fs::write(&other, "other\r\n").unwrap();
    let theirs = open_sheet(&other, false).unwrap();
    app.run(Command::Reread { from: other.clone(), result: Ok(Box::new(theirs)) });
    assert_eq!(raw(&app, "A1"), "1", "it was not this sheet's file");
    assert_eq!(path_of(&app), Some(path));
    assert_eq!(notice(&app), "");
}

#[test]
fn our_own_save_is_not_another_programs_edit() {
    let (app, _) = with_file("own", "1\r\n");
    act(&app, "set_cell", json!({ "cell": "A1", "text": "2" })).unwrap();
    act(&app, "save_over", json!({})).unwrap();
    app.look_at_files();
    settle(&app);
    assert_eq!(notice(&app), "");
    assert!(!modified(&app));
}
