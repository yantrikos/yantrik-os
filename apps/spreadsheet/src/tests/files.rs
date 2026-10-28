//! Opening and saving CSV files on real files, read back off the disk: quotes, line breaks, the
//! byte-order mark, formulas kept as typed, and a file that cannot be read costing nothing.

use serde_json::json;

use super::{act, file, notice, raw, scratch, started, state};

#[test]
fn a_csv_with_quotes_line_breaks_cr_and_a_bom_opens_and_saves_back_the_same() {
    let app = started();
    let dir = scratch("roundtrip");
    let source = dir.join("in.csv");
    std::fs::write(
        &source,
        "\u{feff}name,note,amount\r\n\"Smith, Jo\",\"two\nlines\",3\r\n\"cr\",\"a\rb\",4\r\n,,=SUM(C2:C3)\r\n",
    )
    .unwrap();

    let opened = act(&app, "open", json!({ "path": source.display().to_string() })).unwrap();
    assert_eq!((opened["file_rows"].as_u64(), opened["file_cols"].as_u64()), (Some(4), Some(3)));
    assert_eq!((opened["rows"].as_u64(), opened["cols"].as_u64()), (Some(50), Some(26)), "a short file still fills the grid");
    let range = act(&app, "get_range", json!({ "range": "A1:C4" })).unwrap();
    assert_eq!(range["typed"][1], json!(["Smith, Jo", "two\nlines", "3"]));
    assert_eq!(range["typed"][2][1], "a\rb");
    assert_eq!(range["typed"][0][0], "name", "the byte-order mark is not part of the first cell");
    assert_eq!(range["values"][3][2], json!(7.0), "the formula in the file is worked out");

    let copy = dir.join("out.csv");
    let saved = act(&app, "save", json!({ "path": copy.display().to_string() })).unwrap();
    assert_eq!(saved["replaced"], false);
    let again = started();
    act(&again, "open", json!({ "path": copy.display().to_string() })).unwrap();
    for cell in ["A2", "B2", "B3", "C4", "A1"] {
        assert_eq!(raw(&again, cell), raw(&app, cell), "{cell} came back different");
    }
    assert_eq!(saved["bytes"].as_u64().unwrap() as usize, std::fs::read(&copy).unwrap().len());
    assert!(file(&copy).contains("=SUM(C2:C3)"), "formulas are saved as typed");
}

#[test]
fn a_formula_like_cell_of_an_opened_file_is_saved_back_unchanged() {
    // `=HYPERLINK(...)` and `=cmd|...` stay what they were: ySheets writes what was typed.
    let (app, path) = super::with_file("formula-text", "\"=HYPERLINK(\"\"http://x\"\",\"\"y\"\")\",=cmd|' /C calc'!A0\r\n");
    act(&app, "set_cell", json!({ "cell": "A2", "text": "plain" })).unwrap();
    act(&app, "save_over", json!({})).unwrap();
    assert_eq!(file(&path), "\"=HYPERLINK(\"\"http://x\"\",\"\"y\"\")\",=cmd|' /C calc'!A0\r\nplain,\r\n");
}

#[test]
fn a_file_that_cannot_be_read_changes_nothing_and_says_why() {
    let app = started();
    let dir = scratch("unreadable");
    act(&app, "set_cell", json!({ "cell": "A1", "text": "keep" })).unwrap();
    act(&app, "save", json!({ "path": dir.join("keep.csv").display().to_string() })).unwrap();

    let missing = act(&app, "open", json!({ "path": dir.join("missing.csv").display().to_string() })).unwrap_err();
    assert!(missing.contains("there is no such file"), "{missing}");
    std::fs::write(dir.join("bad.csv"), "a,\"never closed\n").unwrap();
    let bad = act(&app, "open", json!({ "path": dir.join("bad.csv").display().to_string() })).unwrap_err();
    assert!(bad.contains("a closing \" is missing"), "{bad}");
    std::fs::write(dir.join("latin1.csv"), [b'a', 0xe9, b'\n']).unwrap();
    let latin = act(&app, "open", json!({ "path": dir.join("latin1.csv").display().to_string() })).unwrap_err();
    assert!(latin.contains("not UTF-8"), "{latin}");

    assert_eq!(raw(&app, "A1"), "keep");
    assert_eq!(notice(&app), latin, "the last failure is the notice");
    assert_eq!(state(&app)["notice"], json!(latin));
    assert!(latin.starts_with(&dir.join("latin1.csv").display().to_string()), "a refusal names the file first: {latin}");
}

#[test]
fn a_sheet_is_saved_whole_or_not_at_all_and_no_temporary_is_left() {
    let app = started();
    let dir = scratch("whole");
    act(&app, "set_cell", json!({ "cell": "A1", "text": "1" })).unwrap();
    let path = dir.join("a.csv");
    act(&app, "save", json!({ "path": path.display().to_string() })).unwrap();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "2" })).unwrap();
    act(&app, "save_over", json!({})).unwrap();
    assert_eq!(file(&path), "2\r\n");
    assert_eq!(super::listing(&dir), vec!["a.csv".to_string()], "no temporary file beside it");
}

#[test]
fn a_file_bigger_than_the_sheet_holds_is_refused_with_its_size_and_costs_nothing() {
    let app = started();
    let dir = scratch("big");
    // Two columns by 300,000 rows is 600,000 cells.
    let text = "1,2\n".repeat(300_000);
    let path = dir.join("big.csv");
    std::fs::write(&path, text).unwrap();
    let e = act(&app, "open", json!({ "path": path.display().to_string() })).unwrap_err();
    assert!(e.contains("500000 cells"), "{e}");
    assert_eq!(state(&app)["rows"], 50, "the sheet in front is untouched");
}
