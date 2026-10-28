//! Saving through the framework's disk: a failing step, edits made while the file is written,
//! another program's edit never written over, and a replaced file's mode.

use serde_json::json;
use yantrik_app::documents::disk::Step;

use super::{act, failing, file, header, listing, modified, notice, raw, settle, shown, started, state, with_file};

#[test]
fn the_windows_save_writes_the_sheets_own_file_and_the_sheet_is_saved() {
    let (app, path) = with_file("window-save", "1\r\n");
    super::enter(&app, "changed");
    assert!(modified(&app));
    header(&app, "save");
    assert_eq!(file(&path), "changed\r\n");
    assert!(!modified(&app));
    assert!(!app.ui(|ui| ui.get_modified()));
    assert!(super::status(&app).starts_with(&format!("Saved {}", path.display())), "{}", super::status(&app));
    assert_eq!(listing(path.parent().unwrap()), vec!["budget.csv".to_string()]);
}

#[test]
fn a_failing_step_leaves_the_old_file_the_sheet_unsaved_and_nothing_behind() {
    let (app, path) = with_file("fails", "old\r\n");
    let disk = failing(&app);
    disk.fail(Step::Write, "budget.csv", Some(1));
    act(&app, "set_cell", json!({ "cell": "A1", "text": "new" })).unwrap();
    let err = act(&app, "save_over", json!({})).unwrap_err();
    assert!(err.starts_with("Save failed:") && err.contains("Your sheet is still open and unchanged"), "{err}");
    assert_eq!(file(&path), "old\r\n", "the old file is as it was");
    assert!(modified(&app), "nothing is called saved that is not");
    assert_eq!(listing(path.parent().unwrap()), vec!["budget.csv".to_string()], "no temporary file");
    assert_eq!(notice(&app), err);
    // The next try works: the failure was one step, once.
    act(&app, "save_over", json!({})).unwrap();
    assert_eq!(file(&path), "new\r\n");
    assert!(!modified(&app));
}

#[test]
fn edits_made_while_the_file_is_written_stay_unsaved() {
    let (app, path) = with_file("during", "1\r\n");
    act(&app, "set_cell", json!({ "cell": "A1", "text": "first" })).unwrap();
    // Save is started and has not landed; the person types another cell.
    app.ui(|ui| ui.invoke_command("save".into()));
    app.ui(|ui| {
        ui.invoke_cell_clicked(1, 1);
        ui.set_cell_data("second".into());
        ui.invoke_formula_submitted("second".into(), "down".into());
    });
    settle(&app);
    assert_eq!(file(&path), "first\r\n", "the file is of the sheet when the save began");
    assert!(modified(&app), "the second edit is not in it");
    assert_eq!(raw(&app, "B2"), "second");
    // Undoing it is the state that was written.
    act(&app, "undo", json!({})).unwrap();
    assert!(!modified(&app));
    assert_eq!(state(&app)["dirty"], false);
}

#[test]
fn undoing_back_to_what_was_saved_is_saved_again_but_retyping_it_is_an_edit() {
    // As it always was: unsaved is a place in the history, not a comparison with the file.
    let (app, _) = with_file("typed-back", "1\r\n");
    act(&app, "set_cell", json!({ "cell": "A1", "text": "2" })).unwrap();
    assert!(modified(&app));
    act(&app, "set_cell", json!({ "cell": "A1", "text": "1" })).unwrap();
    assert!(modified(&app), "typing it back is a second edit");
    act(&app, "undo", json!({})).unwrap();
    act(&app, "undo", json!({})).unwrap();
    assert!(!modified(&app), "undone all the way back");
}

#[test]
fn a_file_changed_by_another_program_is_never_written_over() {
    let (app, path) = with_file("conflict", "mine\r\n");
    act(&app, "set_cell", json!({ "cell": "A1", "text": "edited" })).unwrap();
    std::fs::write(&path, "theirs\r\n").unwrap();
    // The save's own check finds it, before the watch has looked.
    let err = act(&app, "save_over", json!({})).unwrap_err();
    assert!(err.contains("changed on disk since it was opened"), "{err}");
    assert_eq!(file(&path), "theirs\r\n");
    assert!(modified(&app));
    // The window's Save, which has the picker for it, asks where to put the sheet instead.
    let elsewhere = path.with_file_name("kept.csv");
    let target = elsewhere.clone();
    super::picker_says(&app, move |_, _| crate::files::picker::Choice::Chosen(target.clone()));
    header(&app, "save");
    assert_eq!(file(&elsewhere), "edited\r\n", "Save keeps the sheet in a file of its own");
    assert_eq!(file(&path), "theirs\r\n", "and the other version is untouched");
    assert!(!modified(&app));
}

#[test]
fn a_replace_that_finds_another_programs_version_keeps_it() {
    let dir = super::scratch("replace-keeps");
    let target = dir.join("taken.csv");
    std::fs::write(&target, "somebody else's\r\n").unwrap();
    let app = started();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "mine" })).unwrap();
    act(&app, "save_over", json!({ "path": target.display().to_string() })).unwrap();
    assert_eq!(file(&target), "mine\r\n");
    let names = listing(&dir);
    assert_eq!(names, vec!["taken.csv".to_string()], "what was read just before the replace is what it replaced");
}

#[test]
fn save_as_a_second_file_moves_the_sheet_there_and_watches_it() {
    let (app, path) = with_file("moves", "1\r\n");
    act(&app, "set_cell", json!({ "cell": "A1", "text": "2" })).unwrap();
    let other = path.with_file_name("copy.csv");
    act(&app, "save", json!({ "path": other.display().to_string() })).unwrap();
    assert_eq!(super::path_of(&app), Some(other.clone()));
    assert_eq!(file(&path), "1\r\n", "the first file is as it was");
    let watched = app.watched();
    assert_eq!(watched.len(), 1);
    assert_eq!(watched[0].1, other);
    assert_eq!(shown(&app, "A1"), "2");
}

/// The words in backticks in `sentence`, each of which names an action.
fn named_actions(sentence: &str) -> Vec<String> {
    sentence.split('`').skip(1).step_by(2).map(|w| w.split_whitespace().next().unwrap_or_default().to_string()).collect()
}

#[test]
fn no_sentence_names_an_action_the_app_does_not_have() {
    use crate::files::rules::SheetRules;
    use yantrik_app::documents::files::FileRules as _;
    let app = started();
    let table: Vec<String> =
        app.describe()["actions"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap().to_string()).collect();
    let mut sentences: Vec<String> = Vec::new();
    // The file was moved: found again beside where it was, and not found at all.
    let (app, path) = with_file("sentences", "1\r\n");
    let moved = path.with_file_name("renamed.csv");
    std::fs::rename(&path, &moved).unwrap();
    sentences.push(act(&app, "save", json!({})).unwrap_err());
    std::fs::remove_file(&moved).unwrap();
    sentences.push(act(&app, "save", json!({})).unwrap_err());
    // The rest of the file layer's words, and the notices that say what to do next.
    let p = std::path::Path::new("/x/y.csv");
    sentences.extend([
        SheetRules.already_there(p),
        SheetRules.changed_on_disk(p),
        SheetRules.linked(p),
        SheetRules.read_only(p),
        SheetRules.write_into_moved(p),
        SheetRules.write_here_or_elsewhere(),
        crate::update::conflict_sentence("x.csv"),
    ]);
    for sentence in &sentences {
        assert!(!sentence.contains("overwrite") && !sentence.contains("save_as"), "{sentence}");
        assert!(!sentence.contains("   "), "a run of spaces (a lost line continuation?): {sentence}");
        for word in named_actions(sentence) {
            assert!(table.contains(&word), "`{word}` is not one of the actions {table:?}: {sentence}");
        }
    }
    assert!(sentences[0].contains("`save_over` with the path"), "{}", sentences[0]);
    assert!(sentences[1].contains("`save_over` writes it here again, or `save` with a path"), "{}", sentences[1]);
}
