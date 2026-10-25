//! A start after the editor ended with unsaved tabs: a crash, a `kill -9`. Every tab's draft is
//! opened again, as the old build's recovery file always was, and the old build's file itself is
//! read once, its tabs handed to the lane, and set aside.

use serde_json::json;
use yantrik_testing::TestApp;

use super::{act, draft_file, file, killed_and_started_again, modified, notice, press, settle, state, tabs, text, typed, with_file};
use crate::command::WindowAction;
use crate::recovery::RECOVERED;
use crate::TextEditor;

#[test]
fn a_kill_after_the_drafts_landed_loses_nothing() {
    let (app, path) = with_file("killed", "on disk\n");
    typed(&app, "typed before the crash\n");
    act(&app, "new", json!({ "text": "an untitled draft 🦀" })).unwrap();
    app.run_writes();
    let app = killed_and_started_again(app);
    assert_eq!(tabs(&app), 2, "both tabs came back");
    assert_eq!(notice(&app), RECOVERED);
    assert_eq!(text(&app), "typed before the crash\n", "in the order they were opened");
    assert!(modified(&app), "a recovered tab is unsaved until it is saved");
    assert_eq!(file(&path), "on disk\n", "the file was never touched");
    let s = state(&app);
    assert_eq!(s["tabs"][1]["name"], "Untitled");
    assert_eq!(s["tabs"][1]["opened_by"], "person", "a recovered tab is the person's");
    press(&app, WindowAction::Save);
    assert_eq!(file(&path), "typed before the crash\n");
    assert!(!modified(&app));
    app.run_writes();
    assert!(!draft_file(&app).exists(), "a saved tab has no draft");
}

#[test]
fn a_tab_saved_or_typed_back_to_its_file_keeps_no_draft() {
    let (app, _) = with_file("no-draft", "same\n");
    typed(&app, "other\n");
    app.run_writes();
    assert!(draft_file(&app).exists());
    assert_eq!(state(&app)["recovery"], "Draft recovery up to date");
    typed(&app, "same\n");
    app.run_writes();
    assert!(!modified(&app) && !draft_file(&app).exists(), "typed back to its file's text is saved again");
    let again = killed_and_started_again(app);
    assert_eq!(notice(&again), "", "nothing to recover");
}

#[test]
fn a_recovered_tab_whose_file_moved_on_is_a_conflict_never_a_silent_replace() {
    let (app, path) = with_file("moved-on", "v1\n");
    typed(&app, "mine\n");
    app.run_writes();
    std::fs::write(&path, "theirs since\n").unwrap();
    let app = killed_and_started_again(app);
    app.look_at_files();
    settle(&app);
    assert!(notice(&app).contains("while this tab has unsaved changes"), "{}", notice(&app));
    assert!(act(&app, "save", json!({})).is_err());
    assert_eq!(file(&path), "theirs since\n");
}

#[test]
fn a_draft_that_cannot_be_written_is_said() {
    let app = TestApp::<TextEditor>::start();
    let drafts = app.paths().state_dir().join(yantrik_app::drafts::DIR);
    let _ = std::fs::remove_dir_all(&drafts);
    std::fs::create_dir_all(app.paths().state_dir()).unwrap();
    std::fs::write(&drafts, "not a folder").unwrap();
    typed(&app, "unprotected");
    assert_eq!(state(&app)["recovery"], "Protecting draft…");
    app.run_writes();
    assert_eq!(state(&app)["recovery"], "Recovery unavailable");
    assert!(notice(&app).starts_with("Draft recovery failed: "), "{}", notice(&app));
}

/// The old build's file: every unsaved tab, in one JSON list.
fn legacy(app_paths: &yantrik_app::paths::Paths, body: &str) {
    let dir = app_paths.state_dir();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("drafts.json"), body).unwrap();
}

#[test]
fn the_old_builds_recovery_file_is_opened_once_and_set_aside_once_its_drafts_are_the_lanes() {
    let paths = yantrik_app::paths::Paths::temporary("editor");
    let file = super::scratch("legacy").join("a.txt");
    std::fs::write(&file, "disk").unwrap();
    let old = json!([
        { "path": file, "text": "draft 🦀", "baseline": "disk", "recovered": false },
        { "path": null, "text": "untitled", "baseline": "", "recovered": true },
    ]);
    legacy(&paths, &old.to_string());
    let app = TestApp::<TextEditor>::start_at(paths.clone(), Vec::new());
    settle(&app);
    assert_eq!(tabs(&app), 2);
    assert_eq!(notice(&app), RECOVERED);
    assert!(paths.state_dir().join("drafts.json").exists(), "kept until its tabs are the lane's");
    app.run_writes();
    settle(&app);
    assert!(!paths.state_dir().join("drafts.json").exists());
    assert!(paths.state_dir().join("drafts.json.imported").exists(), "backed up, never deleted");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "disk", "the original is untouched");
    let again = killed_and_started_again(app);
    assert_eq!(tabs(&again), 2, "each tab once, from the lane");
    assert_eq!(text(&again), "draft 🦀");
}

#[test]
fn an_old_recovery_file_that_cannot_be_read_is_left_exactly_as_it_was() {
    let paths = yantrik_app::paths::Paths::temporary("editor");
    legacy(&paths, "{ not json");
    let app = TestApp::<TextEditor>::start_at(paths.clone(), Vec::new());
    settle(&app);
    assert!(notice(&app).starts_with("Recovery file could not be read"), "{}", notice(&app));
    assert_eq!(tabs(&app), 1);
    app.run_writes();
    settle(&app);
    assert_eq!(std::fs::read_to_string(paths.state_dir().join("drafts.json")).unwrap(), "{ not json");
}
