//! The deck's file changed outside yPresent: never last-writer-wins. Another program's edit to a
//! deck with nothing unsaved is followed; to one with unsaved changes it is a conflict, said at
//! once; a file moved is followed and a file gone is said.

use serde_json::json;

use super::{
    act, deck_text, modified, none, notice, outline, outside_writes, path_of, read_deck, scratch, settle, state,
    type_title, with_file, write_deck,
};
use crate::command::Command;
use crate::files::open::described;

#[test]
fn another_programs_edit_to_a_clean_deck_is_followed() {
    let (app, path) = with_file("follow", &["One", "Two"]);
    act(&app, "go_to", json!({ "index": 1 })).unwrap();
    outside_writes(&app, &path, &deck_text("Talk", &["Theirs one", "Theirs two", "Theirs three"]));
    assert_eq!(outline(&app), vec!["Theirs one", "Theirs two", "Theirs three"]);
    assert_eq!(state(&app)["index"], 1, "the selection stays where it was");
    assert!(!modified(&app), "following the file is not an edit");
    assert_eq!(notice(&app), "Talk was changed outside yPresent; the deck now shows the file as it is.");
    assert_eq!(app.ui(|ui| ui.get_current_title().to_string()), "Theirs two", "and the canvas shows it");
}

#[test]
fn an_edit_to_a_deck_with_unsaved_changes_is_a_conflict_and_the_content_stays() {
    let (app, path) = with_file("conflict", &["One"]);
    type_title(&app, "Mine");
    outside_writes(&app, &path, &deck_text("Talk", &["Theirs"]));
    assert!(notice(&app).contains("was changed outside yPresent while this deck has unsaved changes"), "{}", notice(&app));
    assert_eq!(outline(&app), vec!["Mine"]);
    let err = act(&app, "save", none()).unwrap_err();
    assert!(err.contains("changed on disk since it was opened"), "{err}");
    assert_eq!(read_deck(&path).slides[0].title, "Theirs");
}

#[test]
fn our_own_write_is_never_taken_for_another_programs() {
    let (app, _) = with_file("own-write", &["One"]);
    act(&app, "set_slide", json!({ "index": 0, "title": "Two" })).unwrap();
    act(&app, "save", none()).unwrap();
    app.look_at_files();
    settle(&app);
    assert_eq!(notice(&app), "");
    assert!(!modified(&app));
}

#[test]
fn a_file_renamed_while_open_is_followed_to_its_new_name() {
    let (app, path) = with_file("renamed", &["One"]);
    let there = path.with_file_name("renamed.ydeck");
    std::fs::rename(&path, &there).unwrap();
    app.look_at_files();
    settle(&app);
    assert_eq!(path_of(&app), Some(there.clone()));
    assert_eq!(notice(&app), format!("This deck moved. It is {} now.", there.display()));
    assert!(!modified(&app));
    act(&app, "set_slide", json!({ "index": 0, "title": "Two" })).unwrap();
    act(&app, "save", none()).unwrap();
    assert_eq!(read_deck(&there).slides[0].title, "Two");
}

#[test]
fn a_file_that_is_gone_is_said_and_the_deck_is_unsaved_from_then_on() {
    let (app, path) = with_file("gone", &["One"]);
    std::fs::remove_file(&path).unwrap();
    app.look_at_files();
    settle(&app);
    assert!(notice(&app).contains("is not there any more, and nothing with the same name and the same contents"), "{}", notice(&app));
    assert!(modified(&app), "its content is on no disk at all");
    assert_eq!(state(&app)["dirty"], true);
    app.run_writes();
    assert!(super::draft_file(&app).exists(), "so its draft is kept");
}

#[test]
fn a_late_reread_of_the_previous_file_does_not_land_on_the_deck_opened_since() {
    let (app, a) = with_file("late-a", &["A"]);
    let b = scratch("late-b").join("b.ydeck");
    write_deck(&b, "B", &["B"]);
    act(&app, "open", json!({ "path": b.display().to_string() })).unwrap();
    // A's re-read lands after B is open.
    let theirs = described(a.clone(), deck_text("A", &["A SECRET written outside"]));
    app.run(Command::Reread { from: a, result: Ok(Box::new(theirs)) });
    assert_eq!(path_of(&app), Some(b.clone()));
    assert_eq!(outline(&app), vec!["B"]);
    assert!(!notice(&app).contains("changed outside"), "{}", notice(&app));
    act(&app, "set_slide", json!({ "index": 0, "title": "B2" })).unwrap();
    act(&app, "save", none()).unwrap();
    assert_eq!(read_deck(&b).slides[0].title, "B2");
}

#[test]
fn a_file_that_stopped_being_a_deck_keeps_the_content_and_says_why() {
    let (app, path) = with_file("not-a-deck", &["One"]);
    outside_writes(&app, &path, "someone replaced it with prose");
    assert!(notice(&app).contains("cannot be shown here"), "{}", notice(&app));
    assert_eq!(outline(&app), vec!["One"], "the deck keeps what it had");
    let err = act(&app, "save", none()).unwrap_err();
    assert!(err.contains("changed on disk"), "{err}");
    assert_eq!(super::file(&path), "someone replaced it with prose");
}

#[test]
fn a_clean_deck_is_not_made_a_conflict_by_a_change_that_was_typed_and_typed_back() {
    let (app, path) = with_file("typed-back", &["One"]);
    type_title(&app, "Two");
    type_title(&app, "One");
    outside_writes(&app, &path, &deck_text("Talk", &["Theirs"]));
    assert_eq!(outline(&app), vec!["Theirs"], "nothing was unsaved, so the file is followed");
    assert!(!notice(&app).contains("unsaved changes"), "{}", notice(&app));
}
