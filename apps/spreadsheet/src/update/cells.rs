//! Typing into cells, moving the selection, undo and redo: the changes to the sheet itself, with
//! the words the status line and the notice say about them.
//!
//! None of these works the sheet out. They change the text and say which cells to draw again; the
//! job that works the sheet out is started after the command (`recalc::ensure`), or by the call
//! that needs its result (`recalc::spawn`).

use crate::address::Cell;
use crate::caller::Caller;
use crate::command::Facts;
use crate::model::{Cue, Model, Redraw, Said, Watch};

/// The grid's place for a cell, row by row.
pub fn index(m: &Model, cell: Cell) -> usize {
    cell.row * m.sheet().cols() + cell.col
}

/// Something went right: say it, and clear the last failure.
pub fn done(m: &mut Model, text: impl Into<String>) {
    m.said = Some(Said { text: text.into(), problem: false, quotes_cells: false });
    m.notice.clear();
}

/// Something went wrong: say it on screen and keep it for `describe`. Returns the sentence, so a
/// surface handler can refuse with the same words.
pub fn failed(m: &mut Model, text: impl Into<String>) -> String {
    let text = text.into();
    m.said = Some(Said { text: text.clone(), problem: true, quotes_cells: false });
    m.notice.set(text.clone());
    text
}

/// A cell that shows an error, said: the sentence quotes what the cell reads, so a Mind View
/// window does not show it and an agent is not given it for a sheet it is not shown.
pub fn cell_failed(m: &mut Model, problem: String) {
    m.said = Some(Said { text: problem.clone(), problem: true, quotes_cells: true });
    let hidden = m.doc.hidden;
    m.notice.set_about(hidden, problem, String::new);
}

/// Move the selection, keeping it inside the grid.
pub fn select(m: &mut Model, cell: Cell) {
    let s = m.sheet();
    m.active = Cell::new(cell.row.min(s.rows() - 1), cell.col.min(s.cols() - 1));
    m.said = None;
}

/// The person moved the selection: whoever was waiting to bring it back to a cell that went wrong
/// no longer may.
pub fn moved(m: &mut Model, cell: Cell) {
    m.moves += 1;
    select(m, cell);
    m.cues.push(Cue::LoadField);
}

/// The selection moved by one step, as Enter, Tab and the arrows move it.
pub fn step(m: &mut Model, direction: &str) {
    let Cell { row, col } = m.active;
    let next = match direction {
        "down" => Cell::new(row + 1, col),
        "up" => Cell::new(row.saturating_sub(1), col),
        "right" => Cell::new(row, col + 1),
        "left" => Cell::new(row, col.saturating_sub(1)),
        _ => m.active,
    };
    // Moving off the bottom or right edge grows the sheet by a row or column, so Enter on the last
    // row keeps going, as in every spreadsheet.
    let s = &m.book().sheet;
    if next.row >= s.rows() || next.col >= s.cols() {
        let (rows, cols) = (s.rows().max(next.row + 1), s.cols().max(next.col + 1));
        if m.book_mut().sheet.resize(rows, cols).is_ok() {
            m.redraw = Redraw::All;
        }
    }
    moved(m, next);
}

/// Put `text` in the selected cell, and say what came of it. Nothing happens when the cell
/// already holds it; and in a window drawn for a Mind nothing is ever written from the window.
pub fn commit(m: &mut Model, text: &str) {
    let cell = m.active;
    if m.for_mind || m.sheet().raw(cell) == text {
        return;
    }
    if let Err(e) = put(m, cell, text) {
        failed(m, format!("{} was not changed: {e}", cell.name()));
        return;
    }
    // If the cell comes out an error, the selection comes back to it (`solved::landed`).
    m.watch = Some(Watch { cell, rev: m.sheet().rev(), moves: m.moves });
    m.said = None;
}

/// Put `text` in a cell as one undoable step; returns what it replaced. Says which cells to draw.
pub fn put(m: &mut Model, cell: Cell, text: &str) -> Result<String, String> {
    let (rows, cols) = (m.sheet().rows(), m.sheet().cols());
    let before = m.book_mut().put(cell, text)?;
    if (rows, cols) != (m.sheet().rows(), m.sheet().cols()) {
        m.redraw = Redraw::All;
    } else {
        let at = index(m, cell);
        m.redraw.add([at]);
    }
    Ok(before)
}

/// What a call's answer carries of the sheet as it is after the call's own edit.
pub fn facts(m: &Model) -> Facts {
    let b = m.book();
    Facts {
        dirty: m.modified(),
        can_undo: b.history.can_undo(),
        can_redo: b.history.can_redo(),
        rows: b.sheet.rows(),
        cols: b.sheet.cols(),
    }
}

/// Undo the last edit and select the cell it was in. Says which cell, or that there is nothing.
pub fn undo(m: &mut Model) -> Option<Cell> {
    let Some(edit) = m.book_mut().undo() else {
        m.said = Some(Said { text: "Nothing to undo.".into(), problem: false, quotes_cells: false });
        return None;
    };
    redrawn(m, edit.cell);
    done(m, format!("Undid the change to {}.", edit.cell.name()));
    Some(edit.cell)
}

pub fn redo(m: &mut Model) -> Option<Cell> {
    let Some(edit) = m.book_mut().redo() else {
        m.said = Some(Said { text: "Nothing to redo.".into(), problem: false, quotes_cells: false });
        return None;
    };
    redrawn(m, edit.cell);
    done(m, format!("Redid the change to {}.", edit.cell.name()));
    Some(edit.cell)
}

/// An undo or a redo put a cell back: select it, show it, and show the formula bar again.
fn redrawn(m: &mut Model, cell: Cell) {
    let at = index(m, cell);
    m.redraw.add([at]);
    select(m, cell);
    m.cues.push(Cue::LoadField);
}

/// Whether a call from the surface is held back from the sheet in front: an agent is not shown a
/// sheet the person opened from a place it may not read (#443).
pub fn not_hidden(m: &Model, caller: Caller) -> Result<(), String> {
    if caller.agent && m.doc.hidden {
        return Err(crate::agent_rule::HIDDEN_REFUSAL.into());
    }
    Ok(())
}
