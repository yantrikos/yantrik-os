//! A job that worked the sheet out landed.
//!
//! Its result is taken only if the sheet is still at the revision it was made for. Either way the
//! call it was started for is answered from it, because that is the sheet that call changed: what
//! the person typed afterwards, and no other call, can become its answer.

use serde_json::{json, Value};
use yantrik_app::effects::Effects;

use super::{cells, surface};
use crate::address::Cell;
use crate::command::{Facts, Reply, ReplyKind};
use crate::model::{Cue, Model, Redraw};
use crate::sheet::Solved;
use crate::YSheets;

/// Said when a job working the sheet out ended without an answer.
const LOST: &str = "The sheet could not be worked out, so the values shown may be out of date. Edit a cell to try again.";

pub fn landed(
    m: &mut Model,
    fx: &mut Effects<YSheets>,
    solved: Solved,
    changed: Option<Vec<usize>>,
    then: Option<Reply>,
) {
    let taken = m.book_mut().sheet.install(&solved);
    if taken {
        match changed {
            Some(cells) => m.redraw.add(cells),
            None => m.redraw = Redraw::All,
        }
        watched(m, &solved);
    }
    if let Some(reply) = then {
        let answer = answer(&solved, &reply);
        if taken {
            said(m, &solved, &reply);
        }
        surface::answer(m, fx, answer);
    }
}

/// The cell the person typed into, if it came out an error: said, and the selection brought back
/// to it unless they have moved on, as it was when the sheet was worked out as they typed.
fn watched(m: &mut Model, solved: &Solved) {
    let Some(watch) = m.watch.take() else { return };
    let Some(problem) = solved.problem(watch.cell) else { return };
    cells::cell_failed(m, problem);
    if m.moves == watch.moves && m.active != watch.cell {
        cells::select(m, watch.cell);
        cells::cell_failed(m, solved.problem(watch.cell).unwrap_or_default());
        m.cues.push(Cue::LoadField);
    }
}

/// What a call's edit came to, on the status line and in `describe.notice`.
fn said(m: &mut Model, solved: &Solved, reply: &Reply) {
    if let ReplyKind::SetCell { cell, .. } = &reply.kind {
        match solved.problem(*cell) {
            Some(problem) => cells::cell_failed(m, problem),
            None => cells::done(m, format!("{} set.", cell.name())),
        }
    }
}

/// A job working the sheet out ended without an answer. The call it was for is refused; the sheet
/// is left as it is, and no job is started for that revision again.
pub fn lost(m: &mut Model, fx: &mut Effects<YSheets>, rev: u64, then: Option<Reply>) {
    if m.sheet().rev() == rev {
        m.solve_lost = Some(rev);
        super::say(m, LOST);
    }
    if then.is_some() {
        surface::settle_refused(fx, LOST);
    }
}

/// One cell as a call answers about it.
fn cell_json(solved: &Solved, cell: Cell) -> Value {
    let v = solved.value(cell);
    let mut out = json!({
        "cell": cell.name(),
        "typed": solved.raw(cell),
        "value": v.to_json(),
        "shows": v.display(),
    });
    if let Some(e) = v.error() {
        out["problem"] = json!(format!("{} shows {}: {}.", cell.name(), e.code(), e.message));
    }
    out
}

/// What a call is answered with, built from the sheet its own edit made and the facts of that
/// moment.
fn answer(solved: &Solved, reply: &Reply) -> Value {
    let Facts { dirty, can_undo, can_redo, rows, cols } = reply.facts;
    match &reply.kind {
        ReplyKind::SetCell { cell, before, typed } => {
            let mut out = cell_json(solved, *cell);
            out["before"] = json!(before);
            out["changed"] = json!(before != typed);
            out["rows"] = json!(rows);
            out["cols"] = json!(cols);
            out["dirty"] = json!(dirty);
            out
        }
        ReplyKind::Select(cell) => cell_json(solved, *cell),
        ReplyKind::Undo(cell) => {
            let mut out = cell_json(solved, *cell);
            out["undone"] = json!(true);
            out["can_undo"] = json!(can_undo);
            out["dirty"] = json!(dirty);
            out
        }
        ReplyKind::Redo(cell) => {
            let mut out = cell_json(solved, *cell);
            out["redone"] = json!(true);
            out["can_redo"] = json!(can_redo);
            out["dirty"] = json!(dirty);
            out
        }
        ReplyKind::Range(range) => {
            let typed: Vec<Vec<&str>> = (range.first.row..=range.last.row)
                .map(|r| (range.first.col..=range.last.col).map(|c| solved.raw(Cell::new(r, c))).collect())
                .collect();
            let values: Vec<Vec<Value>> =
                solved.range_values(range).iter().map(|row| row.iter().map(|v| v.to_json()).collect()).collect();
            let errors: Vec<Value> = range
                .cells()
                .filter_map(|c| solved.problem(c).map(|p| json!({ "cell": c.name(), "problem": p })))
                .collect();
            json!({
                "range": range.name(),
                "rows": range.rows(),
                "cols": range.cols(),
                "typed": typed,
                "values": values,
                "errors": errors,
                "outside_sheet": range.last.row >= solved.rows() || range.last.col >= solved.cols(),
            })
        }
    }
}

/// The answer for a sheet that is already worked out, with no job: what `select` and `get_range`
/// give when nothing was typed since the last result.
pub fn answer_now(solved: &Solved, reply: &Reply) -> Value {
    answer(solved, reply)
}
