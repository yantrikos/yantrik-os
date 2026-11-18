//! Every answer reports what was observed: the path, the bytes, the count that changed. Each of
//! these used to answer `{"accepted": true}`.

use serde_json::{json, Value};

use super::{act, file, notice, none, scratch, started, state, summary, tabs, text};

#[test]
fn the_editor_answers_with_what_it_wrote() {
    let app = started();
    let dir = scratch("answers");
    let fresh = act(&app, "new", none()).expect("a new tab");
    assert_eq!(fresh["title"], "Untitled", "{fresh}");
    assert_eq!(fresh["path"], Value::Null, "{fresh}");
    assert_eq!(fresh["modified"], false, "{fresh}");

    let written = act(&app, "set_content", json!({ "text": "alpha\nbeta\nalpha\n" })).unwrap();
    assert_eq!(written["lines"], 4, "{written}");
    assert_eq!(written["modified"], true, "{written}");
    assert_eq!(written["on_disk"], false, "nothing has been written yet: {written}");

    let found = act(&app, "find", json!({ "text": "alpha" })).unwrap();
    assert_eq!(found["matches"], 2, "{found}");
    act(&app, "replace_text", json!({ "text": "omega" })).unwrap();
    let replaced = act(&app, "replace-all", none()).unwrap();
    assert_eq!(replaced["replaced"], 2, "the answer counts what it changed: {replaced}");
    assert_eq!(text(&app), "omega\nbeta\nomega\n");

    // A caller that can rewrite a whole tab has to be able to put it back.
    let undone = act(&app, "undo", none()).unwrap();
    assert_eq!(undone["modified"], true, "{undone}");
    assert_eq!(text(&app), "alpha\nbeta\nalpha\n");
    act(&app, "redo", none()).unwrap();
    assert_eq!(text(&app), "omega\nbeta\nomega\n");

    let refused = act(&app, "save", none()).expect_err("a tab with no file");
    assert!(refused.contains("save_as"), "the refusal names the action that would work: {refused}");

    let path = dir.join("surface.txt");
    let saved = act(&app, "save_as", json!({ "path": path.display().to_string() })).unwrap();
    assert_eq!(saved["path"], path.display().to_string(), "{saved}");
    assert_eq!(saved["matches_disk"], true, "{saved}");
    assert_eq!(saved["modified"], false, "{saved}");
    assert_eq!(file(&path), "omega\nbeta\nomega\n");
    let s = summary(&app);
    assert!(s.contains("surface.txt") && s.contains("saved") && s.contains("tab "), "{s}");

    // `save` writes the tab back and reads the bytes on disk.
    act(&app, "append", json!({ "text": "tail\n" })).unwrap();
    let saved = act(&app, "save", none()).unwrap();
    assert_eq!(saved["saved"], true, "{saved}");
    assert_eq!(saved["bytes"], 22, "{saved}");
    assert_eq!(saved["lines"], 5, "{saved}");
    assert_eq!(saved["now"]["matches_disk"], true, "{saved}");

    // #86, on the surface: a file already at the path is refused, the refusal names the way
    // through it, and nothing is overwritten until the caller says so.
    let occupied = dir.join("occupied.txt");
    std::fs::write(&occupied, "somebody else's\n").unwrap();
    let refused = act(&app, "save_as", json!({ "path": occupied.display().to_string() })).unwrap_err();
    assert!(refused.contains("already exists") && refused.contains("overwrite=true"), "{refused}");
    assert_eq!(file(&occupied), "somebody else's\n", "a refused save_as changes nothing on disk");
    let replaced = act(&app, "save_as", json!({ "path": occupied.display().to_string(), "overwrite": true })).unwrap();
    assert_eq!(replaced["matches_disk"], true, "{replaced}");
    assert_eq!(file(&occupied), "omega\nbeta\nomega\ntail\n");
    assert!(!dir.join("occupied (outside version).txt").exists(), "the file the caller agreed to replace goes");

    // A refusal names what it could not do, and reaches the window's notice.
    let missing = dir.join("not-here.txt");
    let failed = act(&app, "open", json!({ "path": missing.display().to_string() })).unwrap_err();
    assert!(!failed.is_empty() && !notice(&app).is_empty());
    let out = act(&app, "select_tab", json!({ "index": 99 })).unwrap_err();
    assert!(out.contains("99") && out.contains("open"), "{out}");
}

#[test]
fn a_blank_required_argument_is_refused_by_name_and_said_in_the_window() {
    let app = started();
    let before = text(&app);
    let refused = act(&app, "open", json!({ "path": "  " })).unwrap_err();
    assert!(refused.contains("`open` was given an empty `path`"), "{refused}");
    assert!(notice(&app).contains("path"), "{}", notice(&app));
    let refused = act(&app, "find", json!({ "text": "" })).unwrap_err();
    assert!(refused.contains("`find` was given an empty `text`"), "{refused}");
    assert_eq!(text(&app), before, "a refused call changes nothing");
    // A missing one is the dispatch's, before the editor is asked.
    assert!(act(&app, "set_content", none()).is_err());
}

/// A Writer's way into the Editor (#253): `append` is `standard`, and `new` with `text` then
/// `save_as` is the one-call route to a document with text in it.
#[test]
fn append_adds_to_the_end_and_takes_nothing_away() {
    let app = started();
    let before = text(&app);
    assert_eq!(act(&app, "append", json!({ "text": "\nMinutes\n" })).unwrap()["modified"], true);
    act(&app, "append", json!({ "text": "- budget agreed\n" })).unwrap();
    assert_eq!(text(&app), format!("{before}\nMinutes\n- budget agreed\n"));
    act(&app, "undo", none()).unwrap();
    assert_eq!(text(&app), format!("{before}\nMinutes\n"), "undo takes one append back off");
    let refused = act(&app, "append", json!({ "text": "" })).unwrap_err();
    assert!(refused.contains("text"), "{refused}");
    assert_eq!(text(&app), format!("{before}\nMinutes\n"));
    let n = tabs(&app);
    let fresh = act(&app, "new", json!({ "text": "Agenda\n- one\n" })).unwrap();
    assert_eq!(text(&app), "Agenda\n- one\n");
    assert_eq!(fresh["modified"], true, "{fresh}");
    assert_eq!(tabs(&app), n + 1, "in a tab of its own");
    let nothing = act(&app, "redo", none()).unwrap_err();
    assert_eq!(nothing, "There is nothing left to redo in this tab; it is unchanged.");
}

#[test]
fn a_call_while_a_dialog_asks_is_refused_with_the_answer_it_wants() {
    let app = started();
    act(&app, "set_content", json!({ "text": "draft" })).unwrap();
    act(&app, "close", none()).unwrap();
    let refused = act(&app, "append", json!({ "text": "x" })).unwrap_err();
    assert!(refused.contains("asking about unsaved changes (Untitled has unsaved changes.)"), "{refused}");
    assert!(refused.contains("`discard` throws the changes away"), "{refused}");
    assert_eq!(act(&app, "cancel", none()).unwrap()["was_asking"], 3);
    assert_eq!(state(&app)["dialog"], 0);
    let nothing = act(&app, "cancel", none()).unwrap_err();
    assert_eq!(nothing, "No dialog is open, so there is nothing to cancel.");
    let refused = act(&app, "discard", none()).unwrap_err();
    assert!(refused.starts_with("Nothing is waiting to be discarded"), "{refused}");
}

#[test]
fn a_call_while_a_file_is_written_is_refused_and_changes_nothing() {
    let (app, path) = super::with_file("busy", "one\n");
    super::typed(&app, "two\n");
    app.run(crate::command::Command::Window(crate::command::WindowAction::Save));
    assert_eq!(app.jobs_waiting(), 1, "the save is in flight");
    let refused = app.act("set_content", json!({ "text": "x" })).unwrap_err();
    assert_eq!(
        refused,
        "`set_content` cannot run while the editor is reading or writing a file; read `describe` again in a moment."
    );
    super::settle(&app);
    assert_eq!(file(&path), "two\n");
    assert_eq!(text(&app), "two\n");
}

#[test]
fn show_answers_with_the_tab_in_front() {
    let app = started();
    let shown = act(&app, "show", none()).unwrap();
    assert_eq!(shown["shown"], true);
    assert_eq!(shown["now"]["title"], "Untitled");
}
