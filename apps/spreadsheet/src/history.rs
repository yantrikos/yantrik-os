//! Undo and redo, one cell edit at a time, and whether the sheet matches what was last saved.
//!
//! The old engine kept a whole copy of every sheet per step (5c6a55aa^, `Snapshot`), fifty deep.
//! A step here is the one cell that changed, its text before and after, so a deep history of a
//! large sheet costs a few strings each.
//!
//! A save is of the sheet as it was when the save began, and the sheet may be typed into while the
//! file is written. So "saved" is a [`Mark`] taken at the start, and [`History::mark_saved_at`]
//! finds the marked state again when the write lands: still there, further along by edits, or
//! further back by undo, it is the saved state; gone, because other edits were made after undoing
//! past it, nothing is.

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

/// A state of the sheet to find again: the edit that led to it, or nothing for the state before
/// the first edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark(Option<u64>);

#[derive(Debug, Default, Clone)]
pub struct History {
    undo: Vec<(u64, Edit)>,
    /// Most recently undone last.
    redo: Vec<(u64, Edit)>,
    /// How deep the undo stack was when the file was last saved or opened; `None` once that
    /// state can no longer be reached by undo or redo.
    saved: Option<usize>,
    next_id: u64,
    /// Edits at the bottom of the stack were let go, so the state before the first edit is gone.
    forgotten: bool,
}

impl History {
    /// A history whose present is the file as it is on disk (or a new, empty sheet).
    pub fn clean() -> Self {
        Self { saved: Some(0), ..Self::default() }
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
        self.next_id += 1;
        self.undo.push((self.next_id, edit));
        if self.undo.len() > DEPTH {
            self.undo.remove(0);
            self.forgotten = true;
            self.saved = self.saved.and_then(|s| s.checked_sub(1));
        }
    }

    /// Put the last edit back. Returns it, or `None` when there is nothing to undo. The sheet is
    /// not worked out (`Sheet::put`).
    pub fn undo(&mut self, sheet: &mut Sheet) -> Option<Edit> {
        let (id, edit) = self.undo.pop()?;
        // An edit could only have been made inside the grid, so this cannot fail to fit.
        let _ = sheet.put(edit.cell, &edit.before);
        self.redo.push((id, edit.clone()));
        Some(edit)
    }

    pub fn redo(&mut self, sheet: &mut Sheet) -> Option<Edit> {
        let (id, edit) = self.redo.pop()?;
        let _ = sheet.put(edit.cell, &edit.after);
        self.undo.push((id, edit.clone()));
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

    /// The state the sheet is in now, to be found again.
    pub fn mark(&self) -> Mark {
        Mark(self.undo.last().map(|(id, _)| *id))
    }

    /// The sheet as it is now is what is on disk.
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.undo.len());
    }

    /// The sheet as it was at `mark` is what is on disk now. Where that state is, is found again
    /// by the edit that led to it; if it cannot be reached any more, nothing is saved.
    pub fn mark_saved_at(&mut self, mark: Mark) {
        self.saved = match mark.0 {
            None => (!self.forgotten).then_some(0),
            Some(id) => match self.undo.iter().position(|(i, _)| *i == id) {
                Some(at) => Some(at + 1),
                None => {
                    // Undone since: the state is the one reached by redoing back to that edit.
                    let at = self.redo.iter().position(|(i, _)| *i == id);
                    at.map(|at| self.undo.len() + (self.redo.len() - at))
                }
            },
        };
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
        s.settle();
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

    #[test]
    fn a_save_that_lands_after_more_typing_marks_the_state_it_wrote() {
        let mut s = Sheet::default();
        let mut h = History::clean();
        edit(&mut s, &mut h, Cell::new(0, 0), "1");
        let mark = h.mark();
        // Typed while the file was being written.
        edit(&mut s, &mut h, Cell::new(0, 1), "2");
        h.mark_saved_at(mark);
        assert!(h.dirty(), "the second edit is not in the file");
        h.undo(&mut s);
        assert!(!h.dirty(), "undoing it is the state that was written");
    }

    #[test]
    fn a_save_that_lands_after_an_undo_still_finds_its_state() {
        let mut s = Sheet::default();
        let mut h = History::clean();
        edit(&mut s, &mut h, Cell::new(0, 0), "1");
        edit(&mut s, &mut h, Cell::new(0, 1), "2");
        let mark = h.mark();
        h.undo(&mut s);
        h.mark_saved_at(mark);
        assert!(h.dirty(), "the sheet in front is not what was written");
        h.redo(&mut s);
        assert!(!h.dirty(), "redoing is");
    }

    #[test]
    fn a_save_whose_state_was_typed_over_marks_nothing() {
        let mut s = Sheet::default();
        let mut h = History::clean();
        edit(&mut s, &mut h, Cell::new(0, 0), "1");
        edit(&mut s, &mut h, Cell::new(0, 1), "2");
        let mark = h.mark();
        h.undo(&mut s);
        edit(&mut s, &mut h, Cell::new(3, 3), "other");
        h.mark_saved_at(mark);
        assert!(h.dirty());
        h.undo(&mut s);
        h.undo(&mut s);
        assert!(h.dirty(), "no state that can be reached is the one that was written");
    }

    #[test]
    fn a_save_of_the_untouched_sheet_marks_it_unless_its_start_was_forgotten() {
        let mut s = Sheet::default();
        let mut h = History::clean();
        let start = h.mark();
        edit(&mut s, &mut h, Cell::new(0, 0), "1");
        h.mark_saved_at(start);
        assert!(h.dirty());
        h.undo(&mut s);
        assert!(!h.dirty());
        for n in 0..DEPTH + 2 {
            edit(&mut s, &mut h, Cell::new(n % 40, 0), &format!("v{n}"));
        }
        h.mark_saved_at(start);
        assert!(h.dirty() && h.can_undo(), "the empty state is no longer reachable");
    }
}
