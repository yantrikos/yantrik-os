//! The real window's callbacks, invoked as the screen invokes them: typing into the formula bar,
//! Enter and the arrows, clicking cells, the header's buttons, and the promise that nothing on the
//! window is dead.

use slint::Model as _;

use super::{
    active, bar, click, enter, header, modified, notice, raw, settle, shown, started, status, submit, type_in_bar,
};

#[test]
fn a_person_fills_the_grid_types_sums_undoes_and_the_sheet_follows() {
    let app = started();
    // (1) The grid is filled at start: 50 rows of 26 cells, and 26 column names.
    assert_eq!(app.ui(|ui| ui.get_rows().row_count()), 50);
    assert_eq!(app.ui(|ui| ui.get_rows().row_data(49).unwrap().cells.row_count()), 26);
    assert_eq!(app.ui(|ui| ui.get_col_names().row_count()), 26);
    assert_eq!(app.ui(|ui| ui.get_col_names().row_data(25).unwrap().to_string()), "Z");

    // Numbers, a blank, and a SUM over the blank.
    click(&app, "A1");
    assert_eq!(app.ui(|ui| ui.get_active_ref().to_string()), "A1");
    for text in ["2", "3", "", "=SUM(A1:A3)"] {
        enter(&app, text);
    }
    assert_eq!(shown(&app, "A1"), "2");
    assert_eq!(shown(&app, "A2"), "3");
    assert_eq!(shown(&app, "A3"), "");
    assert_eq!(shown(&app, "A4"), "5", "SUM over a blank works it out as 0");
    assert_eq!(active(&app), "A5", "Enter moves down");
    assert!(modified(&app) && app.ui(|ui| ui.get_modified()));

    // Recalculation: change A1 and the sum follows.
    click(&app, "A1");
    assert_eq!(bar(&app), "2", "the formula bar shows what A1 holds");
    enter(&app, "10");
    assert_eq!(shown(&app, "A4"), "13");

    // An error keeps its code in the cell, and the status line says what went wrong.
    click(&app, "B1");
    enter(&app, "hello");
    enter(&app, "=B1+1");
    assert_eq!(shown(&app, "B2"), "#VALUE!");
    assert!(app.ui(|ui| ui.get_status_is_problem()));
    assert_eq!(status(&app), "B2 shows #VALUE!: B1 holds text (\"hello\"), so it can't be added.");
    assert_eq!(active(&app), "B2", "a cell that went wrong stays selected");
    assert_eq!(bar(&app), "=B1+1", "and the bar shows it again");

    // Undo takes B2 back; redo puts it again; the buttons follow.
    header(&app, "undo");
    assert_eq!(shown(&app, "B2"), "");
    assert!(app.ui(|ui| ui.get_can_redo()));
    header(&app, "redo");
    assert_eq!(shown(&app, "B2"), "#VALUE!");
    header(&app, "undo");
    header(&app, "undo");
    assert_eq!(shown(&app, "B1"), "", "B1's hello is undone too");
    click(&app, "A1");
    header(&app, "undo");
    assert_eq!(shown(&app, "A1"), "2", "A1 is back to 2");
    assert_eq!(shown(&app, "A4"), "5");
    assert_eq!(bar(&app), "2", "after a header command the bar shows the selected cell");
}

#[test]
fn keys_typed_before_the_sheet_is_worked_out_still_go_to_the_right_cells() {
    let app = started();
    // Three Enters in a row with no job run between them, as fast typing is.
    for (text, _) in [("1", 0), ("=A1+1", 1), ("=A2+1", 2)] {
        let text = text.to_string();
        app.ui(move |ui| {
            ui.set_cell_data(text.clone().into());
            ui.invoke_formula_submitted(text.into(), "down".into());
        });
    }
    assert_eq!(active(&app), "A4");
    settle(&app);
    assert_eq!((shown(&app, "A1"), shown(&app, "A2"), shown(&app, "A3")), ("1".into(), "2".into(), "3".into()));
    assert_eq!(active(&app), "A4", "nothing went wrong, so nothing came back");
}

#[test]
fn a_cell_that_went_wrong_does_not_pull_the_selection_back_from_where_the_person_went() {
    let app = started();
    app.ui(|ui| {
        ui.set_cell_data("=1/0".into());
        ui.invoke_formula_submitted("=1/0".into(), "down".into());
        // Moved on before the sheet was worked out.
        ui.invoke_cell_clicked(5, 5);
    });
    settle(&app);
    assert_eq!(active(&app), "F6");
    assert_eq!(shown(&app, "A1"), "#DIV/0!");
    assert_eq!(notice(&app), "A1 shows #DIV/0!: dividing by zero.", "it is still said");
}

#[test]
fn clicking_away_keeps_what_was_typed_and_loads_the_next_cell() {
    let app = started();
    type_in_bar(&app, "typed but not entered");
    click(&app, "C3");
    assert_eq!(raw(&app, "A1"), "typed but not entered");
    assert_eq!(active(&app), "C3");
    assert_eq!(bar(&app), "");
}

#[test]
fn escape_puts_the_bar_back_to_what_the_cell_holds() {
    let app = started();
    enter(&app, "kept");
    click(&app, "A1");
    type_in_bar(&app, "half a thought");
    app.ui(|ui| ui.invoke_formula_cancelled());
    settle(&app);
    assert_eq!(bar(&app), "kept");
    assert_eq!(raw(&app, "A1"), "kept");
}

#[test]
fn the_arrows_move_and_the_bottom_edge_grows_the_sheet() {
    let app = started();
    submit(&app, "", "right");
    assert_eq!(active(&app), "B1");
    submit(&app, "", "left");
    submit(&app, "", "left");
    assert_eq!(active(&app), "A1", "the left edge stops there");
    submit(&app, "", "up");
    assert_eq!(active(&app), "A1");
    click(&app, "A50");
    submit(&app, "last", "down");
    assert_eq!(active(&app), "A51", "Enter on the last row keeps going");
    assert_eq!(app.ui(|ui| ui.get_rows().row_count()), 51);
    assert_eq!(app.model(|m| m.sheet().rows()), 51);
    assert_eq!(shown(&app, "A50"), "last");
}

#[test]
fn a_header_command_the_window_does_not_know_says_so() {
    let app = started();
    header(&app, "frobnicate");
    assert_eq!(status(&app), "The window asked for `frobnicate`, which this app does not have.");
    assert!(app.ui(|ui| ui.get_status_is_problem()));
}

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
    let rust: String = ["src/view/render.rs", "src/view/mod.rs"].iter().map(|p| code(p)).collect();
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

/// Every command the header can send is one the window handles.
#[test]
fn every_header_button_is_a_command_the_app_handles() {
    let screen = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/yantrik-ui-slint/ui/spreadsheet.slint"),
    )
    .unwrap();
    let handled = code("src/update/window.rs");
    let mut ids: Vec<&str> = screen.match_indices("{ id: \"").map(|(i, _)| screen[i + 7..].split('"').next().unwrap()).collect();
    ids.extend(screen.match_indices("root.command(\"").map(|(i, _)| screen[i + 14..].split('"').next().unwrap()));
    assert!(ids.len() >= 5, "found {ids:?}");
    for id in ids {
        assert!(handled.contains(&format!("\"{id}\" =>")), "the header sends `{id}` and nothing handles it");
    }
}
