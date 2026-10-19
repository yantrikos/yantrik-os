//! The session, drawn: every `in` property of the window is written from here.
//!
//! The grid's models are kept between draws and only the cells that changed are written, so an
//! edit in a large sheet repaints one cell rather than rebuilding every row the ListView holds.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{Model, ModelRc, SharedString, VecModel};

use crate::address::{col_name, Cell};
use crate::formula::Value;
use crate::session::Session;
use crate::{SheetRow, SpreadsheetApp, SpreadsheetCell};

#[derive(Default)]
pub struct Grid {
    rows: Rc<VecModel<SheetRow>>,
    row_models: Vec<Rc<VecModel<SpreadsheetCell>>>,
    cols: usize,
    installed: bool,
}

thread_local! {
    static GRID: RefCell<Grid> = RefCell::new(Grid::default());
}

/// What a cell shows. A line break in a cell is drawn as a mark, since a row is one line high.
pub fn shown(value: &Value) -> SpreadsheetCell {
    SpreadsheetCell {
        text: SharedString::from(value.display().replace("\r\n", " ↵ ").replace(['\n', '\r'], " ↵ ")),
        is_number: matches!(value, Value::Number(_)),
        is_error: value.error().is_some(),
    }
}

pub fn render(ui: &SpreadsheetApp, s: &Session) {
    GRID.with(|g| g.borrow_mut().draw(ui, s));
    let doc = &s.doc;
    ui.set_file_name(doc.title().into());
    ui.set_modified(doc.dirty());
    ui.set_active_row(s.active.row as i32);
    ui.set_active_col(s.active.col as i32);
    ui.set_active_ref(s.active.name().into());
    ui.set_can_undo(doc.history.can_undo());
    ui.set_can_redo(doc.history.can_redo());
    let (status, problem) = s.status();
    ui.set_status_text(status.into());
    ui.set_status_is_problem(problem);
}

/// Put the selected cell's typed text in the formula bar. Separate from [`render`], because
/// rendering while the person is typing must not overwrite what they have typed.
pub fn load_formula_bar(ui: &SpreadsheetApp, s: &Session) {
    ui.set_cell_data(s.doc.sheet.raw(s.active).into());
}

impl Grid {
    fn draw(&mut self, ui: &SpreadsheetApp, s: &Session) {
        let sheet = &s.doc.sheet;
        let (rows, cols) = (sheet.rows(), sheet.cols());
        if !self.installed {
            ui.set_rows(ModelRc::from(self.rows.clone()));
            self.installed = true;
        }
        if cols != self.cols {
            let names: Vec<SharedString> = (0..cols).map(|c| col_name(c).into()).collect();
            ui.set_col_names(ModelRc::new(VecModel::from(names)));
            // Every row's width changed: start them again.
            self.row_models.clear();
            self.rows.set_vec(Vec::new());
            self.cols = cols;
        }
        // Rows past the end go; new ones are added below.
        while self.row_models.len() > rows {
            self.row_models.pop();
            self.rows.remove(self.rows.row_count() - 1);
        }
        for r in 0..rows {
            if r == self.row_models.len() {
                let cells: Vec<SpreadsheetCell> = (0..cols).map(|c| shown(&sheet.value(Cell::new(r, c)))).collect();
                let model = Rc::new(VecModel::from(cells));
                self.rows.push(SheetRow { cells: ModelRc::from(model.clone()) });
                self.row_models.push(model);
                continue;
            }
            let model = &self.row_models[r];
            for c in 0..cols {
                let next = shown(&sheet.value(Cell::new(r, c)));
                if model.row_data(c).as_ref() != Some(&next) {
                    model.set_row_data(c, next);
                }
            }
        }
    }
}
