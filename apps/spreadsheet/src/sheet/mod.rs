//! One sheet: a `rows × cols` grid of what was typed, and what each cell works out to.
//!
//! Every cell exists. The first engine kept the grid as whatever the last import produced, so a
//! short file left fewer cells than the window drew and edits to the missing ones vanished
//! (review of #727, finding 4). Here the text and the values are always exactly `rows × cols`,
//! and anything that changes the size goes through [`Sheet::resize`].

mod recalc;
#[cfg(test)]
mod tests;

use crate::address::{Cell, Range, MAX_ROWS};
use crate::formula::Value;

/// The size a new sheet opens at, and the least an opened file is given, so there is always
/// room to type beside what was read.
pub const DEFAULT_ROWS: usize = 50;
pub const DEFAULT_COLS: usize = 26;

/// The most cells one sheet holds, and the most columns. The window draws only the rows in view,
/// but every column of them, so columns are what drawing limits. A file larger than this is
/// refused with its size rather than half-read.
pub const MAX_CELLS: usize = 500_000;
pub const MAX_SHEET_COLS: usize = 256;

#[derive(Debug, Clone)]
pub struct Sheet {
    rows: usize,
    cols: usize,
    raw: Vec<String>,
    values: Vec<Value>,
}

impl Default for Sheet {
    fn default() -> Self {
        Self::new(DEFAULT_ROWS, DEFAULT_COLS)
    }
}

impl Sheet {
    pub fn new(rows: usize, cols: usize) -> Self {
        let mut s = Self { rows, cols, raw: vec![String::new(); rows * cols], values: Vec::new() };
        s.recalc();
        s
    }

    /// A sheet holding `rows` of text, at least the default size.
    pub fn from_rows(data: &[Vec<String>]) -> Result<Self, String> {
        let rows = data.len().max(DEFAULT_ROWS);
        let cols = data.iter().map(Vec::len).max().unwrap_or(0).max(DEFAULT_COLS);
        check_size(rows, cols)?;
        let mut raw = vec![String::new(); rows * cols];
        for (r, line) in data.iter().enumerate() {
            for (c, text) in line.iter().enumerate() {
                raw[r * cols + c] = text.clone();
            }
        }
        let mut s = Self { rows, cols, raw, values: Vec::new() };
        s.recalc();
        Ok(s)
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn contains(&self, cell: Cell) -> bool {
        cell.row < self.rows && cell.col < self.cols
    }

    /// What was typed into a cell; "" outside the grid.
    pub fn raw(&self, cell: Cell) -> &str {
        if self.contains(cell) { &self.raw[self.index(cell)] } else { "" }
    }

    /// What a cell works out to; blank outside the grid.
    pub fn value(&self, cell: Cell) -> Value {
        if self.contains(cell) { self.values[self.index(cell)].clone() } else { Value::Empty }
    }

    /// Put `text` in a cell, growing the grid when the cell is past its edge, and work everything
    /// out again. Returns what the cell held before.
    pub fn set(&mut self, cell: Cell, text: &str) -> Result<String, String> {
        if !self.contains(cell) {
            self.resize(self.rows.max(cell.row + 1), self.cols.max(cell.col + 1))?;
        }
        let i = self.index(cell);
        let old = std::mem::replace(&mut self.raw[i], text.to_string());
        self.recalc();
        Ok(old)
    }

    /// Change the size, keeping every cell that is still inside it.
    pub fn resize(&mut self, rows: usize, cols: usize) -> Result<(), String> {
        check_size(rows, cols)?;
        let mut raw = vec![String::new(); rows * cols];
        for r in 0..self.rows.min(rows) {
            for c in 0..self.cols.min(cols) {
                raw[r * cols + c] = std::mem::take(&mut self.raw[r * self.cols + c]);
            }
        }
        self.rows = rows;
        self.cols = cols;
        self.raw = raw;
        self.recalc();
        Ok(())
    }

    /// The smallest top-left block that holds every non-empty cell.
    pub fn used(&self) -> (usize, usize) {
        let (mut rows, mut cols) = (0, 0);
        for r in 0..self.rows {
            for c in 0..self.cols {
                if !self.raw[r * self.cols + c].is_empty() {
                    rows = rows.max(r + 1);
                    cols = cols.max(c + 1);
                }
            }
        }
        (rows, cols)
    }

    /// The typed text of the used block, row by row: what a file is written from.
    pub fn used_rows(&self) -> Vec<Vec<String>> {
        let (rows, cols) = self.used();
        (0..rows)
            .map(|r| (0..cols).map(|c| self.raw[r * self.cols + c].clone()).collect())
            .collect()
    }

    /// The values of a range, row by row, clipped to nothing: cells past the edge are blank.
    pub fn range_values(&self, range: &Range) -> Vec<Vec<Value>> {
        (range.first.row..=range.last.row)
            .map(|r| (range.first.col..=range.last.col).map(|c| self.value(Cell::new(r, c))).collect())
            .collect()
    }

    /// How many cells hold something, and how many of those show an error.
    pub fn counts(&self) -> (usize, usize) {
        let filled = self.raw.iter().filter(|t| !t.is_empty()).count();
        let errors = self.values.iter().filter(|v| v.error().is_some()).count();
        (filled, errors)
    }

    /// The first cell that shows an error, in reading order.
    pub fn first_error(&self) -> Option<Cell> {
        self.values.iter().position(|v| v.error().is_some()).map(|i| Cell::new(i / self.cols, i % self.cols))
    }

    /// What is wrong with a cell, in a sentence, if anything is.
    pub fn problem(&self, cell: Cell) -> Option<String> {
        let v = self.value(cell);
        let e = v.error()?;
        Some(format!("{} shows {}: {}.", cell.name(), e.code(), e.message))
    }

    fn index(&self, cell: Cell) -> usize {
        cell.row * self.cols + cell.col
    }

    fn recalc(&mut self) {
        self.values = recalc::recalc(self.rows, self.cols, &self.raw);
    }
}

fn check_size(rows: usize, cols: usize) -> Result<(), String> {
    if cols > MAX_SHEET_COLS {
        return Err(format!("{cols} columns is more than the {MAX_SHEET_COLS} one sheet holds"));
    }
    if rows > MAX_ROWS || rows.saturating_mul(cols) > MAX_CELLS {
        return Err(format!(
            "{rows} rows by {cols} columns is {} cells, more than the {MAX_CELLS} one sheet holds",
            rows.saturating_mul(cols)
        ));
    }
    Ok(())
}
