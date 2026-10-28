//! The sheet and its history: the content of the framework's document.
//!
//! What is saved is what was typed, formulas included (`=SUM(A1:A5)`), the way LibreOffice's "save
//! cell formulas" does it. CSV has no other place for them, and saving only the values would mean
//! a sheet saved and reopened had lost every formula in it. Excel and LibreOffice both read a
//! leading `=` in a CSV cell as a formula again.

use crate::address::Cell;
use crate::csv;
use crate::history::{Edit, History, Mark};
use crate::sheet::Sheet;

#[derive(Debug, Clone)]
pub struct Book {
    pub sheet: Sheet,
    pub history: History,
    /// Changes whenever the sheet's text does, by an edit, an undo, a redo or a file taking its
    /// place. How the document is told the content moved.
    revision: u64,
}

impl Default for Book {
    fn default() -> Self {
        Self::new()
    }
}

impl Book {
    /// An empty sheet, saved as far as anything knows.
    pub fn new() -> Self {
        Self { sheet: Sheet::default(), history: History::clean(), revision: 0 }
    }

    /// A sheet that was read from a file: nothing to undo, nothing unsaved.
    pub fn opened(sheet: Sheet) -> Self {
        Self { sheet, history: History::clean(), revision: 0 }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn dirty(&self) -> bool {
        self.history.dirty()
    }

    /// Put `text` in a cell as one undoable step. Returns the text it replaced. The sheet is not
    /// worked out (`Sheet::put`).
    pub fn put(&mut self, cell: Cell, text: &str) -> Result<String, String> {
        let before = self.sheet.put(cell, text)?;
        self.history.record(Edit { cell, before: before.clone(), after: text.to_string() });
        self.revision += 1;
        Ok(before)
    }

    pub fn undo(&mut self) -> Option<Edit> {
        let edit = self.history.undo(&mut self.sheet)?;
        self.revision += 1;
        Some(edit)
    }

    pub fn redo(&mut self) -> Option<Edit> {
        let edit = self.history.redo(&mut self.sheet)?;
        self.revision += 1;
        Some(edit)
    }

    /// The file this sheet is written as: the used block, one line per row.
    pub fn csv(&self) -> String {
        csv::write(&self.sheet.used_rows())
    }

    /// Where the sheet is now, to be marked saved when the write of its text lands.
    pub fn mark(&self) -> Mark {
        self.history.mark()
    }

    /// The write of the sheet as it was at `mark` landed.
    pub fn saved_at(&mut self, mark: Mark) {
        self.history.mark_saved_at(mark);
    }

    /// Another program's version of the file takes the place of the sheet: what was typed is
    /// gone, nothing is left to undo, and nothing is unsaved.
    pub fn followed(&mut self, theirs: Sheet) {
        self.sheet = theirs;
        self.history = History::clean();
        self.revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_change_moves_the_revision_and_a_following_clears_the_history() {
        let mut book = Book::new();
        let a1 = Cell::new(0, 0);
        assert_eq!(book.revision(), 0);
        book.put(a1, "1").unwrap();
        assert_eq!(book.revision(), 1);
        book.undo().unwrap();
        book.redo().unwrap();
        assert_eq!(book.revision(), 3);
        assert!(book.dirty());
        book.followed(Sheet::default());
        assert_eq!(book.revision(), 4);
        assert!(!book.dirty() && !book.history.can_undo());
    }

    #[test]
    fn the_csv_is_the_used_block_of_what_was_typed() {
        let mut book = Book::new();
        book.put(Cell::new(1, 1), "=1+1").unwrap();
        book.put(Cell::new(0, 0), "a,b").unwrap();
        assert_eq!(book.csv(), "\"a,b\",\r\n,=1+1\r\n");
    }
}
