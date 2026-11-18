//! A start after yDoc ended with unsaved text: a crash, a `kill -9`. The draft is opened again,
//! as the old build's recovery file always was, and the old build's file itself is read once,
//! handed to the lane and set aside.

use serde_json::json;
use yantrik_app::paths::Paths;
use yantrik_testing::TestApp;

use super::{act, draft_file, file, killed_and_started_again, modified, none, notice, path_of, press, scratch, settle, started, state, text, typed, with_file};
use crate::command::WindowAction;
use crate::files::legacy;
use crate::recovery::RECOVERED;
use crate::YDoc;

#[test]
fn a_kill_after_the_draft_landed_loses_nothing() {
    let (app, path) = with_file("killed", "on disk\n");
    typed(&app, "typed before the crash\n");
    app.run_writes();
    let app = killed_and_started_again(app);
    assert_eq!(notice(&app), RECOVERED);
    assert_eq!(text(&app), "typed before the crash\n");
    assert!(modified(&app), "a recovered draft is unsaved until it is saved");
    let s = state(&app);
    assert_eq!((s["recovered_draft"].clone(), s["dirty"].clone()), (json!(true), json!(true)));
    assert_eq!(file(&path), "on disk\n", "the file was never touched");
    assert_eq!(path_of(&app), Some(path.clone()));
    press(&app, WindowAction::Save);
    assert_eq!(file(&path), "typed before the crash\n");
    assert!(!modified(&app));
    assert_eq!(state(&app)["recovered_draft"], false);
    app.run_writes();
    assert!(!draft_file(&app).exists(), "a saved document has no draft");
}

#[test]
fn an_untitled_draft_comes_back_untitled() {
    let app = started();
    typed(&app, "# Never saved\n");
    app.run_writes();
    let again = killed_and_started_again(app);
    assert_eq!(text(&again), "# Never saved\n");
    assert_eq!(path_of(&again), None);
    assert_eq!(state(&again)["title"], "Never saved");
}

#[test]
fn a_document_saved_or_typed_back_to_its_file_keeps_no_draft() {
    let (app, _) = with_file("no-draft", "same\n");
    typed(&app, "other\n");
    app.run_writes();
    assert!(draft_file(&app).exists());
    typed(&app, "same\n");
    app.run_writes();
    assert!(!modified(&app) && !draft_file(&app).exists(), "typed back to its file's text is saved again");
    let again = killed_and_started_again(app);
    assert_eq!(notice(&again), "", "nothing to recover");
}

#[test]
fn a_recovered_draft_whose_file_moved_on_is_a_conflict_never_a_silent_replace() {
    let (app, path) = with_file("moved-on", "v1\n");
    typed(&app, "mine\n");
    app.run_writes();
    std::fs::write(&path, "theirs since\n").unwrap();
    let app = killed_and_started_again(app);
    app.look_at_files();
    settle(&app);
    assert!(notice(&app).contains("while this document has unsaved changes"), "{}", notice(&app));
    assert!(act(&app, "save", none()).is_err());
    assert_eq!(file(&path), "theirs since\n");
}

#[test]
fn a_draft_that_cannot_be_written_is_said() {
    let app = TestApp::<YDoc>::start();
    let drafts = app.paths().state_dir().join(yantrik_app::drafts::DIR);
    let _ = std::fs::remove_dir_all(&drafts);
    std::fs::create_dir_all(app.paths().state_dir()).unwrap();
    std::fs::write(&drafts, "not a folder").unwrap();
    typed(&app, "unprotected");
    app.run_writes();
    assert!(notice(&app).starts_with("Draft recovery failed: "), "{}", notice(&app));
}

/// The old build's file: the unsaved document, in the folder it kept it in.
fn old_file(paths: &Paths, body: &str) -> std::path::PathBuf {
    let dir = legacy::old_dir(paths.state_dir());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(legacy::NAME), body).unwrap();
    dir
}

#[test]
fn the_old_builds_recovery_file_is_opened_once_and_set_aside_once_its_draft_is_the_lanes() {
    let paths = Paths::temporary("documents");
    let file_on_disk = scratch("legacy").join("a.md");
    std::fs::write(&file_on_disk, "disk").unwrap();
    let old = json!({
        "path": file_on_disk, "text": "draft 🦀", "baseline": "disk",
        "stamp": { "bytes": 4, "modified_unix": 1, "modified_nanos": 0 }, "recovered": true,
    });
    let dir = old_file(&paths, &old.to_string());
    let app = TestApp::<YDoc>::start_at(paths.clone(), Vec::new());
    settle(&app);
    assert_eq!(notice(&app), RECOVERED);
    assert_eq!(text(&app), "draft 🦀");
    assert_eq!(path_of(&app), Some(file_on_disk.clone()));
    assert!(dir.join(legacy::NAME).exists(), "kept until its document is the lane's");
    app.run_writes();
    settle(&app);
    assert!(!dir.join(legacy::NAME).exists());
    assert!(dir.join(legacy::ARCHIVED).exists(), "backed up, never deleted");
    assert_eq!(std::fs::read_to_string(&file_on_disk).unwrap(), "disk", "the original is untouched");
    let again = killed_and_started_again(app);
    assert_eq!(text(&again), "draft 🦀", "from the lane, once");
}

#[test]
fn an_old_recovery_file_that_cannot_be_read_is_left_exactly_as_it_was() {
    let paths = Paths::temporary("documents");
    let dir = old_file(&paths, "{ not json");
    let app = TestApp::<YDoc>::start_at(paths.clone(), Vec::new());
    settle(&app);
    assert!(notice(&app).starts_with("The recovery file could not be read"), "{}", notice(&app));
    assert!(notice(&app).ends_with("It has been left alone."), "{}", notice(&app));
    assert_eq!(text(&app), "");
    app.run_writes();
    settle(&app);
    assert_eq!(std::fs::read_to_string(dir.join(legacy::NAME)).unwrap(), "{ not json");
}

#[test]
fn a_file_named_on_the_command_line_wins_the_window_and_the_draft_is_left_where_it_is() {
    let (app, _) = with_file("hold", "disk\n");
    typed(&app, "my unsaved draft\n");
    app.run_writes();
    let paths = app.paths();
    let draft = draft_file(&app);
    drop(app);
    let given = scratch("hold-argv").join("given.md");
    std::fs::write(&given, "# Given\n").unwrap();
    let again = TestApp::<YDoc>::start_at(paths.clone(), vec![given.clone().into()]);
    settle(&again);
    assert_eq!(path_of(&again), Some(given));
    assert!(notice(&again).contains("An unsaved draft from a previous session is waiting in"), "{}", notice(&again));
    assert!(notice(&again).contains("open yDoc with no file to pick it up"), "{}", notice(&again));
    assert!(draft.exists(), "left exactly where it is");
    again.run_writes();
    assert!(draft.exists(), "this session's own saves do not write over it");
    // The next start with no file offers it.
    drop(again);
    let next = TestApp::<YDoc>::start_at(paths, Vec::new());
    settle(&next);
    assert_eq!(text(&next), "my unsaved draft\n");
}

/// How many drafts the lane holds.
fn lane_count(paths: &Paths) -> usize {
    std::fs::read_dir(paths.state_dir().join(yantrik_app::drafts::DIR))
        .map(|d| d.filter_map(Result::ok).filter(|e| e.path().extension().is_some_and(|x| x == "json")).count())
        .unwrap_or(0)
}

const UNTITLED_X: &str = r#"{ "path": null, "text": "LEGACY X only copy", "baseline": "" }"#;

#[test]
fn the_old_file_is_imported_once_even_when_the_lane_already_holds_another_draft() {
    let (app, _) = with_file("legacy-lane", "disk\n");
    typed(&app, "Y edited\n");
    app.run_writes();
    let paths = app.paths();
    drop(app);
    let dir = old_file(&paths, UNTITLED_X);
    let first = TestApp::<YDoc>::start_at(paths.clone(), Vec::new());
    settle(&first);
    assert_eq!(text(&first), "LEGACY X only copy", "the old text is in front, not left out");
    assert!(notice(&first).contains("1 more draft"), "and the lane's draft is counted: {}", notice(&first));
    first.run_writes();
    settle(&first);
    assert!(!dir.join(legacy::NAME).exists() && dir.join(legacy::ARCHIVED).exists(), "set aside");
    assert_eq!(lane_count(&paths), 2, "both are the lane's now");
    drop(first);
    // Later starts offer each draft once: X is never offered again from the old folder.
    let second = TestApp::<YDoc>::start_at(paths.clone(), Vec::new());
    settle(&second);
    assert!(notice(&second).contains("1 more draft"), "{}", notice(&second));
    assert_eq!(lane_count(&paths), 2);
    assert!(!dir.join(legacy::NAME).exists());
}

#[test]
fn an_old_file_whose_draft_is_already_the_lanes_is_set_aside_at_once_and_not_offered_twice() {
    let paths = Paths::temporary("documents");
    let dir = old_file(&paths, UNTITLED_X);
    let first = TestApp::<YDoc>::start_at(paths.clone(), Vec::new());
    settle(&first);
    first.run_writes();
    settle(&first);
    drop(first);
    // A start killed after the draft landed and before the old file was set aside.
    std::fs::rename(dir.join(legacy::ARCHIVED), dir.join(legacy::NAME)).unwrap();
    let again = TestApp::<YDoc>::start_at(paths.clone(), Vec::new());
    settle(&again);
    assert_eq!(text(&again), "LEGACY X only copy");
    assert_eq!(notice(&again), RECOVERED, "once, with nothing more to say");
    assert!(!dir.join(legacy::NAME).exists(), "set aside without waiting for another save");
    assert!(dir.join(legacy::ARCHIVED).exists());
    assert_eq!(lane_count(&paths), 1);
}

#[test]
fn a_file_named_at_launch_says_where_the_old_file_is_not_the_lanes_folder() {
    let paths = Paths::temporary("documents");
    let dir = old_file(&paths, UNTITLED_X);
    let given = scratch("legacy-named").join("given.md");
    std::fs::write(&given, "# Given\n").unwrap();
    let app = TestApp::<YDoc>::start_at(paths, vec![given.into()]);
    settle(&app);
    assert!(notice(&app).contains(&dir.join(legacy::NAME).display().to_string()), "{}", notice(&app));
    assert!(dir.join(legacy::NAME).exists(), "left alone");
}

#[test]
fn the_old_file_is_set_aside_when_its_document_is_saved_before_the_draft_landed() {
    let paths = Paths::temporary("documents");
    let dir = old_file(&paths, UNTITLED_X);
    let app = TestApp::<YDoc>::start_at(paths, Vec::new());
    settle(&app);
    let target = scratch("legacy-saved").join("x.md");
    act(&app, "save_as", json!({ "path": target.display().to_string() })).unwrap();
    app.run_writes();
    settle(&app);
    assert_eq!(file(&target), "LEGACY X only copy");
    assert!(dir.join(legacy::ARCHIVED).exists() && !dir.join(legacy::NAME).exists(), "its text is in a file now");
}
