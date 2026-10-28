//! What a recalculation is given and what it hands back.
//!
//! A [`Snapshot`] is the typed text of the sheet at one revision. It shares its rows with the sheet
//! (one `Arc` per row), so taking one costs a pointer per row and a later edit copies only the
//! row it touches. A job works it out off the window's thread and hands back a [`Solved`]: the
//! snapshot and the values it came to. A call from the surface is answered from the `Solved` its
//! own job made, so nothing typed meanwhile can become its answer.

use std::sync::Arc;

use super::recalc;
use crate::address::{Cell, Range};
use crate::formula::Value;

/// The typed text of every cell at one revision of the sheet.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub(super) rev: u64,
    pub(super) rows: usize,
    pub(super) cols: usize,
    pub(super) raw: Vec<Arc<Vec<String>>>,
}

impl Snapshot {
    /// The revision of the sheet this is the text of.
    pub fn rev(&self) -> u64 {
        self.rev
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
        if self.contains(cell) { &self.raw[cell.row][cell.col] } else { "" }
    }

    /// The smallest top-left block that holds every non-empty cell.
    pub fn used(&self) -> (usize, usize) {
        let (mut rows, mut cols) = (0, 0);
        for (r, row) in self.raw.iter().enumerate() {
            if let Some(c) = row.iter().rposition(|t| !t.is_empty()) {
                rows = r + 1;
                cols = cols.max(c + 1);
            }
        }
        (rows, cols)
    }

    /// The typed text of the used block, row by row: what a file is written from.
    pub fn used_rows(&self) -> Vec<Vec<String>> {
        let (rows, cols) = self.used();
        self.raw[..rows].iter().map(|row| row[..cols].to_vec()).collect()
    }

    /// How many cells hold something.
    pub fn filled(&self) -> usize {
        self.raw.iter().map(|row| row.iter().filter(|t| !t.is_empty()).count()).sum()
    }

    /// Works every cell out. The work of a job; it never touches the sheet.
    pub fn solve(self) -> Solved {
        let values = recalc::recalc(self.rows, self.cols, &self.raw);
        Solved { snap: self, values: Arc::new(values) }
    }
}

/// A snapshot and what each cell of it works out to.
#[derive(Debug, Clone)]
pub struct Solved {
    pub(super) snap: Snapshot,
    pub(super) values: Arc<Vec<Value>>,
}

impl Solved {
    pub fn rev(&self) -> u64 {
        self.snap.rev
    }

    pub fn rows(&self) -> usize {
        self.snap.rows
    }

    pub fn cols(&self) -> usize {
        self.snap.cols
    }

    pub fn contains(&self, cell: Cell) -> bool {
        self.snap.contains(cell)
    }

    pub fn raw(&self, cell: Cell) -> &str {
        self.snap.raw(cell)
    }

    pub fn values(&self) -> &Arc<Vec<Value>> {
        &self.values
    }

    /// What a cell works out to; blank outside the grid.
    pub fn value(&self, cell: Cell) -> Value {
        if self.contains(cell) { self.values[cell.row * self.snap.cols + cell.col].clone() } else { Value::Empty }
    }

    pub fn used(&self) -> (usize, usize) {
        self.snap.used()
    }

    /// The values of a range, row by row; cells past the edge are blank.
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

    /// The flat indices of the cells whose value is not what `before` held, or `None` when the
    /// two are not the same shape (everything is to be drawn again).
    pub fn changed_from(&self, before: &[Value]) -> Option<Vec<usize>> {
        if before.len() != self.values.len() {
            return None;
        }
        Some(self.values.iter().zip(before).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i).collect())
    }
}
