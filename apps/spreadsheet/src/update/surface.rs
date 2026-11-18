//! A call from the surface, carried out, and its answer.
//!
//! Every answer is built from that call's own work, when it committed: the cell it changed, read
//! back from the sheet worked out for that change; the file it read or wrote. Nothing the person
//! does meanwhile, and no other call, becomes this call's answer: a call that starts a job is
//! settled when that job lands, with what the job found. A refusal is said twice, as it always
//! was: to the caller, and on the status line and in `describe.notice`.
//!
//! Grades: reading is `safe`; changing the sheet, opening a file and writing a NEW file are
//! `standard`; replacing a file that is already on the disk is `save_over`, graded `sensitive`,
//! because that is the one that destroys something a person cannot get back. `save` refuses to
//! replace and names `save_over`, so the grade cannot be stepped around (intents.rs).

use std::path::{Path, PathBuf};

use serde_json::Value;
use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use super::{cells, opening, recalc, saving, solved};
use crate::address::{self, Cell};
use crate::caller::Caller;
use crate::command::{Act, AfterSave, Cause, Reply, ReplyKind};
use crate::files::names::resolve;
use crate::model::Model;
use crate::YSheets;

/// The most cells one `get_range` returns.
pub const MAX_RANGE_CELLS: usize = 2_000;

/// The most characters one cell holds, typed or worked out.
const MAX_TEXT: usize = crate::formula::budget::MAX_TEXT;

type Done = Result<Option<Value>, String>;

pub fn act(m: &mut Model, fx: &mut Effects<YSheets>, act: Act, caller: Caller) {
    let done = match act {
        Act::SetCell { cell, text } => set_cell(m, fx, &cell, text, caller),
        Act::GetRange { range } => get_range(m, fx, &range, caller),
        Act::Select { cell } => select(m, fx, &cell, caller),
        Act::Undo => step(m, fx, caller, true),
        Act::Redo => step(m, fx, caller, false),
        Act::Open { path } => open(m, fx, &path, caller),
        Act::Save { path } => save(m, fx, path.as_deref(), false, caller),
        Act::SaveOver { path } => save(m, fx, path.as_deref(), true, caller),
    };
    match done {
        Ok(Some(value)) => answer(m, fx, value),
        // Answered when its job lands.
        Ok(None) => {}
        Err(sentence) => refuse(m, fx, sentence),
    }
}

/// The call answered with `value`, in the command that committed its work.
pub fn answer(m: &mut Model, fx: &mut Effects<YSheets>, value: Value) {
    m.answer = value;
    fx.settle(Ok(()));
}

/// A refusal the person at the window sees too: the status line and the notice, and the call's
/// error.
pub fn refuse(m: &mut Model, fx: &mut Effects<YSheets>, sentence: impl Into<String>) {
    let sentence = cells::failed(m, sentence);
    tracing::warn!(error = %sentence, "ySheets refused an action");
    settle_refused(fx, sentence);
}

/// The call refused with `sentence`, when the notice already says it in the form its caller may
/// read.
pub fn settle_refused(fx: &mut Effects<YSheets>, sentence: impl Into<String>) {
    fx.settle(Err(Refused::new("refused", sentence.into())));
}

fn cell_arg(raw: &str) -> Result<Cell, String> {
    address::parse_cell(raw).ok_or_else(|| format!("`{raw}` is not a cell; name one like B3"))
}

/// A path from an argument, made absolute, or a refusal saying what was wrong with it.
fn path_arg(raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("No path was given.".into());
    }
    if !raw.starts_with("~/") && !Path::new(raw).is_absolute() {
        return Err(format!("`{raw}` is not an absolute path; give one starting with / or ~/"));
    }
    Ok(resolve(raw, Path::new("/")))
}

/// `set_cell`: the text typed into the cell, and the answer read back from the sheet worked out for
/// that edit.
fn set_cell(m: &mut Model, fx: &mut Effects<YSheets>, cell: &str, text: String, caller: Caller) -> Done {
    cells::not_hidden(m, caller)?;
    let cell = cell_arg(cell)?;
    if text.chars().count() > MAX_TEXT {
        return Err(format!("That text is longer than the {} characters one cell holds.", group(MAX_TEXT)));
    }
    let before =
        cells::put(m, cell, &text).map_err(|e| format!("{} was not changed: {e}", cell.name()))?;
    // The person sees where the mind typed.
    cells::moved(m, cell);
    let reply = Reply { kind: ReplyKind::SetCell { cell, before, typed: text }, facts: cells::facts(m) };
    recalc::spawn(m, fx, Some(reply));
    Ok(None)
}

/// 32768 as 32,768.
fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A call whose answer is the sheet as worked out: at once if it is, from a job of its own if not.
fn read(m: &mut Model, fx: &mut Effects<YSheets>, kind: ReplyKind) -> Done {
    let reply = Reply { kind, facts: cells::facts(m) };
    if m.sheet().is_solved() {
        return Ok(Some(solved::answer_now(&m.sheet().view(), &reply)));
    }
    recalc::spawn(m, fx, Some(reply));
    Ok(None)
}

fn get_range(m: &mut Model, fx: &mut Effects<YSheets>, range: &str, caller: Caller) -> Done {
    cells::not_hidden(m, caller)?;
    let range =
        address::parse_range(range).ok_or_else(|| format!("`{range}` is not a range; name one like A1:C10"))?;
    if range.len() > MAX_RANGE_CELLS {
        return Err(format!(
            "{} is {} cells; ask for {MAX_RANGE_CELLS} or fewer at a time",
            range.name(),
            range.len()
        ));
    }
    read(m, fx, ReplyKind::Range(range))
}

fn select(m: &mut Model, fx: &mut Effects<YSheets>, cell: &str, caller: Caller) -> Done {
    cells::not_hidden(m, caller)?;
    let cell = cell_arg(cell)?;
    if !m.sheet().contains(cell) {
        return Err(format!(
            "{} is outside the sheet, which is {} rows × {} columns",
            cell.name(),
            m.sheet().rows(),
            m.sheet().cols()
        ));
    }
    cells::moved(m, cell);
    read(m, fx, ReplyKind::Select(cell))
}

fn step(m: &mut Model, fx: &mut Effects<YSheets>, caller: Caller, undo: bool) -> Done {
    cells::not_hidden(m, caller)?;
    let moved = if undo { cells::undo(m) } else { cells::redo(m) };
    let cell = moved.ok_or_else(|| format!("There is nothing to {}.", if undo { "undo" } else { "redo" }))?;
    let kind = if undo { ReplyKind::Undo(cell) } else { ReplyKind::Redo(cell) };
    let reply = Reply { kind, facts: cells::facts(m) };
    recalc::spawn(m, fx, Some(reply));
    Ok(None)
}

/// `open`: the file read in a job, and the answer built from what it found. Refused while the sheet
/// has unsaved changes.
fn open(m: &mut Model, fx: &mut Effects<YSheets>, path: &str, caller: Caller) -> Done {
    let target = path_arg(path)?;
    opening::start(m, fx, target, Cause::Surface(caller), caller);
    Ok(None)
}

/// `save` and `save_over`: the sheet written to the path, or to its own file, in a job.
fn save(m: &mut Model, fx: &mut Effects<YSheets>, path: Option<&str>, over: bool, caller: Caller) -> Done {
    cells::not_hidden(m, caller)?;
    let target = match path {
        Some(raw) if !raw.trim().is_empty() => path_arg(raw)?,
        _ => m.path().map(Path::to_path_buf).ok_or("This sheet has no file yet; give a path ending in .csv.")?,
    };
    saving::start(m, fx, target, over, !over, AfterSave::Surface(caller), caller.agent);
    Ok(None)
}
