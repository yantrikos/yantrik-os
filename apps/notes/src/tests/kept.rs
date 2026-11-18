//! Text typed while a reload, an import or a restore runs is never closed away when the job lands:
//! the open note stays open and unsaved, with its undo history, its autosave owed, and the window
//! says what the job could not do because of it.

use std::path::{Path, PathBuf};

use serde_json::json;
use yantrik_testing::TestApp;

use super::{act, autosave, note, settle, started, state};
use crate::command::{Command, WindowAction};
use crate::update::AUTOSAVE_AFTER;
use crate::Notes;

/// A saved note, open: its file, and what the file holds.
fn draft(app: &TestApp<Notes>) -> (PathBuf, String) {
    let path = PathBuf::from(note(app, "Draft", "first")["path"].as_str().unwrap());
    let saved = std::fs::read_to_string(&path).unwrap();
    (path, saved)
}

fn open(app: &TestApp<Notes>) -> (String, String, bool) {
    app.model(|m| m.current().map(|c| (c.id().to_string(), c.text().to_string(), c.unsaved())).unwrap())
}

fn file(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// A text file outside the vault, to import.
fn outside_file() -> PathBuf {
    let path = super::store::fixture("kept-import").join("in.md");
    std::fs::write(&path, "# Imported\n\nfrom outside\n").unwrap();
    path
}

#[test]
fn text_typed_during_a_reload_is_kept_unsaved_with_its_history_and_then_autosaved() {
    let app = started();
    let (path, saved) = draft(&app);
    let id = open(&app).0;
    app.run(Command::Window(WindowAction::Reload));
    assert_eq!(app.jobs_waiting(), 1, "the library is being read");
    let typed = format!("{saved}typed meanwhile\n");
    app.run(Command::Edit(typed.clone()));
    // The debounce runs out while the store is busy, and saves nothing: the save is still owed.
    app.advance(AUTOSAVE_AFTER);
    assert_eq!(app.jobs_waiting(), 1);
    settle(&app);
    assert_eq!(open(&app), (id, typed.clone(), true), "the reload closed the typing away");
    assert_eq!(app.model(|m| m.pane.status.clone()), "Unsaved · autosave pending");
    assert_eq!(state(&app)["notice"], "", "the file had not changed, so there is nothing to say");
    assert_eq!(file(&path), saved);
    // Not shown afresh: the edit typed during the reload is in the view's history.
    app.run(Command::Window(WindowAction::Undo));
    assert_eq!(open(&app).1, saved);
    app.run(Command::Window(WindowAction::Redo));
    autosave(&app);
    assert_eq!(file(&path), typed, "the owed autosave saved it");
    assert!(!open(&app).2);
}

#[test]
fn text_typed_during_a_reload_over_a_file_changed_outside_is_kept_and_never_read_over() {
    let app = started();
    let (path, saved) = draft(&app);
    app.run(Command::Window(WindowAction::Reload));
    let typed = format!("{saved}typed meanwhile\n");
    app.run(Command::Edit(typed.clone()));
    std::fs::write(&path, "# Draft\n\nfrom another editor\n").unwrap();
    settle(&app);
    assert_eq!(open(&app).1, typed);
    assert!(open(&app).2, "still unsaved");
    assert_eq!(
        state(&app)["notice"],
        "The open note changed on disk while you typed in it. What you typed is kept, unsaved; the note was not read again."
    );
    // Saving it is the outside-change flow's: the store refuses, and neither text is lost.
    autosave(&app);
    assert_eq!(file(&path), "# Draft\n\nfrom another editor\n", "the outside edit was written over");
    assert_eq!(open(&app).1, typed);
}

#[test]
fn text_typed_during_an_import_is_kept_and_the_imported_note_waits_in_the_list() {
    let app = started();
    let (path, saved) = draft(&app);
    let id = open(&app).0;
    app.run(Command::Window(WindowAction::Import));
    app.run(Command::Window(WindowAction::Confirm { path: outside_file().display().to_string() }));
    assert_eq!(app.jobs_waiting(), 1, "the import is running");
    let typed = format!("{saved}typed meanwhile\n");
    app.run(Command::Edit(typed.clone()));
    settle(&app);
    assert_eq!(open(&app), (id, typed.clone(), true), "the import closed the typing away");
    assert_eq!(
        state(&app)["notice"],
        "\"Imported\" is in the library but was not opened: the open note has unsaved text typed meanwhile, so it stays open."
    );
    assert_eq!(state(&app)["note_count"], 2);
    assert_eq!(app.model(|m| m.pane.dialog), 0, "the import is done, so its dialog closed");
    autosave(&app);
    assert_eq!(file(&path), typed);
}

#[test]
fn an_import_call_whose_note_could_not_be_opened_says_so_and_does_not_answer_with_another_note() {
    let app = started();
    let (_, saved) = draft(&app);
    let pending = app.act("import", json!({ "path": outside_file().display().to_string() })).unwrap();
    assert_eq!(app.jobs_waiting(), 1);
    app.run(Command::Edit(format!("{saved}typed meanwhile\n")));
    settle(&app);
    let said = "\"Imported\" is in the library but was not opened: the open note has unsaved text typed meanwhile, so it stays open.";
    assert_eq!(pending.settle(), Err(said.to_string()));
    assert_eq!(state(&app)["notice"], said);
    assert_eq!(state(&app)["note_count"], 2, "the import itself happened");
}

#[test]
fn a_restore_never_lands_on_typed_text_because_a_note_in_the_trash_takes_none() {
    let app = started();
    let (_, saved) = draft(&app);
    let id = open(&app).0;
    act(&app, "trash", json!({})).unwrap();
    app.run(Command::Window(WindowAction::Open { id: id.clone(), trash: true }));
    app.run(Command::Window(WindowAction::Restore));
    assert_eq!(app.jobs_waiting(), 1, "the restore is running");
    app.run(Command::Edit(format!("{saved}typed meanwhile\n")));
    settle(&app);
    assert_eq!(open(&app), (id, saved, false), "the restored note is open, as it was trashed");
    assert_eq!(state(&app)["trashed_note_open"], false);
}
