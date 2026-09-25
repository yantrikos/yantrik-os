//! Yantrik Spreadsheet — standalone app binary.
//!
//! Full-featured spreadsheet with formulas, multi-sheet, formatting, charts, AI assist.

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-spreadsheet");

    // One window per app: a second launch defers to the running one (the shell focuses it).
    let Some(_instance) = instance::claim("spreadsheet") else { return };

    let app = SpreadsheetApp::new().unwrap();
    // The window's title bar is the app's own (#256).
    yantrik_app_runtime::window_chrome!(app);

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    wire(&app);
    run_until_closed(&app, "yantrik-spreadsheet");
}

fn wire(app: &SpreadsheetApp) {
    // ── Cell interaction ──
    {
        let weak = app.as_weak();
        app.on_cell_clicked(move |row, col| {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_active_row(row);
            ui.set_active_col(col);
            // Read cell data from grid
            let grid = ui.get_cell_grid();
            let cols = ui.get_col_count();
            let idx = (row * cols + col) as usize;
            if idx < grid.row_count() {
                if let Some(cell) = grid.row_data(idx) {
                    ui.set_cell_data(cell.text);
                }
            }
        });
    }

    {
        let weak = app.as_weak();
        app.on_cell_edited(move |row, col, text| {
            let Some(ui) = weak.upgrade() else { return };
            let grid = ui.get_cell_grid();
            let cols = ui.get_col_count();
            let idx = (row * cols + col) as usize;
            if idx < grid.row_count() {
                if let Some(mut cell) = grid.row_data(idx) {
                    cell.text = text;
                    cell.is_formula = cell.text.as_str().starts_with('=');
                    grid.set_row_data(idx, cell);
                }
            }
        });
    }

    // ── Formula bar ──
    {
        let weak = app.as_weak();
        app.on_formula_submitted(move |formula| {
            let Some(ui) = weak.upgrade() else { return };
            let row = ui.get_active_row();
            let col = ui.get_active_col();
            let grid = ui.get_cell_grid();
            let cols = ui.get_col_count();
            let idx = (row * cols + col) as usize;
            if idx < grid.row_count() {
                if let Some(mut cell) = grid.row_data(idx) {
                    cell.text = formula.clone();
                    cell.is_formula = formula.as_str().starts_with('=');
                    grid.set_row_data(idx, cell);
                }
            }
            ui.set_cell_data(formula);
        });
    }

    // ── Sheet tabs ──
    app.on_switch_sheet(|idx| { tracing::info!("Switch to sheet {idx}"); });
    app.on_add_sheet(|| { tracing::info!("Add new sheet"); });

    // ── Formatting ──
    app.on_format_bold(|| { tracing::info!("Toggle bold"); });
    app.on_format_italic(|| { tracing::info!("Toggle italic"); });
    app.on_format_align(|align| { tracing::info!("Set alignment: {align}"); });
    app.on_format_number(|fmt| { tracing::info!("Set number format: {fmt}"); });
    app.on_set_bg_color(|idx| { tracing::info!("Set bg color: {idx}"); });
    app.on_set_text_color(|idx| { tracing::info!("Set text color: {idx}"); });

    // ── Import / Export ──
    app.on_import_csv(|| { tracing::info!("Import CSV"); });
    app.on_export_csv(|| { tracing::info!("Export CSV"); });
    app.on_save_sheet(|| { tracing::info!("Save sheet"); });
    app.on_load_sheet(|| { tracing::info!("Load sheet"); });

    // ── Sort / Filter ──
    app.on_sort_column(|col, asc| { tracing::info!("Sort column {col}, ascending={asc}"); });
    app.on_filter_column(|col, text| { tracing::info!("Filter column {col}: {text}"); });
    app.on_clear_filter(|| { tracing::info!("Clear filter"); });

    // ── Row / Column operations ──
    app.on_insert_row(|after| { tracing::info!("Insert row after {after}"); });
    app.on_delete_row(|row| { tracing::info!("Delete row {row}"); });
    app.on_insert_col(|after| { tracing::info!("Insert col after {after}"); });
    app.on_delete_col(|col| { tracing::info!("Delete col {col}"); });

    // ── Find / Replace ──
    app.on_find_text(|q| { tracing::info!("Find: {q}"); });
    app.on_find_next(|| { tracing::info!("Find next"); });
    app.on_replace_one(|find, rep| { tracing::info!("Replace '{find}' with '{rep}'"); });
    app.on_replace_all(|find, rep| { tracing::info!("Replace all '{find}' with '{rep}'"); });

    // ── Clipboard ──
    app.on_copy_cell(|| { tracing::info!("Copy cell"); });
    app.on_paste_cell(|| { tracing::info!("Paste cell"); });
    app.on_cut_cell(|| { tracing::info!("Cut cell"); });

    // ── Undo / Redo ──
    app.on_undo(|| { tracing::info!("Undo"); });
    app.on_redo(|| { tracing::info!("Redo"); });

    // ── Keyboard ──
    app.on_key_pressed(|key| { tracing::info!("Key pressed: {key}"); });

    // ── Comments ──
    app.on_add_comment(|text| { tracing::info!("Add comment: {text}"); });
    app.on_delete_comment(|| { tracing::info!("Delete comment"); });

    // ── Merge / Freeze ──
    app.on_merge_cells(|r1, c1, r2, c2| { tracing::info!("Merge cells ({r1},{c1})->({r2},{c2})"); });
    app.on_unmerge_cells(|| { tracing::info!("Unmerge cells"); });
    app.on_freeze_panes(|rows, cols| { tracing::info!("Freeze panes: {rows} rows, {cols} cols"); });

    // ── Charts ──
    app.on_create_chart(|chart_type, range| { tracing::info!("Create chart type={chart_type}, range={range}"); });

    // ── Formula help ──
    app.on_sheet_show_formula_help(|| { tracing::info!("Show formula help"); });

    // ── AI assist ──
    //
    // Shelved: the panel and its buttons are in the markup, but nothing behind them asks
    // anything — every handler here wrote a log line and returned, so pressing Analyze opened
    // a panel that stayed empty forever, which reads as a request in flight. Until this app
    // grows real AI actions, the buttons say that plainly, in the panel they open.
    app.on_sheet_ai_submit({
        let weak = app.as_weak();
        move |_prompt| {
            if let Some(ui) = weak.upgrade() { say_unavailable(&ui); }
        }
    });
    app.on_sheet_ai_apply({
        let weak = app.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_sheet_ai_response("There is nothing to apply.".into());
            }
        }
    });
    app.on_sheet_ai_dismiss({
        let weak = app.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() { ui.set_sheet_ai_response("".into()); }
        }
    });
    app.on_sheet_ai_formula({
        let weak = app.as_weak();
        move |_desc| {
            if let Some(ui) = weak.upgrade() { say_unavailable(&ui); }
        }
    });
    app.on_sheet_ai_analyze({
        let weak = app.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() { say_unavailable(&ui); }
        }
    });
    app.on_sheet_ai_suggest_chart({
        let weak = app.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() { say_unavailable(&ui); }
        }
    });
    app.on_sheet_ai_insights({
        let weak = app.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() { say_unavailable(&ui); }
        }
    });
    app.on_sheet_ai_generate_data({
        let weak = app.as_weak();
        move |_desc| {
            if let Some(ui) = weak.upgrade() { say_unavailable(&ui); }
        }
    });
}

/// The one sentence every shelved AI button in this app says, so six ways of being unfinished
/// do not become six different excuses.
fn say_unavailable(ui: &SpreadsheetApp) {
    ui.set_sheet_ai_response(companion::NOT_BUILT_YET.into());
}

#[cfg(test)]
mod ai_stub_tests {
    /// Everything above the first test module: the wiring assertions read this, so test code
    /// mentioning the same names cannot satisfy them.
    fn main_source() -> String {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs");
        let src = std::fs::read_to_string(path).expect("this file");
        src.split("#[cfg(test)]").next().unwrap_or_default().to_string()
    }

    /// The defect: every AI handler wrote a log line and returned, so the button opened a panel
    /// that stayed empty forever. Each one must now say so in the panel itself.
    #[test]
    fn every_ai_button_says_it_is_not_available_yet() {
        let src = main_source();
        for name in [
            "on_sheet_ai_submit",
            "on_sheet_ai_formula",
            "on_sheet_ai_analyze",
            "on_sheet_ai_suggest_chart",
            "on_sheet_ai_insights",
            "on_sheet_ai_generate_data",
        ] {
            let start = src.find(name).unwrap_or_else(|| panic!("{name} is not wired at all"));
            let body = &src[start..];
            let end = body.find("});").map(|e| e + 3).unwrap_or(body.len());
            assert!(
                body[..end].contains("say_unavailable"),
                "{name} must put the not-available-yet sentence in the panel"
            );
        }
        assert!(src.contains("on_sheet_ai_dismiss"), "dismiss must be wired to clear the panel");
        assert!(
            src.contains("companion::NOT_BUILT_YET"),
            "the sentence is the shared one, not this app's own wording"
        );
    }
}
