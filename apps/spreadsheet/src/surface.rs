//! `app.describe` and `app.act` for ySheets: what the window holds, and the same things its
//! buttons do, offered by name.
//!
//! Every handler answers from the sheet after the change, never from what it was asked to do:
//! `set_cell` reads the cell back, `save` reads the file back (`Document::save`), `open` counts
//! what it read. A refusal is said twice, as the contract asks: in the reply, and on the status
//! line and in `describe.notice`.
//!
//! Grades: reading is `safe`; changing the sheet, opening a file and writing a NEW file are
//! `standard`; replacing a file that is already on the disk is `save_over`, graded `sensitive`,
//! because that is the one that destroys something a person cannot get back. `save` refuses to
//! replace and names `save_over`, so the grade cannot be stepped around.

use std::path::{Path, PathBuf};

use serde_json::{json, Value as Json};
use yantrik_app_runtime::control::{Action, Param, View};

use crate::address::{self, Cell};
use crate::agent_rule;
use crate::document::{self, SaveError};
use crate::formula::functions::NAMES;
use crate::session::Session;

/// The most cells one `get_range` returns.
pub const MAX_RANGE_CELLS: usize = 2_000;

pub type Handler = fn(&mut Session, &Json) -> Result<Json, String>;

/// Run one action for whoever is calling. The rule that a sheet opened from a place an agent
/// may not read is not read back to one is kept here, once, for every action: `select`,
/// `set_cell` (`=A1&B1`), `undo` and `redo` all answer with cell values, and `save` would put
/// the cells in a file the agent can read (security review of #727). Only `open` goes through,
/// because it replaces the sheet and is refused while there are unsaved changes.
pub fn run(s: &mut Session, name: &str, handler: Handler, args: &Json) -> Result<Json, String> {
    if name != "open" {
        if let Some(why) = agent_rule::hidden_from_caller(s.doc.path.as_deref()) {
            return Err(s.failed(format!("This sheet is not read or saved for an agent: {why}")));
        }
    }
    handler(s, args)
}

pub fn actions() -> Vec<(Action, Handler)> {
    vec![
        (
            Action::new("set_cell", "Type into one cell, as the person would: a number, text, or a formula starting with =. Answers with what the cell now holds and works out to.")
                .arg(Param::text("cell").describe("The cell, like B3"))
                .arg(Param::text("text").describe("What to type; empty clears the cell")),
            set_cell,
        ),
        (
            Action::new("get_range", "Read a range of cells: what was typed in each, and what each works out to.")
                .arg(Param::text("range").describe("Like A1:C10, or one cell"))
                .risk("safe"),
            get_range,
        ),
        (
            Action::new("select", "Select a cell, so the person sees it and the formula bar shows it.")
                .arg(Param::text("cell").describe("The cell, like B3")),
            select,
        ),
        (Action::new("undo", "Undo the last change to a cell."), undo),
        (Action::new("redo", "Redo the change undo last took back."), redo),
        (
            Action::new("open", "Open a CSV file in this window. Refused while the sheet has unsaved changes.")
                .arg(Param::text("path").describe("An absolute path, or one starting with ~/")),
            open,
        ),
        (
            Action::new("save", "Save the sheet as a NEW CSV file. Refuses to replace a file that is already there; that is save_over.")
                .arg(Param::text("path").describe("Where to write, ending in .csv. Omit to use the sheet's own file, if it has none on disk yet").optional()),
            save,
        ),
        (
            Action::new("save_over", "Save the sheet as CSV, replacing the file that is already at the path. What was in that file is lost.")
                .arg(Param::text("path").describe("The file to replace, ending in .csv. Omit to save over the sheet's own file").optional())
                .risk("sensitive"),
            save_over,
        ),
    ]
}

pub fn describe(s: &Session) -> View {
    let doc = &s.doc;
    let sheet = &doc.sheet;
    let hidden = agent_rule::hidden_from_caller(doc.path.as_deref());
    if let Some(why) = hidden {
        return hidden_view(s, why);
    }
    let (used_rows, used_cols) = sheet.used();
    let (filled, errors) = sheet.counts();
    let notice = s.notice.clone();
    let mut summary = format!(
        "ySheets — {} · {} rows × {} columns · {} selected · {}",
        doc.title(),
        sheet.rows(),
        sheet.cols(),
        s.active.name(),
        if doc.dirty() { "unsaved changes" } else { "saved" }
    );
    if !notice.is_empty() {
        summary.push_str(&format!(" · {notice}"));
    }
    View::new(summary)
        .with("sheet", doc.title())
        .with("path", doc.path.as_ref().map(|p| p.display().to_string()))
        .with("rows", sheet.rows() as i64)
        .with("cols", sheet.cols() as i64)
        .with("used", if used_rows == 0 { "".to_string() } else {
            address::Range::new(Cell::new(0, 0), Cell::new(used_rows - 1, used_cols - 1)).name()
        })
        .with("selection", json!({
            "cell": s.active.name(),
            "typed": sheet.raw(s.active),
            "value": sheet.value(s.active).to_json(),
        }))
        .with("dirty", doc.dirty())
        .with("can_undo", doc.history.can_undo())
        .with("can_redo", doc.history.can_redo())
        .with("filled_cells", filled as i64)
        .with("error_cells", errors as i64)
        .with("first_error", sheet.first_error().map(Cell::name))
        .with("status", s.status().0)
        .with("notice", notice)
        .with("hidden", None::<String>)
        .with("functions", NAMES.to_vec())
}

/// What an agent sees of a sheet it may not read: its name, whether it has unsaved changes (an
/// agent's `open` is refused until it is saved), and the sentence every refused action gives.
/// Nothing of its shape — size, used range, counts, the first error, the selected cell, its
/// history — and not the status line or the notice, which can quote a cell (security reviews of
/// #727). The path is left out too; the sentence names the file's place as every refusal does.
fn hidden_view(s: &Session, why: String) -> View {
    let doc = &s.doc;
    let summary = format!(
        "ySheets — {} · not read for an agent · {}",
        doc.title(),
        if doc.dirty() { "unsaved changes" } else { "saved" }
    );
    View::new(summary)
        .with("sheet", doc.title())
        .with("path", None::<String>)
        .with("rows", json!(null))
        .with("cols", json!(null))
        .with("used", json!(null))
        .with("selection", json!(null))
        .with("dirty", doc.dirty())
        .with("can_undo", json!(null))
        .with("can_redo", json!(null))
        .with("filled_cells", json!(null))
        .with("error_cells", json!(null))
        .with("first_error", None::<String>)
        .with("status", "")
        .with("notice", "")
        .with("hidden", Some(why))
        .with("functions", NAMES.to_vec())
}

fn text<'a>(args: &'a Json, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Json::as_str)
}

fn cell_arg(s: &mut Session, args: &Json) -> Result<Cell, String> {
    let raw = text(args, "cell").unwrap_or_default();
    address::parse_cell(raw).ok_or_else(|| s.failed(format!("`{raw}` is not a cell; name one like B3")))
}

/// A path from an argument, made absolute, or a refusal saying what was wrong with it.
fn path_arg(s: &mut Session, raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(s.failed("No path was given."));
    }
    let home_relative = raw.starts_with("~/");
    if !home_relative && !Path::new(raw).is_absolute() {
        return Err(s.failed(format!("`{raw}` is not an absolute path; give one starting with / or ~/")));
    }
    Ok(document::resolve(raw, Path::new("/")))
}

fn cell_json(s: &Session, cell: Cell) -> Json {
    let v = s.doc.sheet.value(cell);
    let mut out = json!({
        "cell": cell.name(),
        "typed": s.doc.sheet.raw(cell),
        "value": v.to_json(),
        "shows": v.display(),
    });
    if let Some(e) = v.error() {
        out["problem"] = json!(format!("{} shows {}: {}.", cell.name(), e.code(), e.message));
    }
    out
}

fn set_cell(s: &mut Session, args: &Json) -> Result<Json, String> {
    let cell = cell_arg(s, args)?;
    let typed = text(args, "text").unwrap_or_default().to_string();
    if typed.chars().count() > 32_768 {
        return Err(s.failed("That text is longer than the 32,768 characters one cell holds."));
    }
    let before = s.doc.set(cell, &typed).map_err(|e| s.failed(format!("{} was not changed: {e}", cell.name())))?;
    s.select(cell);
    let mut out = cell_json(s, cell);
    match s.doc.sheet.problem(cell) {
        Some(problem) => {
            s.failed(problem);
        }
        None => s.done(format!("{} set.", cell.name())),
    }
    out["before"] = json!(before);
    out["changed"] = json!(before != typed);
    out["rows"] = json!(s.doc.sheet.rows());
    out["cols"] = json!(s.doc.sheet.cols());
    out["dirty"] = json!(s.doc.dirty());
    Ok(out)
}

fn get_range(s: &mut Session, args: &Json) -> Result<Json, String> {
    let raw = text(args, "range").unwrap_or_default();
    let range = address::parse_range(raw)
        .ok_or_else(|| s.failed(format!("`{raw}` is not a range; name one like A1:C10")))?;
    if range.len() > MAX_RANGE_CELLS {
        return Err(s.failed(format!(
            "{} is {} cells; ask for {MAX_RANGE_CELLS} or fewer at a time",
            range.name(),
            range.len()
        )));
    }
    let sheet = &s.doc.sheet;
    let typed: Vec<Vec<&str>> = (range.first.row..=range.last.row)
        .map(|r| (range.first.col..=range.last.col).map(|c| sheet.raw(Cell::new(r, c))).collect())
        .collect();
    let values: Vec<Vec<Json>> =
        sheet.range_values(&range).iter().map(|row| row.iter().map(|v| v.to_json()).collect()).collect();
    let errors: Vec<Json> = range
        .cells()
        .filter_map(|c| sheet.problem(c).map(|p| json!({ "cell": c.name(), "problem": p })))
        .collect();
    Ok(json!({
        "range": range.name(),
        "rows": range.rows(),
        "cols": range.cols(),
        "typed": typed,
        "values": values,
        "errors": errors,
        "outside_sheet": range.last.row >= sheet.rows() || range.last.col >= sheet.cols(),
    }))
}

fn select(s: &mut Session, args: &Json) -> Result<Json, String> {
    let cell = cell_arg(s, args)?;
    if !s.doc.sheet.contains(cell) {
        return Err(s.failed(format!(
            "{} is outside the sheet, which is {} rows × {} columns",
            cell.name(),
            s.doc.sheet.rows(),
            s.doc.sheet.cols()
        )));
    }
    s.select(cell);
    Ok(cell_json(s, s.active))
}

fn undo(s: &mut Session, _: &Json) -> Result<Json, String> {
    if !s.undo() {
        return Err(s.failed("There is nothing to undo."));
    }
    let mut out = cell_json(s, s.active);
    out["undone"] = json!(true);
    out["can_undo"] = json!(s.doc.history.can_undo());
    out["dirty"] = json!(s.doc.dirty());
    Ok(out)
}

fn redo(s: &mut Session, _: &Json) -> Result<Json, String> {
    if !s.redo() {
        return Err(s.failed("There is nothing to redo."));
    }
    let mut out = cell_json(s, s.active);
    out["redone"] = json!(true);
    out["can_redo"] = json!(s.doc.history.can_redo());
    out["dirty"] = json!(s.doc.dirty());
    Ok(out)
}

/// Open `path`, for the surface and for the window alike. The agent rule is the caller's.
pub fn open_path(s: &mut Session, path: &Path) -> Result<Json, String> {
    let opened = s.doc.open(path).map_err(|e| s.failed(e))?;
    s.active = Cell::new(0, 0);
    let (filled, errors) = s.doc.sheet.counts();
    let mut line = format!(
        "Opened {} — {} rows × {} columns.",
        s.doc.title(),
        opened.file_rows,
        opened.file_cols
    );
    if errors > 0 {
        line.push_str(&format!(" {errors} cells show an error; the first is {}.", s.doc.sheet.first_error().map(Cell::name).unwrap_or_default()));
    }
    s.done(line);
    Ok(json!({
        "path": opened.path.display().to_string(),
        "file_rows": opened.file_rows,
        "file_cols": opened.file_cols,
        "rows": s.doc.sheet.rows(),
        "cols": s.doc.sheet.cols(),
        "filled_cells": filled,
        "error_cells": errors,
        "dirty": s.doc.dirty(),
    }))
}

fn open(s: &mut Session, args: &Json) -> Result<Json, String> {
    let path = path_arg(s, text(args, "path").unwrap_or_default())?;
    agent_rule::may_read(&path).map_err(|e| s.failed(e))?;
    if s.doc.dirty() {
        return Err(s.failed(format!(
            "{} has unsaved changes, so nothing was opened. Save them (save, save_over) or undo them first.",
            s.doc.title()
        )));
    }
    open_path(s, &path)
}

/// Save to `path`, replacing a file there only when `replace` is set, and say what happened.
pub fn save_path(s: &mut Session, path: &Path, replace: bool) -> Result<Json, String> {
    match s.doc.save(path, replace) {
        Ok(saved) => {
            s.confirm_replace = None;
            s.done(format!(
                "Saved {} — {} rows × {} columns, {} bytes{}.",
                saved.path.display(),
                saved.rows,
                saved.cols,
                saved.bytes,
                if saved.replaced { ", replacing what was there" } else { "" }
            ));
            Ok(json!({
                "path": saved.path.display().to_string(),
                "bytes": saved.bytes,
                "rows": saved.rows,
                "cols": saved.cols,
                "replaced": saved.replaced,
                "dirty": s.doc.dirty(),
            }))
        }
        Err(SaveError::Exists(p)) => Err(s.failed(format!(
            "{} is already there, so nothing was written. Replacing it is save_over.",
            p.display()
        ))),
        Err(SaveError::Failed(why)) => Err(s.failed(why)),
    }
}

fn target(s: &mut Session, args: &Json) -> Result<PathBuf, String> {
    match text(args, "path") {
        Some(raw) if !raw.trim().is_empty() => path_arg(s, raw),
        _ => s.doc.path.clone().ok_or_else(|| s.failed("This sheet has no file yet; give a path ending in .csv.")),
    }
}

fn save(s: &mut Session, args: &Json) -> Result<Json, String> {
    let path = target(s, args)?;
    agent_rule::may_write(&path).map_err(|e| s.failed(e))?;
    save_path(s, &path, false)
}

fn save_over(s: &mut Session, args: &Json) -> Result<Json, String> {
    let path = target(s, args)?;
    agent_rule::may_write(&path).map_err(|e| s.failed(e))?;
    save_path(s, &path, true)
}
