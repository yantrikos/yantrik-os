//! Each action's answer and refusal, through the real dispatch. An answer is built from that call's
//! own work: the sheet worked out for its own edit, the file it wrote read back.

use serde_json::{json, Value};

use super::{act, active, file, listing, modified, notice, raw, scratch, settle, started, state, status};

#[test]
fn every_action_is_graded_and_replacing_a_file_is_sensitive() {
    let app = started();
    let published = app.describe()["actions"].clone();
    let grade = |n: &str| {
        published.as_array().unwrap().iter().find(|a| a["name"] == n).map(|a| a["permission"].as_str().unwrap().to_string())
    };
    for name in ["set_cell", "get_range", "open", "save", "save_over", "undo", "redo", "select"] {
        assert!(grade(name).is_some(), "{name} is missing");
    }
    assert_eq!(grade("get_range").unwrap(), "safe");
    assert_eq!(grade("set_cell").unwrap(), "standard");
    assert_eq!(grade("open").unwrap(), "standard");
    assert_eq!(grade("save").unwrap(), "standard");
    assert_eq!(grade("save_over").unwrap(), "sensitive");
}

#[test]
fn set_cell_answers_with_what_the_cell_now_holds() {
    let app = started();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "2" })).unwrap();
    // A3 is left blank: a blank is 0 in the range.
    act(&app, "set_cell", json!({ "cell": "A2", "text": "3" })).unwrap();
    let r = act(&app, "set_cell", json!({ "cell": "A4", "text": "=SUM(A1:A3)" })).unwrap();
    assert_eq!(r["value"], json!(5.0));
    assert_eq!(r["shows"], "5");
    assert_eq!(r["before"], "");
    assert_eq!(r["changed"], true);
    assert_eq!(r["dirty"], true);
    assert_eq!((r["rows"].clone(), r["cols"].clone()), (json!(50), json!(26)));
    assert_eq!(active(&app), "A4", "the person sees where the mind typed");

    let bad = act(&app, "set_cell", json!({ "cell": "B1", "text": "=A1/0" })).unwrap();
    assert_eq!(bad["shows"], "#DIV/0!");
    assert_eq!(bad["problem"], "B1 shows #DIV/0!: dividing by zero.");
    assert_eq!(status(&app), "B1 shows #DIV/0!: dividing by zero.", "and the status line says it");
    assert_eq!(notice(&app), "B1 shows #DIV/0!: dividing by zero.");

    let again = act(&app, "set_cell", json!({ "cell": "B1", "text": "=A1/0" })).unwrap();
    assert_eq!(again["changed"], false, "the same text again changes nothing");
    assert!(act(&app, "set_cell", json!({ "cell": "nope", "text": "1" })).unwrap_err().contains("not a cell"));
    let long = act(&app, "set_cell", json!({ "cell": "A1", "text": "x".repeat(32_769) })).unwrap_err();
    assert!(long.contains("longer than the 32,768 characters"), "{long}");
}

#[test]
fn set_cell_grows_the_sheet_when_the_cell_is_past_its_edge() {
    let app = started();
    let r = act(&app, "set_cell", json!({ "cell": "AB60", "text": "7" })).unwrap();
    assert_eq!((r["rows"].clone(), r["cols"].clone(), r["shows"].clone()), (json!(60), json!(28), json!("7")));
    assert_eq!(super::shown(&app, "AB60"), "7", "and the window grew its grid");
    assert_eq!(app.ui(|ui| slint::Model::row_count(&ui.get_rows())), 60);
}

#[test]
fn undo_and_redo_from_the_surface_move_the_sheet_and_the_selection() {
    let app = started();
    act(&app, "set_cell", json!({ "cell": "B2", "text": "10" })).unwrap();
    act(&app, "set_cell", json!({ "cell": "B3", "text": "=B2*2" })).unwrap();
    let u = act(&app, "undo", json!({})).unwrap();
    assert_eq!(u["cell"], "B3");
    assert_eq!(u["typed"], "");
    assert_eq!((u["undone"].clone(), u["can_undo"].clone()), (json!(true), json!(true)));
    let u = act(&app, "undo", json!({})).unwrap();
    assert_eq!(u["cell"], "B2");
    assert_eq!(u["dirty"], false, "back to the empty sheet it started as");
    assert!(act(&app, "undo", json!({})).unwrap_err().contains("nothing to undo"));
    let r = act(&app, "redo", json!({})).unwrap();
    assert_eq!((r["redone"].clone(), r["can_redo"].clone()), (json!(true), json!(true)));
    let r = act(&app, "redo", json!({})).unwrap();
    assert_eq!(r["value"], json!(20.0));
    assert!(act(&app, "redo", json!({})).unwrap_err().contains("nothing to redo"));
}

#[test]
fn select_answers_where_it_landed_and_refuses_what_is_not_on_the_sheet() {
    let app = started();
    act(&app, "set_cell", json!({ "cell": "C3", "text": "hello" })).unwrap();
    let s = act(&app, "select", json!({ "cell": "C3" })).unwrap();
    assert_eq!((s["cell"].clone(), s["typed"].clone(), s["shows"].clone()), (json!("C3"), json!("hello"), json!("hello")));
    assert_eq!(super::bar(&app), "hello", "the formula bar shows it");
    let outside = act(&app, "select", json!({ "cell": "AA1" })).unwrap_err();
    assert!(outside.contains("AA1 is outside the sheet, which is 50 rows × 26 columns"), "{outside}");
    assert!(act(&app, "select", json!({ "cell": "1A" })).unwrap_err().contains("not a cell"));
}

#[test]
fn get_range_answers_what_was_typed_and_what_each_works_out_to() {
    let app = started();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "2" })).unwrap();
    act(&app, "set_cell", json!({ "cell": "B1", "text": "=A1*3" })).unwrap();
    act(&app, "set_cell", json!({ "cell": "C1", "text": "=A1/0" })).unwrap();
    let r = act(&app, "get_range", json!({ "range": "A1:C2" })).unwrap();
    assert_eq!(r["range"], "A1:C2");
    assert_eq!((r["rows"].clone(), r["cols"].clone()), (json!(2), json!(3)));
    assert_eq!(r["typed"], json!([["2", "=A1*3", "=A1/0"], ["", "", ""]]));
    assert_eq!(r["values"][0][1], json!(6.0));
    assert_eq!(r["errors"][0]["cell"], "C1");
    assert_eq!(r["outside_sheet"], false);
    assert!(act(&app, "get_range", json!({ "range": "nonsense" })).unwrap_err().contains("not a range"));
}

#[test]
fn get_range_refuses_a_range_too_large_to_answer() {
    let app = started();
    let e = act(&app, "get_range", json!({ "range": "A1:Z1000" })).unwrap_err();
    assert!(e.contains("26000 cells"), "{e}");
    let r = act(&app, "get_range", json!({ "range": "Y49:AB52" })).unwrap();
    assert_eq!(r["outside_sheet"], true);
    assert_eq!(r["values"][3][3], Value::Null);
}

#[test]
fn save_never_replaces_and_save_over_does() {
    let app = started();
    let dir = scratch("grades");
    let path = dir.join("budget.csv");
    std::fs::write(&path, "old\r\n").unwrap();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "new" })).unwrap();

    let refused = act(&app, "save", json!({ "path": path })).unwrap_err();
    assert!(refused.contains("already there") && refused.contains("save_over"), "{refused}");
    assert_eq!(file(&path), "old\r\n", "nothing was written");
    assert!(modified(&app));

    let done = act(&app, "save_over", json!({ "path": path })).unwrap();
    assert_eq!(done["replaced"], true);
    assert_eq!(file(&path), "new\r\n");
    assert!(!modified(&app));

    // With no path, save means the sheet's own file, which is there now, so it is save_over's.
    act(&app, "set_cell", json!({ "cell": "A2", "text": "more" })).unwrap();
    assert!(act(&app, "save", json!({})).unwrap_err().contains("save_over"));
    assert_eq!(act(&app, "save_over", json!({})).unwrap()["replaced"], true);
    assert_eq!(file(&path), "new\r\nmore\r\n");
}

#[test]
fn save_answers_from_the_file_it_wrote() {
    let app = started();
    let dir = scratch("save-answer");
    act(&app, "set_cell", json!({ "cell": "B2", "text": "x" })).unwrap();
    let path = dir.join("x.csv");
    let saved = act(&app, "save", json!({ "path": path })).unwrap();
    assert_eq!(saved["path"], json!(path.display().to_string()));
    assert_eq!((saved["rows"].clone(), saved["cols"].clone()), (json!(2), json!(2)));
    assert_eq!(saved["bytes"].as_u64().unwrap() as usize, std::fs::read(&path).unwrap().len());
    assert_eq!((saved["replaced"].clone(), saved["dirty"].clone()), (json!(false), json!(false)));
    assert_eq!(status(&app), format!("Saved {} — 2 rows × 2 columns, 7 bytes.", path.display()));
}

#[test]
fn save_refuses_what_is_not_a_csv_path_and_leaves_no_temporary_file() {
    let app = started();
    let dir = scratch("paths");
    assert!(act(&app, "save", json!({})).unwrap_err().contains("no file yet"));
    assert!(act(&app, "save", json!({ "path": "relative.csv" })).unwrap_err().contains("not an absolute path"));
    assert!(act(&app, "save", json!({ "path": dir.join("notes.txt").display().to_string() })).unwrap_err().contains(".csv"));
    assert!(act(&app, "save", json!({ "path": dir.join("nowhere/x.csv").display().to_string() })).unwrap_err().contains("does not exist"));
    act(&app, "save", json!({ "path": dir.join("x.csv").display().to_string() })).unwrap();
    assert_eq!(listing(&dir), vec!["x.csv".to_string()]);
    assert_eq!(act(&app, "open", json!({ "path": " " })).unwrap_err(), "No path was given.");
}

#[test]
fn open_refuses_while_there_are_unsaved_changes() {
    let app = started();
    let dir = scratch("dirty");
    let other = dir.join("other.csv");
    std::fs::write(&other, "1,2\n").unwrap();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "mine" })).unwrap();
    let e = act(&app, "open", json!({ "path": other.display().to_string() })).unwrap_err();
    assert!(e.contains("unsaved changes"), "{e}");
    assert_eq!(raw(&app, "A1"), "mine");
    act(&app, "undo", json!({})).unwrap();
    assert!(act(&app, "open", json!({ "path": other.display().to_string() })).is_ok(), "undone back to clean, it opens");
    assert_eq!(raw(&app, "B1"), "2");
}

#[test]
fn open_answers_what_it_read() {
    let (app, path) = super::with_file("open-answer", "a,b\r\n=1/0,3\r\n");
    let s = state(&app);
    assert_eq!(s["sheet"], "budget.csv");
    assert_eq!(s["path"], json!(path.display().to_string()));
    assert_eq!((s["filled_cells"].clone(), s["error_cells"].clone(), s["first_error"].clone()), (json!(4), json!(1), json!("A2")));
    assert_eq!(status(&app), "Opened budget.csv — 2 rows × 2 columns. 1 cells show an error; the first is A2.");
    let again = act(&app, "open", json!({ "path": path.display().to_string() })).unwrap();
    assert_eq!(
        again,
        json!({
            "path": path.display().to_string(), "file_rows": 2, "file_cols": 2, "rows": 50, "cols": 26,
            "filled_cells": 4, "error_cells": 1, "dirty": false,
        })
    );
}

#[test]
fn a_surface_call_while_a_file_is_read_is_refused_as_busy() {
    let (app, path) = super::with_file("busy", "1\r\n");
    let reading = app.act("open", json!({ "path": path.display().to_string() })).unwrap();
    let err = app.act("set_cell", json!({ "cell": "A1", "text": "2" })).unwrap_err();
    assert!(err.contains("cannot run while ySheets is reading or writing a file"), "{err}");
    settle(&app);
    reading.settle().unwrap();
    assert_eq!(raw(&app, "A1"), "1", "the refused call changed nothing");
}

#[test]
fn describe_says_the_path_and_that_nothing_is_unsaved_after_a_save() {
    let app = started();
    let dir = scratch("describe-save");
    act(&app, "set_cell", json!({ "cell": "C2", "text": "=1/0" })).unwrap();
    let before = state(&app);
    assert_eq!(before["path"], Value::Null);
    assert_eq!(before["dirty"], true);
    assert!(before["notice"].as_str().unwrap().contains("dividing by zero"));
    let path = dir.join("d.csv");
    act(&app, "save", json!({ "path": path.display().to_string() })).unwrap();
    let after = state(&app);
    assert_eq!(after["dirty"], false);
    assert_eq!(after["path"], json!(path.display().to_string()));
    assert_eq!(after["sheet"], "d.csv");
    assert_eq!(after["notice"], "", "a success clears the last failure");
}
