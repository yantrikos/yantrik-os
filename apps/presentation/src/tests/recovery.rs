//! The deck a start opens again: the draft the last run left, the old build's recovery file once,
//! and what a file named on the command line does to either. Nothing here removes a draft, and
//! nothing is offered twice.

use std::path::{Path, PathBuf};

use serde_json::json;
use yantrik_app::paths::Paths;
use yantrik_testing::TestApp;

use super::{
    act, content_of, deck_folder, draft_file, killed_and_started_again, listing, modified, notice, outline, path_of,
    press, read_deck, scratch, settle, started, state, type_title, with_file, write_deck,
};
use crate::command::WindowAction;
use crate::files::drafts::{DeckDraft, VERSION};
use crate::YPresent;

/// A start at `paths` with these arguments.
fn start_at(paths: Paths, args: Vec<std::ffi::OsString>) -> TestApp<YPresent> {
    let app = TestApp::<YPresent>::start_at(paths, args);
    deck_folder(&app);
    settle(&app);
    app
}

/// The old build's recovery file, as it wrote it.
fn old_file(paths: &Paths, path: Option<&Path>, titles: &[&str]) -> PathBuf {
    let file = paths.state_dir().join("recovery.json");
    std::fs::create_dir_all(paths.state_dir()).unwrap();
    let text = json!({ "version": 1, "path": path, "content": content_of("Old", titles) });
    std::fs::write(&file, text.to_string()).unwrap();
    file
}

/// A draft in the lane, as this build writes one.
fn lane_draft(paths: &Paths, name: &str, path: Option<&Path>, titles: &[&str], at: u64) {
    let draft = DeckDraft {
        version: VERSION,
        path: path.map(Path::to_path_buf),
        content: content_of("Lane", titles),
        on_disk: None,
        at,
    };
    let file = yantrik_app::drafts::path_in(paths.state_dir(), name);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, draft.bytes()).unwrap();
}

#[test]
fn a_kill_after_the_draft_landed_loses_nothing() {
    let app = started();
    act(&app, "set_slide", json!({ "index": 0, "title": "Typed", "body": "and not kept" })).unwrap();
    act(&app, "add_slide", json!({ "title": "Two" })).unwrap();
    app.run_writes();
    assert!(draft_file(&app).exists());
    let again = killed_and_started_again(app);
    assert_eq!(outline(&again), vec!["Typed", "Two"]);
    assert_eq!(app_body(&again), "and not kept");
    assert_eq!(state(&again)["recovered"], true);
    assert_eq!(super::path_of(&again), None, "a deck that never had a file still has none");
    assert!(modified(&again), "a recovered deck is by construction not what is on disk");
    assert_eq!(notice(&again), "Recovered unsaved changes. Save keeps them.");
}

fn app_body(app: &TestApp<YPresent>) -> String {
    app.model(|m| m.deck().slides()[0].body.clone())
}

#[test]
fn a_deck_with_a_file_comes_back_against_that_file_and_saves_into_it() {
    let (app, path) = with_file("recovered-named", &["One"]);
    type_title(&app, "Edited after saving");
    app.run_writes();
    let again = killed_and_started_again(app);
    assert_eq!(path_of(&again), Some(path.clone()));
    assert_eq!(notice(&again), format!("Recovered unsaved changes to {}. Save keeps them.", path.display()));
    act(&again, "save", super::none()).expect("the file is what the draft was made against, so Save is not a conflict");
    assert_eq!(read_deck(&path).slides[0].title, "Edited after saving");
    assert!(!modified(&again));
    assert_eq!(state(&again)["recovered"], false);
}

#[test]
fn a_deck_saved_or_typed_back_to_its_file_keeps_no_draft() {
    let (app, _) = with_file("no-draft", &["One"]);
    type_title(&app, "Two");
    app.run_writes();
    let draft = draft_file(&app);
    assert!(draft.exists());
    type_title(&app, "One");
    app.run_writes();
    assert!(!draft.exists(), "typed back: nothing is owed");
    type_title(&app, "Three");
    press(&app, WindowAction::Save);
    app.run_writes();
    assert!(!draft.exists(), "saved: the draft is cleared, not offered again");
    app.shutdown();
    assert_eq!(notice(&killed_and_started_again(app)), "");
}

#[test]
fn a_draft_that_is_not_one_of_ours_is_left_alone_and_said() {
    let paths = Paths::temporary("presentation");
    let file = yantrik_app::drafts::path_in(paths.state_dir(), "somebody-elses");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "{ not json").unwrap();
    let app = start_at(paths, Vec::new());
    assert!(notice(&app).contains("1 more draft could not be opened at once and is kept"), "{}", notice(&app));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not json", "nothing removes a draft");
    assert_eq!(outline(&app), vec!["Title Slide"]);
}

#[test]
fn the_newest_of_several_drafts_opens_and_the_rest_are_counted() {
    let paths = Paths::temporary("presentation");
    lane_draft(&paths, "deck-1-1", None, &["older"], 100);
    lane_draft(&paths, "deck-2-1", None, &["newer"], 200);
    let app = start_at(paths, Vec::new());
    assert_eq!(outline(&app), vec!["newer"]);
    assert!(notice(&app).contains("1 more draft could not be opened at once and is kept"), "{}", notice(&app));
}

// ── The old build's recovery file ────────────────────────────────────

#[test]
fn the_old_builds_recovery_file_opens_once_and_is_set_aside() {
    let paths = Paths::temporary("presentation");
    let old = old_file(&paths, None, &["from the old build"]);
    let app = start_at(paths.clone(), Vec::new());
    assert_eq!(outline(&app), vec!["from the old build"]);
    assert_eq!(notice(&app), "Recovered unsaved changes. Save keeps them.");
    app.run_writes();
    settle(&app);
    assert!(!old.exists(), "set aside once its draft is the lane's");
    assert!(paths.state_dir().join("recovery.json.imported").exists(), "backed up, never deleted");
    let again = killed_and_started_again(app);
    assert_eq!(outline(&again), vec!["from the old build"], "the lane has it now");
    assert!(!paths.state_dir().join("recovery.json").exists());
}

#[test]
fn an_old_recovery_file_that_cannot_be_read_is_left_exactly_as_it_was_and_said() {
    let paths = Paths::temporary("presentation");
    let file = paths.state_dir().join("recovery.json");
    std::fs::create_dir_all(paths.state_dir()).unwrap();
    for (bytes, why) in [("{ not json", "not a yPresent deck"), ("{\"version\":99,\"content\":{}}", "version 99")] {
        std::fs::write(&file, bytes).unwrap();
        let app = start_at(paths.clone(), Vec::new());
        assert!(notice(&app).contains("has been left alone") || notice(&app).contains("left alone"), "{}", notice(&app));
        assert!(notice(&app).contains(why), "{}", notice(&app));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), bytes);
        assert_eq!(outline(&app), vec!["Title Slide"], "an empty deck, not a guess");
        app.shutdown();
    }
}

#[test]
fn the_old_decks_file_is_not_trusted_to_be_what_it_was_typed_against() {
    let paths = Paths::temporary("presentation");
    let dir = scratch("legacy-path");
    let deck_path = dir.join("talk.ydeck");
    write_deck(&deck_path, "Talk", &["On disk"]);
    old_file(&paths, Some(&deck_path), &["Typed in the old build"]);
    let app = start_at(paths, Vec::new());
    assert_eq!(path_of(&app), Some(deck_path.clone()));
    // Nothing says the file is still what the deck was typed against, so Save keeps both.
    press(&app, WindowAction::Save);
    assert_eq!(read_deck(&deck_path).slides[0].title, "On disk", "the file is not written over");
    let now = path_of(&app).unwrap();
    assert_ne!(now, deck_path);
    assert_eq!(read_deck(&now).slides[0].title, "Typed in the old build");
    assert!(notice(&app).contains("came back from an older recovery file"), "{}", notice(&app));
}

#[test]
fn the_old_file_opens_first_when_the_lane_holds_another_deck_and_that_one_is_counted() {
    let paths = Paths::temporary("presentation");
    lane_draft(&paths, "deck-9-9", None, &["lane deck"], 500);
    old_file(&paths, None, &["old deck"]);
    let app = start_at(paths.clone(), Vec::new());
    assert_eq!(outline(&app), vec!["old deck"], "the one that is in no other place cannot wait");
    assert!(notice(&app).contains("1 more draft could not be opened at once"), "{}", notice(&app));
    app.run_writes();
    settle(&app);
    assert!(paths.state_dir().join("recovery.json.imported").exists());
}

#[test]
fn an_old_file_already_in_the_lane_is_set_aside_at_once_and_not_offered_twice() {
    let paths = Paths::temporary("presentation");
    // A start that ended after the lane took the deck and before the old file was set aside.
    let draft = DeckDraft { version: VERSION, path: None, content: content_of("Old", &["same"]), on_disk: None, at: 1 };
    let file = yantrik_app::drafts::path_in(paths.state_dir(), "deck-1-1");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, draft.bytes()).unwrap();
    let old = old_file(&paths, None, &["same"]);
    let app = start_at(paths.clone(), Vec::new());
    assert_eq!(outline(&app), vec!["same"]);
    settle(&app);
    assert!(!old.exists(), "archived at the first command");
    assert_eq!(listing(&paths.state_dir().join("drafts")), vec!["deck-1-1.json"], "and nothing was made twice");
}

#[test]
fn a_file_named_on_the_command_line_wins_the_window_and_leaves_every_draft_where_it_is() {
    let paths = Paths::temporary("presentation");
    lane_draft(&paths, "deck-3-3", None, &["unsaved from last time"], 300);
    let old = old_file(&paths, None, &["older still"]);
    let given = scratch("argv").join("given.ydeck");
    write_deck(&given, "Given", &["from the command line"]);
    let app = start_at(paths.clone(), vec![given.clone().into()]);
    assert_eq!(path_of(&app), Some(given));
    let said = notice(&app);
    assert!(said.contains("An unsaved draft from a previous session is waiting in"), "{said}");
    assert!(said.contains(&paths.state_dir().join("drafts").display().to_string()), "{said}");
    assert!(said.contains(&paths.state_dir().join("recovery.json").display().to_string()), "the old file's real place: {said}");
    assert!(old.exists(), "not touched");
    app.shutdown();
    let next = start_at(paths, Vec::new());
    assert!(outline(&next) == vec!["older still"] || outline(&next) == vec!["unsaved from last time"], "offered at the next start with no file");
}

#[test]
fn an_old_deck_saved_before_its_draft_landed_still_has_its_file_set_aside() {
    let paths = Paths::temporary("presentation");
    let old = old_file(&paths, None, &["from the old build"]);
    let app = start_at(paths.clone(), Vec::new());
    assert!(old.exists(), "its draft has not reached the disk, so the file waits");
    // Saved before the draft landed: the draft is discarded, and the old file's deck is in a file.
    let saved = act(&app, "save", super::none()).unwrap();
    settle(&app);
    assert!(!old.exists(), "set aside when the draft was discarded, not left to be offered again");
    assert!(paths.state_dir().join("recovery.json.imported").exists());
    assert_eq!(read_deck(std::path::Path::new(saved["path"].as_str().unwrap())).slides[0].title, "from the old build");
    app.shutdown();
    assert_eq!(notice(&killed_and_started_again(app)), "", "and nothing is offered at the next start");
}

#[test]
fn a_deck_from_the_old_file_that_names_a_file_is_replaced_into_a_file_of_its_own_first_time() {
    let paths = Paths::temporary("presentation");
    let dir = scratch("legacy-replace");
    let deck_path = dir.join("talk.ydeck");
    write_deck(&deck_path, "Talk", &["On disk"]);
    old_file(&paths, Some(&deck_path), &["Typed in the old build"]);
    let app = start_at(paths, Vec::new());
    let folder = super::deck_folder(&app);
    let answer = act(&app, "new_deck", super::none()).expect("not a false conflict on the first try");
    let kept = std::path::PathBuf::from(answer["previous_deck_saved_to"].as_str().unwrap());
    assert_eq!(kept.parent(), Some(folder.as_path()));
    assert_eq!(read_deck(&kept).slides[0].title, "Typed in the old build");
    assert_eq!(read_deck(&deck_path).slides[0].title, "On disk", "the file is not written over");
}

#[test]
fn an_unreadable_old_file_and_a_waiting_draft_are_both_said_when_a_file_is_named() {
    let paths = Paths::temporary("presentation");
    lane_draft(&paths, "deck-3-3", None, &["waiting"], 300);
    std::fs::create_dir_all(paths.state_dir()).unwrap();
    std::fs::write(paths.state_dir().join("recovery.json"), "{ not json").unwrap();
    let given = scratch("argv-both").join("given.ydeck");
    write_deck(&given, "Given", &["given"]);
    let app = start_at(paths.clone(), vec![given.into()]);
    let said = notice(&app);
    assert!(said.contains(&paths.state_dir().join("drafts").display().to_string()), "{said}");
    assert!(said.contains("recovery file") && said.contains("left alone"), "{said}");
    assert!(app.model(|m| m.held_aside.is_some()), "and it is said again after the open");
}
