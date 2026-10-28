//! ySheets drawn for a Mind (Mind View): `read_mind_view` reads the window as text, so the window
//! holds no cell contents. The sheet's name, its size and the selection stay; the formula bar takes
//! no typing, and what was refused does not stay in it; nothing that would change the sheet, or
//! read or write a file as the person, runs; and looking never writes the mask over a cell.

use std::sync::Arc;

use serde_json::json;
use slint::Model as _;
use yantrik_app::for_mind::{HIDDEN, READ_ONLY};
use yantrik_app::mind::fake::FakeMind;
use yantrik_testing::TestApp;

use super::{act, click, listing, raw, scratch, settle, shown, started, status};
use crate::command::Command;
use crate::YSheets;

const SENTINEL: &str = "SENTINEL-ysheets-mind-view-3c9d";

/// A sheet with the sentinel in three cells and a formula quoting it, kept as a draft, and where it
/// is kept.
fn kept() -> yantrik_app::Paths {
    let app = started();
    act(&app, "set_cell", json!({ "cell": "A1", "text": format!("one {SENTINEL}") })).unwrap();
    act(&app, "set_cell", json!({ "cell": "B1", "text": SENTINEL })).unwrap();
    act(&app, "set_cell", json!({ "cell": "A2", "text": "=A1+1" })).unwrap();
    act(&app, "set_cell", json!({ "cell": "C3", "text": "12" })).unwrap();
    act(&app, "select", json!({ "cell": "A1" })).unwrap();
    super::flush_drafts(&app);
    let paths = app.paths();
    let _ = app.shutdown();
    paths
}

fn start_for_mind(paths: yantrik_app::Paths) -> TestApp<YSheets> {
    let app = TestApp::<YSheets>::start_for_mind(paths, Vec::new(), Arc::new(FakeMind::new()));
    settle(&app);
    app
}

/// Every string the window holds that a screen reader or OCR could see.
fn window_text(app: &TestApp<YSheets>) -> String {
    app.ui(|ui| {
        let mut all = vec![
            ui.get_cell_data().to_string(),
            ui.get_status_text().to_string(),
            ui.get_file_name().to_string(),
            ui.get_active_ref().to_string(),
            ui.get_dialog_message().to_string(),
            ui.get_dialog_path().to_string(),
        ];
        let rows = ui.get_rows();
        for r in 0..rows.row_count() {
            let cells = rows.row_data(r).unwrap().cells;
            all.extend((0..cells.row_count()).map(|c| cells.row_data(c).unwrap().text.to_string()));
        }
        all.join("\n")
    })
}

#[test]
fn a_mind_view_window_holds_no_cell_contents_but_keeps_the_sheets_name_and_shape() {
    let app = start_for_mind(kept());
    let text = window_text(&app);
    assert!(!text.contains(SENTINEL), "{text}");
    assert!(!text.contains("12"), "numbers are contents too: {text}");
    assert_eq!(shown(&app, "A1"), HIDDEN);
    assert_eq!(shown(&app, "C3"), HIDDEN);
    assert_eq!(shown(&app, "D4"), "", "an empty cell stays empty: a mask over nothing says there is something");
    assert_eq!(super::bar(&app), HIDDEN, "the formula bar holds the mask for a cell that holds something");
    assert_eq!(status(&app), "A1 · 50 rows × 26 columns · 4 cells filled · 1 shows an error, the first in A2");
    assert_eq!(app.ui(|ui| ui.get_file_name().to_string()), "Untitled");
    assert_eq!(raw(&app, "A1"), format!("one {SENTINEL}"), "the sheet itself is whole");
}

#[test]
fn a_normal_window_still_shows_the_cells() {
    let paths = kept();
    let app = TestApp::<YSheets>::start_at(paths, Vec::new());
    settle(&app);
    assert!(window_text(&app).contains(SENTINEL));
    assert_eq!(super::bar(&app), format!("one {SENTINEL}"));
}

#[test]
fn typing_into_a_mind_view_window_is_refused_and_what_was_typed_does_not_stay() {
    let app = start_for_mind(kept());
    let before = app.model(|m| m.sheet().used_rows());
    app.ui(|ui| {
        ui.set_cell_data("typed by someone".into());
        ui.invoke_formula_submitted("typed by someone".into(), "down".into());
    });
    settle(&app);
    assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY);
    assert_eq!(super::bar(&app), HIDDEN, "the refused text is not left in the field");
    assert!(status(&app).contains(READ_ONLY), "the window says why: {}", status(&app));
    assert_eq!(super::notice(&app), READ_ONLY);
    assert_eq!(app.model(|m| m.sheet().used_rows()), before);
    assert!(!super::modified(&app) || app.model(|m| m.doc.recovered), "nothing was typed into the sheet");
    assert_eq!(super::active(&app), "A1", "and the selection did not move on");
    // Clicking away with typing in the bar is refused the same way.
    app.ui(|ui| {
        ui.set_cell_data("again".into());
        ui.invoke_cell_clicked(4, 4);
    });
    settle(&app);
    assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY);
    assert_eq!(super::bar(&app), HIDDEN);
    assert_eq!(super::active(&app), "A1");
    assert!(!app.model(|m| m.sheet().used_rows().iter().flatten().any(|c| c.contains("typed by someone") || c == "again")));
}

#[test]
fn typing_into_an_empty_cell_is_refused_and_leaves_the_bar_empty() {
    let app = start_for_mind(kept());
    super::click(&app, "E5");
    assert_eq!(super::bar(&app), "");
    app.ui(|ui| {
        ui.set_cell_data("hello".into());
        ui.invoke_formula_submitted("hello".into(), "down".into());
    });
    settle(&app);
    assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY);
    assert_eq!(super::bar(&app), "", "nothing is hidden in an empty cell, so nothing is put in the bar");
    assert_eq!(raw(&app, "E5"), "");
}

#[test]
fn every_edit_and_file_button_in_a_mind_view_window_is_refused_before_anything_changes() {
    let app = start_for_mind(kept());
    let dir = scratch("mind-files");
    let before = app.model(|m| (m.sheet().used_rows(), m.book().history.can_undo()));
    for id in ["undo", "redo", "open", "save", "save-as"] {
        app.ui(move |ui| ui.invoke_command(id.into()));
        settle(&app);
        assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY, "{id}");
        assert_eq!(super::bar(&app), HIDDEN, "{id}");
    }
    app.run(Command::Launch(dir.join("elsewhere.csv")));
    assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY, "a launch's file is read as the person");
    app.run(Command::Confirm { path: dir.join("x.csv").display().to_string() });
    assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY);
    settle(&app);
    assert_eq!(app.model(|m| (m.sheet().used_rows(), m.book().history.can_undo())), before);
    assert!(listing(&dir).is_empty(), "no file was read, written or made");
    assert!(app.model(|m| !m.picking && m.busy.is_none()));
}

#[test]
fn looking_at_the_sheet_cell_by_cell_never_writes_the_mask_over_a_cell() {
    let app = start_for_mind(kept());
    for name in ["B1", "A2", "C3", "D4", "A1"] {
        click(&app, name);
        assert!(app.last_refusal().is_none_or(|r| r.sentence != READ_ONLY), "looking is not refused: {name}");
        assert_eq!(super::active(&app), name);
    }
    // Enter and the arrows on a bar nobody typed in are looking too.
    super::submit(&app, &super::bar(&app), "right");
    assert!(app.last_refusal().is_none_or(|r| r.sentence != READ_ONLY));
    assert_eq!(super::active(&app), "B1");
    super::header(&app, "frobnicate");
    assert_eq!(raw(&app, "A1"), format!("one {SENTINEL}"));
    assert_eq!(raw(&app, "B1"), SENTINEL);
    assert!(app.model(|m| !m.sheet().used_rows().iter().flatten().any(|c| c == HIDDEN)));
}

#[test]
fn what_a_cell_error_quotes_is_not_on_the_status_line_of_a_mind_view_window() {
    let app = start_for_mind(kept());
    click(&app, "A2");
    // A2 is `=A1+1`, and A1 is text: its error quotes A1.
    assert_eq!(status(&app), HIDDEN, "{}", status(&app));
    assert!(!window_text(&app).contains(SENTINEL));
}

#[test]
fn masking_is_visual_only_describe_and_calls_are_as_for_a_normal_instance() {
    let paths = kept();
    let what_a_caller_gets = |app: &TestApp<YSheets>| {
        let d = app.describe();
        let moved = act(app, "select", json!({ "cell": "B1" })).unwrap();
        let range = act(app, "get_range", json!({ "range": "A1:C3" })).unwrap();
        json!({ "summary": d["summary"], "state": d["state"], "moved": moved, "range": range })
    };
    let normal = TestApp::<YSheets>::start_at(paths.clone(), Vec::new());
    settle(&normal);
    let expected = what_a_caller_gets(&normal);
    let _ = normal.shutdown();
    drop(normal);
    let masked = start_for_mind(paths);
    let got = what_a_caller_gets(&masked);
    assert!(expected.to_string().contains(SENTINEL), "a Mind still reads the sheet through describe");
    assert_eq!(got["summary"], expected["summary"]);
    assert_eq!(got["state"]["selection"], expected["state"]["selection"]);
    assert_eq!(got["state"]["used"], expected["state"]["used"]);
    assert_eq!(got["moved"], expected["moved"]);
    assert_eq!(got["range"], expected["range"]);
    // And the intents still change the sheet, under the agent rules, for the Mind that drives it.
    act(&masked, "set_cell", json!({ "cell": "D1", "text": "by the Mind" })).unwrap();
    assert_eq!(raw(&masked, "D1"), "by the Mind");
    assert_eq!(shown(&masked, "D1"), HIDDEN, "and the window still shows no contents");
    assert_eq!(super::bar(&masked), HIDDEN);
}

#[test]
fn escape_in_a_mind_view_window_puts_the_mask_back_not_what_was_typed() {
    let app = start_for_mind(kept());
    super::type_in_bar(&app, "half a thought");
    app.ui(|ui| ui.invoke_formula_cancelled());
    settle(&app);
    assert_eq!(super::bar(&app), HIDDEN);
    assert!(app.last_refusal().is_none_or(|r| r.sentence != READ_ONLY), "escape is looking");
}

/// A sheet the person opened from a place an agent may not read, with filled cells and a selection,
/// kept as a draft; and where it is kept. `None` where the temporary folder is inside the home.
fn kept_hidden() -> Option<yantrik_app::Paths> {
    let app = started();
    let secret = scratch("mind-hidden").join("secret.csv");
    if secret.starts_with(std::env::var_os("HOME").unwrap_or_default()) {
        return None;
    }
    std::fs::write(&secret, format!("{SENTINEL},42\n")).unwrap();
    act(&app, "open", json!({ "path": secret.display().to_string() })).unwrap();
    act(&app, "set_cell", json!({ "cell": "D7", "text": "=1/0" })).unwrap();
    super::flush_drafts(&app);
    let paths = app.paths();
    let _ = app.shutdown();
    Some(paths)
}

#[test]
fn a_mind_view_window_of_a_sheet_an_agent_is_not_shown_hides_its_shape_as_describe_does() {
    let Some(paths) = kept_hidden() else { return };
    let app = start_for_mind(paths);
    assert!(app.model(|m| m.doc.hidden));
    // Every cell is the mask, the empty ones too: which cells hold something is not drawn.
    assert_eq!(shown(&app, "A1"), HIDDEN);
    assert_eq!(shown(&app, "D7"), HIDDEN);
    assert_eq!(shown(&app, "Z50"), HIDDEN);
    assert_eq!(super::bar(&app), HIDDEN);
    assert_eq!(status(&app), HIDDEN);
    assert_eq!(app.ui(|ui| ui.get_active_ref().to_string()), "");
    assert_eq!(app.ui(|ui| (ui.get_active_row(), ui.get_active_col())), (-1, -1));
    assert!(window_text(&app).lines().all(|l| l.is_empty() || l == HIDDEN || l == "secret.csv"), "{}", window_text(&app));
    assert!(!window_text(&app).contains(SENTINEL));
    // Looking still works, and typing is still refused.
    click(&app, "E5");
    assert_eq!(super::bar(&app), HIDDEN);
    app.ui(|ui| {
        ui.set_cell_data("typed".into());
        ui.invoke_formula_submitted("typed".into(), "down".into());
    });
    settle(&app);
    assert_eq!(app.last_refusal().unwrap().sentence, READ_ONLY);
    assert_eq!(super::bar(&app), HIDDEN);
    assert_eq!(raw(&app, "E5"), "");
}
