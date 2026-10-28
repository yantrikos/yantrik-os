//! One sheet: a `rows × cols` grid of what was typed, and what each cell works out to.
//!
//! Every cell exists. The first engine kept the grid as whatever the last import produced, so a
//! short file left fewer cells than the window drew and edits to the missing ones vanished
//! (review of #727, finding 4). Here the text and the values are always exactly `rows × cols`,
//! and anything that changes the size goes through [`Sheet::resize`].
//!
//! Typing into a cell does not work the sheet out. [`Sheet::put`] changes the text and the
//! revision, and shows a stand-in for the cell until the sheet is worked out again; that is a job
//! ([`Snapshot::solve`]), and its result is taken only if the sheet is still at the revision it
//! was made for ([`Sheet::install`]). [`Sheet::set`] is `put` and the work done in place, for
//! tests and for building a sheet from a file inside a job.

mod recalc;
mod solved;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use crate::address::{Cell, Range, MAX_ROWS};
use crate::formula::{self, Value};

pub use solved::{Snapshot, Solved};

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
    /// The text of every cell, at the sheet's revision.
    snap: Snapshot,
    /// What every cell works out to, as of `solved`; cells typed since show a stand-in.
    values: Arc<Vec<Value>>,
    /// The revision `values` were worked out for.
    solved: u64,
}

impl Default for Sheet {
    fn default() -> Self {
        Self::new(DEFAULT_ROWS, DEFAULT_COLS)
    }
}

impl Sheet {
    pub fn new(rows: usize, cols: usize) -> Self {
        let raw = (0..rows).map(|_| Arc::new(vec![String::new(); cols])).collect();
        Self::worked_out(Snapshot { rev: fresh_rev(), rows, cols, raw })
    }

    /// A sheet holding `rows` of text, at least the default size, worked out now.
    pub fn from_rows(data: &[Vec<String>]) -> Result<Self, String> {
        let rows = data.len().max(DEFAULT_ROWS);
        let cols = data.iter().map(Vec::len).max().unwrap_or(0).max(DEFAULT_COLS);
        check_size(rows, cols)?;
        let raw = (0..rows)
            .map(|r| {
                let mut line = vec![String::new(); cols];
                for (c, text) in data.get(r).into_iter().flatten().enumerate() {
                    line[c] = text.clone();
                }
                Arc::new(line)
            })
            .collect();
        Ok(Self::worked_out(Snapshot { rev: fresh_rev(), rows, cols, raw }))
    }

    fn worked_out(snap: Snapshot) -> Self {
        let solved = snap.clone().solve();
        Self { solved: snap.rev, snap, values: solved.values }
    }

    pub fn rows(&self) -> usize {
        self.snap.rows
    }

    pub fn cols(&self) -> usize {
        self.snap.cols
    }

    /// Changes whenever any cell's text or the size does.
    pub fn rev(&self) -> u64 {
        self.snap.rev
    }

    /// Whether every cell shows what it works out to, and no stand-in.
    pub fn is_solved(&self) -> bool {
        self.solved == self.snap.rev
    }

    pub fn contains(&self, cell: Cell) -> bool {
        self.snap.contains(cell)
    }

    /// What was typed into a cell; "" outside the grid.
    pub fn raw(&self, cell: Cell) -> &str {
        self.snap.raw(cell)
    }

    /// What a cell shows; blank outside the grid. Between a [`Sheet::put`] and the result of the
    /// job that works the sheet out, a cell typed since shows a stand-in.
    pub fn value(&self, cell: Cell) -> Value {
        if self.contains(cell) { self.values[self.index(cell)].clone() } else { Value::Empty }
    }

    /// Every cell's value, row by row, as last worked out or stood in for.
    pub fn values(&self) -> &Arc<Vec<Value>> {
        &self.values
    }

    /// The text at this moment, to be worked out off the window's thread.
    pub fn snapshot(&self) -> Snapshot {
        self.snap.clone()
    }

    /// The sheet as it shows, for answering a call from it. Only what the sheet is worked out
    /// to, when [`Sheet::is_solved`].
    pub fn view(&self) -> Solved {
        Solved { snap: self.snap.clone(), values: self.values.clone() }
    }

    /// Takes the result of a job that worked out revision `solved.rev()`, if the sheet is still
    /// at it. A result for an older revision is dropped; the answer says whether it was taken.
    pub fn install(&mut self, solved: &Solved) -> bool {
        let fits =
            solved.rev() == self.snap.rev && solved.rows() == self.snap.rows && solved.cols() == self.snap.cols;
        if fits {
            self.values = solved.values.clone();
            self.solved = self.snap.rev;
        }
        fits
    }

    /// Put `text` in a cell, growing the grid when the cell is past its edge. Returns what the
    /// cell held before. The sheet is not worked out: the cell shows a stand-in (what the text
    /// is if it is not a formula, what the cell showed before if it is) until [`Sheet::install`].
    pub fn put(&mut self, cell: Cell, text: &str) -> Result<String, String> {
        if !self.contains(cell) {
            self.resize(self.snap.rows.max(cell.row + 1), self.snap.cols.max(cell.col + 1))?;
        }
        let old = std::mem::replace(&mut Arc::make_mut(&mut self.snap.raw[cell.row])[cell.col], text.to_string());
        self.snap.rev += 1;
        if !formula::is_formula(text) {
            let at = self.index(cell);
            Arc::make_mut(&mut self.values)[at] = Value::from_entry(text);
        }
        Ok(old)
    }

    /// [`Sheet::put`], then the sheet worked out in place.
    pub fn set(&mut self, cell: Cell, text: &str) -> Result<String, String> {
        let old = self.put(cell, text)?;
        self.settle();
        Ok(old)
    }

    /// Works the sheet out in place. A job does this off the window's thread; this is for tests
    /// and for a sheet being built inside a job.
    pub fn settle(&mut self) {
        let solved = self.snap.clone().solve();
        self.values = solved.values;
        self.solved = self.snap.rev;
    }

    /// Change the size, keeping every cell that is still inside it. The values of the cells that
    /// stay are kept as they were until the next result is installed.
    pub fn resize(&mut self, rows: usize, cols: usize) -> Result<(), String> {
        check_size(rows, cols)?;
        let (old_rows, old_cols) = (self.snap.rows, self.snap.cols);
        if cols == old_cols {
            self.snap.raw.truncate(rows);
            while self.snap.raw.len() < rows {
                self.snap.raw.push(Arc::new(vec![String::new(); cols]));
            }
            let values = Arc::make_mut(&mut self.values);
            values.truncate(rows * cols);
            values.resize(rows * cols, Value::Empty);
        } else {
            let mut values = vec![Value::Empty; rows * cols];
            let kept = old_rows.min(rows);
            let mut raw = Vec::with_capacity(rows);
            for r in 0..rows {
                let mut line = vec![String::new(); cols];
                if r < kept {
                    for c in 0..old_cols.min(cols) {
                        line[c] = std::mem::take(&mut Arc::make_mut(&mut self.snap.raw[r])[c]);
                        values[r * cols + c] = self.values[r * old_cols + c].clone();
                    }
                }
                raw.push(Arc::new(line));
            }
            self.snap.raw = raw;
            self.values = Arc::new(values);
        }
        self.snap.rows = rows;
        self.snap.cols = cols;
        self.snap.rev += 1;
        Ok(())
    }

    /// The smallest top-left block that holds every non-empty cell.
    pub fn used(&self) -> (usize, usize) {
        self.snap.used()
    }

    /// The typed text of the used block, row by row: what a file is written from.
    pub fn used_rows(&self) -> Vec<Vec<String>> {
        self.snap.used_rows()
    }

    /// The values of a range, row by row, clipped to nothing: cells past the edge are blank.
    pub fn range_values(&self, range: &Range) -> Vec<Vec<Value>> {
        (range.first.row..=range.last.row)
            .map(|r| (range.first.col..=range.last.col).map(|c| self.value(Cell::new(r, c))).collect())
            .collect()
    }

    /// How many cells hold something, and how many of those show an error.
    pub fn counts(&self) -> (usize, usize) {
        (self.snap.filled(), self.values.iter().filter(|v| v.error().is_some()).count())
    }

    /// The first cell that shows an error, in reading order.
    pub fn first_error(&self) -> Option<Cell> {
        let cols = self.snap.cols.max(1);
        self.values.iter().position(|v| v.error().is_some()).map(|i| Cell::new(i / cols, i % cols))
    }

    /// What is wrong with a cell, in a sentence, if anything is.
    pub fn problem(&self, cell: Cell) -> Option<String> {
        let v = self.value(cell);
        let e = v.error()?;
        Some(format!("{} shows {}: {}.", cell.name(), e.code(), e.message))
    }

    fn index(&self, cell: Cell) -> usize {
        cell.row * self.snap.cols + cell.col
    }
}

/// A revision no other sheet has had. A job's result is matched with its sheet by revision, so a
/// result for a sheet that was replaced can never be taken for the one that replaced it.
fn fresh_rev() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) << 40
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
