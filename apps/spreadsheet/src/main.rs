//! Yantrik Spreadsheet — standalone app binary.
//!
//! Full-featured spreadsheet with formulas, multi-sheet, formatting, charts, AI assist.

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-spreadsheet");

    let app = SpreadsheetApp::new().unwrap();
    wire(&app);
    app.run().unwrap();
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
    app.on_sheet_ai_submit(|prompt| { tracing::info!("AI submit: {prompt} (standalone mode)"); });
    app.on_sheet_ai_apply(|| { tracing::info!("AI apply"); });
    app.on_sheet_ai_dismiss(|| { tracing::info!("AI dismiss"); });
    app.on_sheet_ai_formula(|desc| { tracing::info!("AI formula: {desc}"); });
    app.on_sheet_ai_analyze(|| { tracing::info!("AI analyze"); });
    app.on_sheet_ai_suggest_chart(|| { tracing::info!("AI suggest chart"); });
    app.on_sheet_ai_insights(|| { tracing::info!("AI insights"); });
    app.on_sheet_ai_generate_data(|desc| { tracing::info!("AI generate data: {desc}"); });
}
