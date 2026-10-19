//! Files on disk, the control surface, and the window's promise that nothing on it is dead.

use std::path::PathBuf;

use serde_json::{json, Value as Json};

use crate::session::Session;
use crate::surface;

/// A directory of this test's own, removed when it is dropped.
pub(crate) struct Scratch(PathBuf);

impl Scratch {
    pub(crate) fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ysheets-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn act(s: &mut Session, name: &str, args: Json) -> Result<Json, String> {
    let (_, handler) = surface::actions()
        .into_iter()
        .find(|(spec, _)| spec.schema()["name"] == name)
        .unwrap_or_else(|| panic!("no action {name}"));
    surface::run(s, name, handler, &args)
}

// ── CSV files ───────────────────────────────────────────────────────────

#[test]
fn a_csv_with_quotes_line_breaks_cr_and_a_bom_opens_and_saves_back_the_same() {
    let dir = Scratch::new("roundtrip");
    let source = dir.path("in.csv");
    std::fs::write(
        &source,
        "\u{feff}name,note,amount\r\n\"Smith, Jo\",\"two\nlines\",3\r\n\"cr\",\"a\rb\",4\r\n,,=SUM(C2:C3)\r\n",
    )
    .unwrap();

    let mut s = Session::new();
    let opened = act(&mut s, "open", json!({ "path": source })).unwrap();
    assert_eq!((opened["file_rows"].as_u64(), opened["file_cols"].as_u64()), (Some(4), Some(3)));
    assert_eq!((opened["rows"].as_u64(), opened["cols"].as_u64()), (Some(50), Some(26)), "a short file still fills the grid");
    let range = act(&mut s, "get_range", json!({ "range": "A1:C4" })).unwrap();
    assert_eq!(range["typed"][1], json!(["Smith, Jo", "two\nlines", "3"]));
    assert_eq!(range["typed"][2][1], "a\rb");
    assert_eq!(range["typed"][0][0], "name", "the byte-order mark is not part of the first cell");
    assert_eq!(range["values"][3][2], json!(7.0), "the formula in the file is worked out");

    let copy = dir.path("out.csv");
    let saved = act(&mut s, "save", json!({ "path": copy })).unwrap();
    assert_eq!(saved["replaced"], false);
    let mut again = Session::new();
    act(&mut again, "open", json!({ "path": copy })).unwrap();
    for cell in ["A2", "B2", "B3", "C4"] {
        let c = crate::address::parse_cell(cell).unwrap();
        assert_eq!(again.doc.sheet.raw(c), s.doc.sheet.raw(c), "{cell} came back different");
    }
    assert_eq!(saved["bytes"].as_u64().unwrap() as usize, std::fs::read(&copy).unwrap().len());
}

#[test]
fn a_file_that_cannot_be_read_changes_nothing_and_says_why() {
    let dir = Scratch::new("unreadable");
    let mut s = Session::new();
    act(&mut s, "set_cell", json!({ "cell": "A1", "text": "keep" })).unwrap();
    act(&mut s, "save", json!({ "path": dir.path("keep.csv") })).unwrap();

    let missing = act(&mut s, "open", json!({ "path": dir.path("missing.csv") })).unwrap_err();
    assert!(missing.contains("there is no such file"), "{missing}");
    std::fs::write(dir.path("bad.csv"), "a,\"never closed\n").unwrap();
    let bad = act(&mut s, "open", json!({ "path": dir.path("bad.csv") })).unwrap_err();
    assert!(bad.contains("a closing \" is missing"), "{bad}");
    std::fs::write(dir.path("latin1.csv"), [b'a', 0xe9, b'\n']).unwrap();
    let latin = act(&mut s, "open", json!({ "path": dir.path("latin1.csv") })).unwrap_err();
    assert!(latin.contains("not UTF-8"), "{latin}");

    assert_eq!(s.doc.sheet.raw(crate::address::Cell::new(0, 0)), "keep");
    assert_eq!(s.notice, latin, "the last failure is the notice");
}

// ── The surface ─────────────────────────────────────────────────────────

#[test]
fn every_action_is_graded_and_replacing_a_file_is_sensitive() {
    let grades: Vec<(String, String)> = surface::actions()
        .iter()
        .map(|(a, _)| {
            let s = a.schema();
            (s["name"].as_str().unwrap().to_string(), s["permission"].as_str().unwrap().to_string())
        })
        .collect();
    let grade = |n: &str| grades.iter().find(|(a, _)| a == n).map(|(_, g)| g.as_str()).unwrap();
    for name in ["set_cell", "get_range", "open", "save", "save_over", "undo", "redo", "select"] {
        assert!(grades.iter().any(|(a, _)| a == name), "{name} is missing");
    }
    assert_eq!(grade("get_range"), "safe");
    assert_eq!(grade("set_cell"), "standard");
    assert_eq!(grade("open"), "standard");
    assert_eq!(grade("save"), "standard");
    assert_eq!(grade("save_over"), "sensitive");
}

#[test]
fn set_cell_answers_with_what_the_cell_now_holds() {
    let mut s = Session::new();
    act(&mut s, "set_cell", json!({ "cell": "A1", "text": "2" })).unwrap();
    // A3 is left blank: a blank is 0 in the range.
    act(&mut s, "set_cell", json!({ "cell": "A2", "text": "3" })).unwrap();
    let r = act(&mut s, "set_cell", json!({ "cell": "A4", "text": "=SUM(A1:A3)" })).unwrap();
    assert_eq!(r["value"], json!(5.0));
    assert_eq!(r["shows"], "5");
    assert_eq!(r["before"], "");
    assert_eq!(r["dirty"], true);
    assert_eq!(s.active.name(), "A4", "the person sees where the mind typed");

    let bad = act(&mut s, "set_cell", json!({ "cell": "B1", "text": "=A1/0" })).unwrap();
    assert_eq!(bad["shows"], "#DIV/0!");
    assert_eq!(bad["problem"], "B1 shows #DIV/0!: dividing by zero.");
    assert_eq!(s.status(), ("B1 shows #DIV/0!: dividing by zero.".to_string(), true), "and the status line says it");

    assert!(act(&mut s, "set_cell", json!({ "cell": "nope", "text": "1" })).unwrap_err().contains("not a cell"));
}

#[test]
fn undo_and_redo_from_the_surface_move_the_sheet_and_the_selection() {
    let mut s = Session::new();
    act(&mut s, "set_cell", json!({ "cell": "B2", "text": "10" })).unwrap();
    act(&mut s, "set_cell", json!({ "cell": "B3", "text": "=B2*2" })).unwrap();
    let u = act(&mut s, "undo", json!({})).unwrap();
    assert_eq!(u["cell"], "B3");
    assert_eq!(u["typed"], "");
    let u = act(&mut s, "undo", json!({})).unwrap();
    assert_eq!(u["cell"], "B2");
    assert_eq!(u["dirty"], false, "back to the empty sheet it started as");
    assert!(act(&mut s, "undo", json!({})).unwrap_err().contains("nothing to undo"));
    act(&mut s, "redo", json!({})).unwrap();
    let r = act(&mut s, "redo", json!({})).unwrap();
    assert_eq!(r["value"], json!(20.0));
}

#[test]
fn save_never_replaces_and_save_over_does() {
    let dir = Scratch::new("grades");
    let path = dir.path("budget.csv");
    std::fs::write(&path, "old\r\n").unwrap();
    let mut s = Session::new();
    act(&mut s, "set_cell", json!({ "cell": "A1", "text": "new" })).unwrap();

    let refused = act(&mut s, "save", json!({ "path": path })).unwrap_err();
    assert!(refused.contains("already there") && refused.contains("save_over"), "{refused}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\r\n", "nothing was written");
    assert!(s.doc.dirty());

    let done = act(&mut s, "save_over", json!({ "path": path })).unwrap();
    assert_eq!(done["replaced"], true);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\r\n");
    assert!(!s.doc.dirty());

    // With no path, save means the sheet's own file — which is there now, so it is save_over's.
    act(&mut s, "set_cell", json!({ "cell": "A2", "text": "more" })).unwrap();
    assert!(act(&mut s, "save", json!({})).unwrap_err().contains("save_over"));
    assert_eq!(act(&mut s, "save_over", json!({})).unwrap()["replaced"], true);
}

#[test]
fn save_refuses_what_is_not_a_csv_path_and_leaves_no_temporary_file() {
    let dir = Scratch::new("paths");
    let mut s = Session::new();
    assert!(act(&mut s, "save", json!({})).unwrap_err().contains("no file yet"));
    assert!(act(&mut s, "save", json!({ "path": "relative.csv" })).unwrap_err().contains("not an absolute path"));
    assert!(act(&mut s, "save", json!({ "path": dir.path("notes.txt") })).unwrap_err().contains(".csv"));
    assert!(act(&mut s, "save", json!({ "path": dir.path("nowhere/x.csv") })).unwrap_err().contains("does not exist"));
    act(&mut s, "save", json!({ "path": dir.path("x.csv") })).unwrap();
    let left: Vec<_> = std::fs::read_dir(&dir.0).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, vec![std::ffi::OsString::from("x.csv")]);
}

#[test]
fn open_refuses_while_there_are_unsaved_changes() {
    let dir = Scratch::new("dirty");
    let other = dir.path("other.csv");
    std::fs::write(&other, "1,2\n").unwrap();
    let mut s = Session::new();
    act(&mut s, "set_cell", json!({ "cell": "A1", "text": "mine" })).unwrap();
    let e = act(&mut s, "open", json!({ "path": other })).unwrap_err();
    assert!(e.contains("unsaved changes"), "{e}");
    assert_eq!(s.doc.sheet.raw(crate::address::Cell::new(0, 0)), "mine");
    act(&mut s, "undo", json!({})).unwrap();
    assert!(act(&mut s, "open", json!({ "path": other })).is_ok(), "undone back to clean, it opens");
}

#[test]
fn describe_says_sheet_size_selection_dirty_and_path() {
    let dir = Scratch::new("describe");
    let mut s = Session::new();
    act(&mut s, "set_cell", json!({ "cell": "C2", "text": "=1/0" })).unwrap();
    let view = surface::describe(&s);
    let state = yantrik_app_runtime::control::describe_json("spreadsheet", &view, &[])["state"].clone();
    assert_eq!(state["rows"], 50);
    assert_eq!(state["cols"], 26);
    assert_eq!(state["selection"]["cell"], "C2");
    assert_eq!(state["selection"]["value"]["error"], "#DIV/0!");
    assert_eq!(state["dirty"], true);
    assert_eq!(state["path"], Json::Null);
    assert_eq!(state["error_cells"], 1);
    assert_eq!(state["first_error"], "C2");
    assert_eq!(state["used"], "A1:C2");
    assert!(state["notice"].as_str().unwrap().contains("dividing by zero"));

    act(&mut s, "save", json!({ "path": dir.path("d.csv") })).unwrap();
    let state = yantrik_app_runtime::control::describe_json("spreadsheet", &surface::describe(&s), &[])["state"].clone();
    assert_eq!(state["dirty"], false);
    assert_eq!(state["path"], dir.path("d.csv").display().to_string());
    assert_eq!(state["notice"], "");
}

#[test]
fn get_range_refuses_a_range_too_large_to_answer() {
    let mut s = Session::new();
    let e = act(&mut s, "get_range", json!({ "range": "A1:Z1000" })).unwrap_err();
    assert!(e.contains("26000 cells"), "{e}");
    let r = act(&mut s, "get_range", json!({ "range": "Y49:AB52" })).unwrap();
    assert_eq!(r["outside_sheet"], true);
    assert_eq!(r["values"][3][3], Json::Null);
}

// ── The window ──────────────────────────────────────────────────────────

/// Source text with comments removed, so a word in a comment cannot satisfy a check.
fn code(path: &str) -> String {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap();
    text.lines().map(|l| l.split("//").next().unwrap_or_default()).collect::<Vec<_>>().join("\n")
}

/// Contract item 7, and the two diseases of design/apps-plan-2026-09-20.md: every `in` property
/// of the window is written from Rust, and every callback is connected to something that is not
/// a log line.
#[test]
fn every_property_is_set_and_every_callback_does_something() {
    let window = code("ui/app.slint");
    let rust: String = ["src/render.rs", "src/files.rs", "src/wire.rs", "src/main.rs"].iter().map(|p| code(p)).collect();
    for line in window.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("in property <").or_else(|| line.strip_prefix("in-out property <")) {
            let name = rest.split('>').nth(1).unwrap().split_whitespace().next().unwrap().trim_end_matches(';');
            let setter = format!("set_{}(", name.replace('-', "_"));
            assert!(rust.contains(&setter), "`{name}` is never set from Rust ({setter})");
        }
        if let Some(rest) = line.strip_prefix("callback ") {
            let name = rest.split([' ', '(', ';']).next().unwrap();
            let handler = format!("on_{}(", name.replace('-', "_"));
            assert!(rust.contains(&handler), "`{name}` is never connected ({handler})");
        }
    }
    assert!(!rust.contains("tracing::info!(\"Toggle") && !rust.contains("say_unavailable"), "no log-only or not-built-yet handlers");
}

/// Every command the header can send is one `wire` handles.
#[test]
fn every_header_button_is_a_command_the_app_handles() {
    let screen = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/yantrik-ui-slint/ui/spreadsheet.slint"),
    )
    .unwrap();
    let wire = code("src/wire.rs");
    let mut ids: Vec<&str> = screen.match_indices("{ id: \"").map(|(i, _)| screen[i + 7..].split('"').next().unwrap()).collect();
    ids.extend(screen.match_indices("root.command(\"").map(|(i, _)| screen[i + 14..].split('"').next().unwrap()));
    assert!(ids.len() >= 5, "found {ids:?}");
    for id in ids {
        assert!(wire.contains(&format!("\"{id}\" =>")), "the header sends `{id}` and nothing handles it");
    }
}
