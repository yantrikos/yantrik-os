//! What the window is holding: the document, the selected cell, and what the status line says.
//!
//! One `Session` per window, shared by the buttons, the keyboard and the control surface, so a
//! person and a mind are always looking at the same sheet and the same selection.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::address::Cell;
use crate::document::Document;

/// What waits on the unsaved-changes question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    Open,
    Close,
}

#[derive(Debug)]
pub struct Session {
    pub doc: Document,
    pub active: Cell,
    /// The last thing that happened, in words, and whether it went wrong. Replaces the status
    /// line's summary until the selection moves.
    pub said: Option<(String, bool)>,
    /// The last failure, for `describe.notice`; cleared by the next success.
    pub notice: String,
    pub pending: Option<Pending>,
    /// A typed save path the person has been told already exists, and has not yet confirmed.
    pub confirm_replace: Option<PathBuf>,
    /// The desktop's file picker is open; a second one is not asked for.
    pub picking: bool,
    /// The person chose to close without saving.
    pub closing: bool,
}

pub type Shared = Rc<RefCell<Session>>;

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            doc: Document::new(),
            active: Cell::new(0, 0),
            said: None,
            notice: String::new(),
            pending: None,
            confirm_replace: None,
            picking: false,
            closing: false,
        }
    }

    /// Something went right: say it, and clear the last failure.
    pub fn done(&mut self, text: impl Into<String>) {
        self.said = Some((text.into(), false));
        self.notice.clear();
    }

    /// Something went wrong: say it on screen and keep it for `describe`. Returns the sentence,
    /// so a surface handler can refuse with the same words.
    pub fn failed(&mut self, text: impl Into<String>) -> String {
        let text = text.into();
        self.said = Some((text.clone(), true));
        self.notice = text.clone();
        text
    }

    /// Move the selection, keeping it inside the grid.
    pub fn select(&mut self, cell: Cell) {
        let s = &self.doc.sheet;
        self.active = Cell::new(cell.row.min(s.rows() - 1), cell.col.min(s.cols() - 1));
        self.said = None;
    }

    /// The selection moved by one step, as Enter, Tab and the arrows move it.
    pub fn step(&mut self, direction: &str) {
        let Cell { row, col } = self.active;
        let next = match direction {
            "down" => Cell::new(row + 1, col),
            "up" => Cell::new(row.saturating_sub(1), col),
            "right" => Cell::new(row, col + 1),
            "left" => Cell::new(row, col.saturating_sub(1)),
            _ => self.active,
        };
        // Moving off the bottom or right edge grows the sheet by a row or column, so Enter on
        // the last row keeps going, as in every spreadsheet.
        let s = &mut self.doc.sheet;
        if next.row >= s.rows() || next.col >= s.cols() {
            let _ = s.resize(s.rows().max(next.row + 1), s.cols().max(next.col + 1));
        }
        self.select(next);
    }

    /// Put `text` in the selected cell, and say what came of it.
    pub fn commit(&mut self, text: &str) {
        let cell = self.active;
        if self.doc.sheet.raw(cell) == text {
            return;
        }
        match self.doc.set(cell, text) {
            Ok(_) => match self.doc.sheet.problem(cell) {
                Some(problem) => {
                    self.failed(problem);
                }
                None => self.said = None,
            },
            Err(e) => {
                self.failed(format!("{} was not changed: {e}", cell.name()));
            }
        }
    }

    /// Undo the last edit and select the cell it was in.
    pub fn undo(&mut self) -> bool {
        match self.doc.history.undo(&mut self.doc.sheet) {
            Some(edit) => {
                self.select(edit.cell);
                self.done(format!("Undid the change to {}.", edit.cell.name()));
                true
            }
            None => {
                self.said = Some(("Nothing to undo.".into(), false));
                false
            }
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.doc.history.redo(&mut self.doc.sheet) {
            Some(edit) => {
                self.select(edit.cell);
                self.done(format!("Redid the change to {}.", edit.cell.name()));
                true
            }
            None => {
                self.said = Some(("Nothing to redo.".into(), false));
                false
            }
        }
    }

    /// What the status line says now, and whether it is a problem.
    pub fn status(&self) -> (String, bool) {
        if let Some((text, problem)) = &self.said {
            return (text.clone(), *problem);
        }
        if let Some(problem) = self.doc.sheet.problem(self.active) {
            return (problem, true);
        }
        let s = &self.doc.sheet;
        let (filled, errors) = s.counts();
        let mut line = format!(
            "{} · {} rows × {} columns · {} {} filled",
            self.active.name(),
            s.rows(),
            s.cols(),
            filled,
            if filled == 1 { "cell" } else { "cells" }
        );
        if errors > 0 {
            let first = s.first_error().map(|c| c.name()).unwrap_or_default();
            line.push_str(&format!(
                " · {errors} {} an error, the first in {first}",
                if errors == 1 { "shows" } else { "show" }
            ));
        }
        (line, false)
    }
}
