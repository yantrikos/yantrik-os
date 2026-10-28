//! Opening a deck or starting a new one over a deck with unsaved changes: the deck being replaced
//! is written to a file of its own first and the caller is told where it went. If that write
//! fails the replacement does not happen at all: losing the work is the one outcome that is not
//! allowed.

use serde_json::json;
use yantrik_app::documents::disk::Step;

use super::{
    act, deck_folder, draft_file, failing, file, killed_and_started_again, modified, none, notice, outline, path_of,
    press, read_deck, scratch, settle, started, type_title, with_file, write_deck,
};
use crate::command::WindowAction;

fn other_deck(name: &str) -> std::path::PathBuf {
    let path = scratch(name).join("other.ydeck");
    write_deck(&path, "Other", &["other one"]);
    path
}

#[test]
fn an_open_over_an_unsaved_deck_with_no_file_writes_it_to_a_file_of_its_own_first() {
    let app = started();
    let dir = deck_folder(&app);
    act(&app, "set_slide", json!({ "index": 0, "title": "My unsaved work" })).unwrap();
    let other = other_deck("rescue-new");
    let answer = act(&app, "open", json!({ "path": other.display().to_string() })).unwrap();
    let kept = dir.join("untitled.ydeck");
    assert_eq!(answer["previous_deck_saved_to"], json!(kept.display().to_string()));
    assert_eq!(read_deck(&kept).slides[0].title, "My unsaved work");
    assert_eq!(outline(&app), vec!["other one"]);
    assert_eq!(answer["opened"], json!(other.display().to_string()));
    assert_eq!((answer["slides"].clone(), answer["deck_title"].clone()), (json!(1), json!("Other")));
    assert!(!modified(&app));
}

#[test]
fn an_open_over_an_unsaved_named_deck_saves_it_where_it_lives() {
    let (app, path) = with_file("rescue-named", &["One"]);
    act(&app, "set_slide", json!({ "index": 0, "title": "Edited" })).unwrap();
    let other = other_deck("rescue-named-2");
    let answer = act(&app, "open", json!({ "path": other.display().to_string() })).unwrap();
    assert_eq!(answer["previous_deck_saved_to"], json!(path.display().to_string()));
    assert_eq!(read_deck(&path).slides[0].title, "Edited");
}

#[test]
fn a_clean_deck_is_replaced_with_nothing_to_say() {
    let (app, _) = with_file("rescue-clean", &["One"]);
    let other = other_deck("rescue-clean-2");
    let answer = act(&app, "open", json!({ "path": other.display().to_string() })).unwrap();
    assert_eq!(answer["previous_deck_saved_to"], json!(null));
}

#[test]
fn new_deck_over_an_unsaved_deck_does_the_same() {
    let app = started();
    let dir = deck_folder(&app);
    act(&app, "set_slide", json!({ "index": 0, "title": "Do not lose me" })).unwrap();
    let answer = act(&app, "new_deck", json!({ "title": "Next" })).unwrap();
    assert_eq!(answer["previous_deck_saved_to"], json!(dir.join("untitled.ydeck").display().to_string()));
    assert_eq!(read_deck(&dir.join("untitled.ydeck")).slides[0].title, "Do not lose me");
    assert_eq!((answer["deck_title"].clone(), outline(&app)), (json!("Next"), vec!["Title Slide".to_string()]));
}

#[test]
fn a_file_that_is_not_a_deck_costs_the_unsaved_deck_nothing() {
    let app = started();
    let dir = deck_folder(&app);
    act(&app, "set_slide", json!({ "index": 0, "title": "Mine" })).unwrap();
    let bad = scratch("rescue-bad").join("bad.ydeck");
    std::fs::write(&bad, "not a deck").unwrap();
    act(&app, "open", json!({ "path": bad.display().to_string() })).unwrap_err();
    assert_eq!(outline(&app), vec!["Mine"]);
    assert!(modified(&app), "still unsaved, with its draft");
    assert!(super::listing(&dir).is_empty(), "and nothing was written for it");
}

#[test]
fn a_rescue_that_cannot_be_written_stops_the_replacement() {
    let app = started();
    deck_folder(&app);
    let disk = failing(&app);
    disk.fail(Step::Write, ".ydeck", None);
    act(&app, "set_slide", json!({ "index": 0, "title": "Precious" })).unwrap();
    let other = other_deck("rescue-fails");
    let err = act(&app, "open", json!({ "path": other.display().to_string() })).unwrap_err();
    assert!(err.contains("Save failed") && err.ends_with("The deck was not replaced."), "{err}");
    assert_eq!(outline(&app), vec!["Precious"]);
    assert!(modified(&app));
    let err = act(&app, "new_deck", none()).unwrap_err();
    assert!(err.ends_with("The deck was not replaced."), "{err}");
    assert_eq!(outline(&app), vec!["Precious"]);
}

#[test]
fn edits_made_while_the_rescue_is_written_stop_the_replacement_and_are_not_lost() {
    let app = started();
    deck_folder(&app);
    act(&app, "set_slide", json!({ "index": 0, "title": "First draft" })).unwrap();
    let other = other_deck("rescue-meanwhile");
    let pending = app.act("open", json!({ "path": other.display().to_string() })).unwrap();
    // The file is read; the rescue save is now in flight.
    assert!(app.run_one_job());
    assert_eq!(app.jobs_waiting(), 1);
    type_title(&app, "Typed while it was written");
    settle(&app);
    let err = pending.settle().unwrap_err();
    assert!(err.contains("The deck changed while it was being saved, so it was not replaced"), "{err}");
    assert_eq!(outline(&app), vec!["Typed while it was written"]);
    assert!(modified(&app));
}

#[test]
fn the_windows_open_and_a_launch_say_where_the_deck_they_replaced_went() {
    let app = started();
    let dir = deck_folder(&app);
    write_deck(&dir.join("zz-recent.ydeck"), "Recent", &["recent"]);
    act(&app, "set_slide", json!({ "index": 0, "title": "Unsaved one" })).unwrap();
    press(&app, WindowAction::OpenRecent);
    let kept = dir.join("untitled.ydeck");
    assert_eq!(notice(&app), format!("The deck you were editing was saved to {}.", kept.display()));
    assert_eq!(outline(&app), vec!["recent"]);
}

#[test]
fn a_deck_that_was_replaced_leaves_no_draft_to_be_offered_again() {
    let app = started();
    deck_folder(&app);
    act(&app, "set_slide", json!({ "index": 0, "title": "Gone on purpose" })).unwrap();
    app.run_writes();
    let draft = draft_file(&app);
    assert!(draft.exists());
    let other = other_deck("rescue-draft");
    act(&app, "open", json!({ "path": other.display().to_string() })).unwrap();
    app.run_writes();
    assert!(!draft.exists(), "saved elsewhere, so the draft goes with the deck");
    app.shutdown();
    let again = killed_and_started_again(app);
    assert_eq!(notice(&again), "", "nothing is offered back");
}

#[test]
fn a_deck_whose_file_was_changed_by_another_program_is_rescued_beside_it_not_over_it() {
    let (app, path) = with_file("rescue-conflict", &["One"]);
    let dir = deck_folder(&app);
    act(&app, "set_slide", json!({ "index": 0, "title": "Mine" })).unwrap();
    super::outside_writes(&app, &path, &super::deck_text("Theirs", &["Theirs"]));
    let other = other_deck("rescue-conflict-2");
    let answer = act(&app, "open", json!({ "path": other.display().to_string() })).unwrap();
    let kept = answer["previous_deck_saved_to"].as_str().unwrap().to_string();
    assert!(kept.starts_with(&dir.display().to_string()), "{kept}");
    assert_eq!(read_deck(std::path::Path::new(&kept)).slides[0].title, "Mine");
    assert_eq!(read_deck(&path).slides[0].title, "Theirs", "their version is where they left it");
    assert_eq!(path_of(&app), Some(other));
    assert!(!file(&path).contains("Mine"));
}

#[test]
fn opening_the_deck_that_is_open_with_unsaved_changes_is_refused_and_a_clean_one_is_read_again() {
    let (app, path) = with_file("rescue-same", &["One"]);
    type_title(&app, "Edited");
    let ask = json!({ "path": path.display().to_string() });
    let err = act(&app, "open", ask.clone()).unwrap_err();
    assert!(err.contains("already open and has unsaved changes"), "{err}");
    assert_eq!(outline(&app), vec!["Edited"], "nothing is dropped, and nothing is written over the file");
    assert_eq!(read_deck(&path).slides[0].title, "One");
    act(&app, "save", none()).unwrap();
    write_deck(&path, "Talk", &["Changed on disk"]);
    act(&app, "open", ask).unwrap();
    assert_eq!(outline(&app), vec!["Changed on disk"], "a clean deck opened again is the file as it is");
}
