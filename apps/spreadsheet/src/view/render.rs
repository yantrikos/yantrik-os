//! The only code that writes the window.
//!
//! The grid's models are kept between draws and only the cells that changed are written, so an
//! edit in a large sheet repaints one cell rather than rebuilding every row the ListView holds.
//! The formula bar is given the selected cell's text only when the command asked for it
//! (`Cue::LoadField`): it is bound both ways, and setting it while the person types would
//! overwrite what they have typed. Everything else is derived from the model each time and
//! written only where it differs, so a sheet at rest draws nothing.
//!
//! The sheet is untrusted: a cell's text reaches the window as a plain string in a `Text`, which
//! draws it and interprets nothing. Drawn for a Mind (`Model::for_mind`), every cell's contents
//! and the formula bar's are the framework's one sentence instead (mind_view.rs).

use slint::{Model as _, ModelRc, SharedString, VecModel};
use yantrik_app::app::Changed;
use yantrik_app::for_mind::{shown_if_any, HIDDEN};

use crate::address::col_name;
use crate::formula::Value;
use crate::model::{Cue, Dialog, Model, Pending, Redraw};
use crate::{SheetDialog, SheetRow, SpreadsheetApp, SpreadsheetCell};

/// The cells of the grid. Drawn after a command that changed some.
pub const GRID: &str = "grid";
/// Everything else: the bar, the status line, the dialogs. Drawn after every command.
pub const CHROME: &str = "chrome";

pub fn render(ui: &SpreadsheetApp, m: &Model, changed: &Changed) {
    if changed.has(GRID) {
        grid(ui, m);
    }
    if changed.has(CHROME) {
        chrome(ui, m);
    }
    for cue in &m.cues {
        match cue {
            // Typing goes into the selected cell from the first key, without a click first.
            Cue::EditCell => ui.invoke_edit_cell(),
            Cue::LoadField => ui.set_cell_data(m.field_text().into()),
            Cue::CommitField => ui.invoke_command("commit".into()),
        }
    }
}

/// What a window drawn for a Mind hides. An agent is told nothing of the shape of a sheet it may not
/// read (`describe`'s hidden view), so a Mind View window of such a sheet hides the shape as well:
/// every cell is the mask, empty ones too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mask {
    /// A normal window.
    Off,
    /// A Mind View window: what holds something is the mask, an empty cell stays empty.
    Filled,
    /// A Mind View window of a sheet an agent is not shown: every cell is the mask.
    All,
}

impl Mask {
    pub fn of(for_mind: bool, hidden: bool) -> Mask {
        match (for_mind, hidden) {
            (false, _) => Mask::Off,
            (true, false) => Mask::Filled,
            (true, true) => Mask::All,
        }
    }
}

/// What a cell shows. A line break in a cell is drawn as a mark, since a row is one line high.
pub fn shown(m: &Model, value: &Value) -> SpreadsheetCell {
    shown_for(m.mask(), value)
}

/// [`shown`] without a model, for the job that builds a large sheet's cells off the window's thread.
pub fn shown_for(mask: Mask, value: &Value) -> SpreadsheetCell {
    let text = marked(value.display());
    match mask {
        Mask::Off => {}
        Mask::Filled => {
            return SpreadsheetCell { text: shown_if_any(true, &text).as_ref().into(), is_number: false, is_error: false }
        }
        Mask::All => return SpreadsheetCell { text: HIDDEN.into(), is_number: false, is_error: false },
    }
    SpreadsheetCell {
        text: SharedString::from(text),
        is_number: matches!(value, Value::Number(_)),
        is_error: value.error().is_some(),
    }
}

/// A line break in a cell is drawn as a mark. Almost no cell has one, so almost none is copied.
fn marked(text: String) -> String {
    if text.contains(['\n', '\r']) {
        text.replace("\r\n", " ↵ ").replace(['\n', '\r'], " ↵ ")
    } else {
        text
    }
}

/// Every cell of `sheet` as the window shows it, row by row. A job does this for a sheet just read,
/// so opening a large file leaves the window's thread only the models to hang them on.
pub fn rows_of(mask: Mask, sheet: &crate::sheet::Sheet) -> Vec<Vec<SpreadsheetCell>> {
    let (cols, values) = (sheet.cols(), sheet.values());
    (0..sheet.rows()).map(|r| (0..cols).map(|c| shown_for(mask, &values[r * cols + c])).collect()).collect()
}

fn row_cells(m: &Model, r: usize) -> Vec<SpreadsheetCell> {
    let (cols, values) = (m.sheet().cols(), m.sheet().values());
    (0..cols).map(|c| shown(m, &values[r * cols + c])).collect()
}

fn grid(ui: &SpreadsheetApp, m: &Model) {
    let (rows, cols) = (m.sheet().rows(), m.sheet().cols());
    let shown_rows = ui.get_rows();
    let list = shown_rows.as_any().downcast_ref::<VecModel<SheetRow>>();
    let same_shape = list.is_some_and(|l| {
        l.row_count() == rows && l.row_data(0).is_some_and(|row| row.cells.row_count() == cols)
    });
    let cells = match (&m.redraw, same_shape) {
        (Redraw::Cells(cells), true) => cells,
        _ => return whole(ui, m),
    };
    for &at in cells {
        let (r, c) = (at / cols, at % cols);
        let Some(row) = list.and_then(|l| l.row_data(r)) else { continue };
        let Some(line) = row.cells.as_any().downcast_ref::<VecModel<SpreadsheetCell>>() else { continue };
        let next = shown(m, &m.sheet().values()[at]);
        if line.row_data(c).as_ref() != Some(&next) {
            line.set_row_data(c, next);
        }
    }
}

/// Every row built again: a new sheet, a new size, or a result that changed too much to say where.
fn whole(ui: &SpreadsheetApp, m: &Model) {
    let (rows, cols) = (m.sheet().rows(), m.sheet().cols());
    let built = m.prebuilt.borrow_mut().take().filter(|b| b.rev == m.sheet().rev());
    let lines: Vec<SheetRow> = match built {
        Some(b) => b.rows.into_iter().map(|cells| SheetRow { cells: ModelRc::new(VecModel::from(cells)) }).collect(),
        None => (0..rows).map(|r| SheetRow { cells: ModelRc::new(VecModel::from(row_cells(m, r))) }).collect(),
    };
    ui.set_rows(ModelRc::new(VecModel::from(lines)));
    let names: Vec<SharedString> = (0..cols).map(|c| col_name(c).into()).collect();
    ui.set_col_names(ModelRc::new(VecModel::from(names)));
}

fn dialog_kind(m: &Model) -> SheetDialog {
    match m.dialog {
        Dialog::OpenPath => SheetDialog::OpenPath,
        Dialog::SavePath => SheetDialog::SavePath,
        Dialog::None if m.asking() => SheetDialog::Unsaved,
        Dialog::None => SheetDialog::None,
    }
}

fn chrome(ui: &SpreadsheetApp, m: &Model) {
    ui.set_file_name(m.title().into());
    ui.set_modified(m.modified());
    // A Mind's window of a sheet an agent is not shown says nothing of where the selection is or
    // what can be undone, as `describe` says nothing of either.
    let shape_hidden = m.mask() == Mask::All;
    ui.set_active_row(if shape_hidden { -1 } else { m.active.row as i32 });
    ui.set_active_col(if shape_hidden { -1 } else { m.active.col as i32 });
    ui.set_active_ref(if shape_hidden { "".into() } else { m.active.name().into() });
    let history = &m.book().history;
    ui.set_can_undo(!shape_hidden && history.can_undo());
    ui.set_can_redo(!shape_hidden && history.can_redo());
    let status = m.status();
    if shape_hidden || (m.for_mind && status.quotes_cells) {
        ui.set_status_text(HIDDEN.into());
        ui.set_status_is_problem(false);
    } else {
        ui.set_status_text(status.text.into());
        ui.set_status_is_problem(status.problem);
    }

    let kind = dialog_kind(m);
    // The path field is given its text when the dialog opens, not at every draw: the person types
    // into it.
    if ui.get_dialog() != kind && m.dialog != Dialog::None {
        ui.set_dialog_path(m.dialog_path.clone().into());
    }
    ui.set_dialog_message(dialog_message(m).into());
    ui.set_dialog(kind);
}

fn dialog_message(m: &Model) -> String {
    match (m.dialog, m.pending) {
        (Dialog::None, Some(pending)) => {
            let what = match pending {
                Pending::Open => "opening another file",
                Pending::Close => "closing",
            };
            format!("{} has changes that are not saved. Save them before {what}?", m.title())
        }
        _ => m.dialog_message.clone(),
    }
}
