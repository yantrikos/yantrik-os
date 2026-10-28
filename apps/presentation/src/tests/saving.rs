//! Saving: how much of a failure reached the disk said honestly, another program's edit never
//! written over, and what the window's Save does when it cannot write the deck's own file (it has
//! no Save As, so the deck is kept beside the file rather than lost or written over).

use serde_json::json;
use yantrik_app::documents::disk::Step;
use yantrik_app::documents::OutsideState;

use super::{
    act, draft_file, failing, listing, modified, none, notice, outside_writes, path_of, press, read_deck, scratch,
    settle, type_title, with_file, write_deck,
};
use crate::command::{Command, WindowAction};

#[test]
fn a_save_that_fails_says_so_leaves_the_file_alone_and_keeps_the_deck_unsaved() {
    let (app, path) = with_file("fails", &["One"]);
    let disk = failing(&app);
    disk.fail(Step::Write, "talk.ydeck", Some(1));
    act(&app, "set_slide", json!({ "index": 0, "title": "Two" })).unwrap();
    let err = act(&app, "save", none()).unwrap_err();
    assert!(err.starts_with("Save failed") && err.contains("Your draft is still open"), "{err}");
    assert_eq!(notice(&app), err);
    assert_eq!(read_deck(&path).slides[0].title, "One", "the file is as it was");
    assert!(modified(&app), "nothing is called saved that is not");
    app.run_writes();
    assert!(draft_file(&app).exists(), "and the work is in its draft");
    // The disk back: the same Save goes through.
    disk.heal();
    act(&app, "save", none()).unwrap();
    assert_eq!(read_deck(&path).slides[0].title, "Two");
    assert!(!modified(&app));
}

#[test]
fn edits_made_while_a_save_is_in_flight_stay_unsaved() {
    let (app, path) = with_file("meanwhile", &["One"]);
    act(&app, "set_slide", json!({ "index": 0, "title": "Two" })).unwrap();
    let pending = app.act("save", none()).unwrap();
    type_title(&app, "Three, typed meanwhile");
    settle(&app);
    let answer = pending.settle().unwrap();
    assert_eq!(read_deck(&path).slides[0].title, "Two", "the file holds what was written, not what was typed since");
    assert_eq!(answer["dirty"], true, "and the answer says the deck is not all in it");
    assert!(modified(&app));
    app.run_writes();
    assert!(draft_file(&app).exists());
}

#[test]
fn the_answer_to_a_save_is_about_that_save() {
    let (app, path) = with_file("answer", &["One"]);
    act(&app, "add_slide", json!({ "title": "Two" })).unwrap();
    let pending = app.act("save", none()).unwrap();
    // The person adds another slide before the save lands.
    app.ui(|ui| ui.invoke_add_slide());
    settle(&app);
    let answer = pending.settle().unwrap();
    assert_eq!(answer["slides"], 2, "the deck as it was written, not as it is now");
    assert_eq!(read_deck(&path).slides.len(), 2);
    assert_eq!(super::outline(&app).len(), 3);
}

#[test]
fn another_programs_edit_is_never_written_over_and_the_deck_stays_unsaved() {
    let (app, path) = with_file("conflict", &["One"]);
    act(&app, "set_slide", json!({ "index": 0, "title": "Mine" })).unwrap();
    // Written under the app's feet; the watch has not looked yet.
    std::fs::write(&path, super::deck_text("Theirs", &["Theirs"])).unwrap();
    let err = act(&app, "save", none()).unwrap_err();
    assert!(err.contains("changed on disk since it was opened") && err.contains("your draft is intact"), "{err}");
    assert_eq!(read_deck(&path).slides[0].title, "Theirs", "their version is where they left it");
    assert!(modified(&app));
    assert!(matches!(app.model(|m| m.d().outside.clone()), OutsideState::Conflict { .. }));
}

#[test]
fn the_windows_save_over_another_programs_edit_keeps_both_versions() {
    let (app, path) = with_file("conflict-window", &["One"]);
    let dir = super::deck_folder(&app);
    type_title(&app, "Mine");
    outside_writes(&app, &path, &super::deck_text("Theirs", &["Theirs"]));
    assert!(notice(&app).contains("was changed outside yPresent while this deck has unsaved changes"), "{}", notice(&app));
    press(&app, WindowAction::Save);
    assert_eq!(read_deck(&path).slides[0].title, "Theirs", "theirs is not written over");
    let now = path_of(&app).expect("the deck has a file of its own now");
    assert_eq!((now.parent(), now == path), (Some(dir.as_path()), false));
    assert_eq!(read_deck(&now).slides[0].title, "Mine");
    assert!(notice(&app).contains("was changed by another program, so the deck was saved as a new file"), "{}", notice(&app));
    assert!(!modified(&app));
}

#[test]
fn a_decks_file_that_was_deleted_is_saved_as_a_new_file_by_the_windows_save() {
    let (app, path) = with_file("deleted", &["One"]);
    let dir = super::deck_folder(&app);
    type_title(&app, "Mine");
    std::fs::remove_file(&path).unwrap();
    app.look_at_files();
    settle(&app);
    assert!(modified(&app), "its content is on no disk at all");
    press(&app, WindowAction::Save);
    let now = path_of(&app).expect("a file of its own");
    assert_eq!(now.parent(), Some(dir.as_path()));
    assert_eq!(read_deck(&now).slides[0].title, "Mine");
    assert!(notice(&app).contains("was moved or deleted, so the deck was saved as a new file"), "{}", notice(&app));
    assert!(!modified(&app));
}

#[test]
fn a_read_only_file_and_a_link_are_refused_and_nothing_is_written() {
    let (app, path) = with_file("readonly", &["One"]);
    act(&app, "set_slide", json!({ "index": 0, "title": "Two" })).unwrap();
    let mut mode = std::fs::metadata(&path).unwrap().permissions();
    mode.set_readonly(true);
    std::fs::set_permissions(&path, mode).unwrap();
    let err = act(&app, "save", none()).unwrap_err();
    assert!(err.contains("is read-only, so nothing was written"), "{err}");
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
    assert_eq!(read_deck(&path).slides[0].title, "One");
    let dir = scratch("link-target");
    write_deck(&dir.join("real.ydeck"), "Real", &["real"]);
    let alias = dir.join("alias.ydeck");
    std::os::unix::fs::symlink(dir.join("real.ydeck"), &alias).unwrap();
    let err = act(&app, "save_as", json!({ "path": alias.display().to_string(), "overwrite": true }));
    assert!(err.is_err(), "a save never goes through a link");
    assert_eq!(read_deck(&dir.join("real.ydeck")).slides[0].title, "real");
}

#[test]
fn a_save_onto_a_file_that_was_moved_says_where_it_went() {
    let (app, path) = with_file("moved-save", &["One"]);
    let there = path.with_file_name("renamed.ydeck");
    // Moved, and the watch has not looked yet.
    std::fs::rename(&path, &there).unwrap();
    act(&app, "set_slide", json!({ "index": 0, "title": "Two" })).unwrap();
    let err = act(&app, "save", none()).unwrap_err();
    assert!(err.contains("is not there any more") && err.contains(&there.display().to_string()), "{err}");
    assert!(modified(&app));
    assert_eq!(read_deck(&there).slides[0].title, "One");
}

#[test]
fn a_failed_save_leaves_no_temporary_file_beside_the_deck() {
    let (app, path) = with_file("no-temp", &["One"]);
    let disk = failing(&app);
    disk.fail(Step::Write, "talk.ydeck", Some(2));
    act(&app, "set_slide", json!({ "index": 0, "title": "Two" })).unwrap();
    act(&app, "save", none()).unwrap_err();
    assert_eq!(listing(path.parent().unwrap()), vec!["talk.ydeck"]);
    app.run(Command::Window { action: WindowAction::Save, canvas: super::canvas(&app) });
    settle(&app);
    assert_eq!(listing(path.parent().unwrap()), vec!["talk.ydeck"]);
}

#[test]
fn a_change_typed_and_typed_back_leaves_nothing_unsaved_and_no_draft() {
    let (app, path) = with_file("typed-back", &["One"]);
    type_title(&app, "Two");
    type_title(&app, "One");
    assert!(!modified(&app), "typed and typed back: the deck is the file again");
    assert_eq!(read_deck(&path).slides[0].title, "One");
    app.run_writes();
    assert!(!draft_file(&app).exists(), "and a deck with nothing unsaved has no draft");
}
