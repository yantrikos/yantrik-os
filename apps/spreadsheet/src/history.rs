//! Undo and redo, one cell edit at a time, and whether the sheet matches what was last saved.
//!
//! The old engine kept a whole copy of every sheet per step (5c6a55aa^, `Snapshot`), fifty deep.
//! A step here is the one cell that changed, its text before and after, so a deep history of a
//! large sheet costs a few strings each.

use crate::address::Cell;
use crate::sheet::Sheet;

/// How many edits can be undone.
pub const DEPTH: usize = 500;

#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub cell: Cell,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    /// How deep the undo stack was when the file was last saved or opened; `None` once that
    /// state can no longer be reached by undo or redo.
    saved: Option<usize>,
}

impl History {
    /// A history whose present is the file as it is on disk (or a new, empty sheet).
    pub fn clean() -> Self {
        Self { undo: Vec::new(), redo: Vec::new(), saved: Some(0) }
    }

    pub fn record(&mut self, edit: Edit) {
        if edit.before == edit.after {
            return;
        }
        // Redo is gone; if the saved state was only reachable by redo, so is it.
        if self.saved.is_some_and(|s| s > self.undo.len()) {
            self.saved = None;
        }
        self.redo.clear();
        self.undo.push(edit);
        if self.undo.len() > DEPTH {
            self.undo.remove(0);
            self.saved = self.saved.and_then(|s| s.checked_sub(1));
        }
    }

    /// Put the last edit back. Returns it, or `None` when there is nothing to undo.
    pub fn undo(&mut self, sheet: &mut Sheet) -> Option<Edit> {
        let edit = self.undo.pop()?;
        // An edit could only have been made inside the grid, so this cannot fail to fit.
        let _ = sheet.set(edit.cell, &edit.before);
        self.redo.push(edit.clone());
        Some(edit)
    }

    pub fn redo(&mut self, sheet: &mut Sheet) -> Option<Edit> {
        let edit = self.redo.pop()?;
        let _ = sheet.set(edit.cell, &edit.after);
        self.undo.push(edit.clone());
        Some(edit)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Whether the sheet differs from what was last saved or opened.
    pub fn dirty(&self) -> bool {
        self.saved != Some(self.undo.len())
    }

    pub fn mark_saved(&mut self) {
        self.saved = Some(self.undo.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(sheet: &mut Sheet, h: &mut History, cell: Cell, text: &str) {
        let before = sheet.set(cell, text).unwrap();
        h.record(Edit { cell, before, after: text.into() });
    }

    #[test]
    fn undo_puts_back_and_redo_puts_forward_and_formulas_follow() {
        let mut s = Sheet::default();
        let mut h = History::clean();
        let (a1, a2, a3) = (Cell::new(0, 0), Cell::new(1, 0), Cell::new(2, 0));
        edit(&mut s, &mut h, a1, "2");
        edit(&mut s, &mut h, a2, "3");
        edit(&mut s, &mut h, a3, "=SUM(A1:A2)");
        assert_eq!(s.value(a3).display(), "5");

        assert_eq!(h.undo(&mut s).unwrap().cell, a3);
        assert_eq!(s.raw(a3), "");
        assert_eq!(h.undo(&mut s).unwrap().cell, a2);
        assert_eq!(s.raw(a2), "");
        assert!(h.redo(&mut s).is_some());
        assert!(h.redo(&mut s).is_some());
        assert_eq!(s.value(a3).display(), "5");
        assert!(h.redo(&mut s).is_none(), "nothing left to redo");
    }

    #[test]
    fn a_new_edit_after_undo_drops_the_redo() {
        let mut s = Sheet::default();
        let mut h = History::clean();
        let a1 = Cell::new(0, 0);
        edit(&mut s, &mut h, a1, "1");
        h.undo(&mut s);
        edit(&mut s, &mut h, a1, "9");
        assert!(!h.can_redo());
        assert_eq!(s.raw(a1), "9");
    }

    #[test]
    fn dirty_follows_undo_back_to_the_saved_state() {
        let mut s = Sheet::default();
        let mut h = History::clean();
        assert!(!h.dirty());
        edit(&mut s, &mut h, Cell::new(0, 0), "1");
        assert!(h.dirty());
        h.mark_saved();
        assert!(!h.dirty());
        edit(&mut s, &mut h, Cell::new(0, 1), "2");
        assert!(h.dirty());
        h.undo(&mut s);
        assert!(!h.dirty(), "undoing back to the saved text is the saved text");
        h.undo(&mut s);
        assert!(h.dirty());
        // A different edit from here can never get back to what was saved by redo.
        edit(&mut s, &mut h, Cell::new(5, 5), "x");
        h.undo(&mut s);
        assert!(h.dirty());
    }

    #[test]
    fn an_edit_that_changes_nothing_is_not_a_step() {
        let mut s = Sheet::default();
        let mut h = History::clean();
        edit(&mut s, &mut h, Cell::new(0, 0), "");
        assert!(!h.can_undo());
        assert!(!h.dirty());
    }
}
