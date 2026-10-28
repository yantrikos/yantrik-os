//! Putting a sheet in front: a file read in a job, and the sheet it makes worked out there too.
//!
//! Nothing is replaced until the file has been read: a path that is not a CSV costs nothing, the
//! sheet on screen is untouched and the reason is named. A sheet with unsaved changes is never
//! replaced unasked: the window asks first (the question), and the surface and a launch refuse. If
//! the sheet was changed while the file was being read, the file is dropped rather than the changes.

use std::path::PathBuf;

use serde_json::json;
use yantrik_app::documents::Source;
use yantrik_app::effects::Effects;

use super::{cells, drafts, say, surface};
use crate::address::Cell;
use crate::agent_rule::HIDDEN;
use crate::book::Book;
use crate::caller::Caller;
use crate::command::{Cause, Command};
use crate::files::open::{may_open, open_sheet, OpenedSheet};
use crate::model::{self, Cue, Model, Redraw, Work};
use crate::YSheets;

/// Said when a job ended without an answer (it panicked, or the pool refused it).
const JOB_LOST: &str = "The file could not be read; try opening it again.";

/// What a refusal to replace a sheet with unsaved changes says, for the window, a launch and the
/// surface alike.
pub fn would_throw_away(title: &str) -> String {
    format!("{title} has unsaved changes, so nothing was opened. Save them (save, save_over) or undo them first.")
}

/// Opens the file at `path`, read in a job.
pub fn start(m: &mut Model, fx: &mut Effects<YSheets>, path: PathBuf, then: Cause, caller: Caller) {
    m.busy = Some(Work::Open);
    say(m, "Opening file…");
    let agent = caller.agent;
    // A call is refused for unsaved changes only after the rule for its path has had its say, as it
    // always was; the rule reads the disk, so both are decided in the job.
    let unsaved = matches!(then, Cause::Surface(_)).then(|| m.modified().then(|| would_throw_away(&m.title()))).flatten();
    let for_mind = m.for_mind;
    let lost = Command::Opened { then: then.clone(), result: Err(JOB_LOST.into()) };
    fx.spawn_or(lost, move |_| {
        let result = may_open(&path, agent)
            .and_then(|()| unsaved.map_or(Ok(()), Err))
            .and_then(|()| open_sheet(&path, agent))
            .map(|mut opened| {
                let mask = crate::view::render::Mask::of(for_mind, opened.file.hidden);
                opened.cells = Some(crate::view::render::rows_of(mask, &opened.sheet));
                Box::new(opened)
            });
        Command::Opened { then, result }
    });
}

/// A file a launch asked to open, once ySheets is free.
pub fn launch(m: &mut Model, fx: &mut Effects<YSheets>, path: PathBuf) {
    m.queue = Some(path);
    next(m, fx);
}

/// The file a launch asked for, when ySheets is free and nothing is asked. A launch is an `open`:
/// with unsaved changes on screen it is refused in the notice and the draft stays, as a second
/// launch handing its file to the running window always was.
pub fn next(m: &mut Model, fx: &mut Effects<YSheets>) {
    if m.busy.is_some() || m.dialog_up() || m.picking || m.lingering {
        return;
    }
    if let Some(path) = m.queue.take() {
        if m.modified() {
            let title = m.title();
            return say(m, would_throw_away(&title));
        }
        start(m, fx, path, Cause::Launch, Caller::person());
    }
}

/// An open landed.
pub fn landed(m: &mut Model, fx: &mut Effects<YSheets>, then: Cause, result: Result<Box<OpenedSheet>, String>) {
    if m.busy == Some(Work::Open) {
        m.busy = None;
    }
    m.notice.clear();
    let discarding = std::mem::take(&mut m.discarding);
    match result {
        Ok(_) if m.lingering => {
            let sentence = "ySheets' window is closed while it finishes a save; launch it again to bring the window back.";
            refuse(m, fx, &then, sentence.into());
        }
        Ok(_) if m.modified() && !discarding => {
            // Typed into while the file was read: the changes win, and the file is not opened.
            let title = m.title();
            refuse(m, fx, &then, would_throw_away(&title));
        }
        Ok(opened) => put_in_front(m, fx, *opened, then),
        Err(e) => refuse(m, fx, &then, e),
    }
    next(m, fx);
}

/// Said, to the window and to the call that asked.
pub fn refuse(m: &mut Model, fx: &mut Effects<YSheets>, then: &Cause, sentence: String) {
    match then {
        Cause::Surface(_) => surface::refuse(m, fx, sentence),
        Cause::Window | Cause::Launch => {
            cells::failed(m, sentence);
        }
    }
}

/// The sheet in front closed: unwatched, its draft gone, its view and the table's entry too.
fn close_current(m: &mut Model, fx: &mut Effects<YSheets>) {
    let name = m.doc.draft.clone();
    if let Some(d) = m.documents.close_document(m.doc.id) {
        fx.unwatch(&d.source.key());
    }
    drafts::forget(m, fx, &name);
    m.reread_owed = false;
}

/// The sheet `opened` in front, watched from what was read, and the call that asked for it
/// answered from what it did.
pub fn put_in_front(m: &mut Model, fx: &mut Effects<YSheets>, opened: OpenedSheet, then: Cause) {
    close_current(m, fx);
    let draft = m.next_draft();
    let OpenedSheet { file, sheet, file_rows, file_cols, baseline, cells } = opened;
    let id = m.documents.open(Source::File(file.path.clone()), Book::opened(sheet), Some(baseline));
    let text = file.text;
    if let Some((key, path, seen)) = m.documents.get(id).and_then(|d| d.watch_entry()) {
        fx.watch(&key, path, seen);
    }
    m.doc = model::place(&mut m.documents, id, Some(text), draft, file.hidden, file.inode);
    arrived(m);
    *m.prebuilt.get_mut() = cells.map(|rows| crate::model::Prebuilt { rev: m.sheet().rev(), rows });
    let (filled, errors) = m.sheet().counts();
    let mut line = format!("Opened {} — {file_rows} rows × {file_cols} columns.", m.title());
    if errors > 0 {
        let first = m.sheet().first_error().map(Cell::name).unwrap_or_default();
        line.push_str(&format!(" {errors} cells show an error; the first is {first}."));
    }
    cells::done(m, line);
    match &then {
        Cause::Surface(caller) => {
            // A file an agent is not shown (a link out of the places it may name) is opened for the
            // window, and the agent learns nothing of it.
            let answer = if caller.agent && m.doc.hidden {
                json!({ "opened": HIDDEN })
            } else {
                json!({
                    "path": file.path.display().to_string(),
                    "file_rows": file_rows,
                    "file_cols": file_cols,
                    "rows": m.sheet().rows(),
                    "cols": m.sheet().cols(),
                    "filled_cells": filled,
                    "error_cells": errors,
                    "dirty": m.modified(),
                })
            };
            surface::answer(m, fx, answer);
            // `answer` clears the notice; the line above stays on the status line.
        }
        Cause::Launch => {
            // Last, because a successful open clears the notice and a draft held aside is the one
            // thing a person opening a file still needs to be told.
            if let Some(said) = m.held_aside.clone() {
                m.notice.set(said);
            }
        }
        Cause::Window => {}
    }
}

/// The cues every new sheet in front starts with: A1 selected, the grid drawn whole.
fn arrived(m: &mut Model) {
    m.active = Cell::new(0, 0);
    m.redraw = Redraw::All;
    m.cues.push(Cue::LoadField);
    m.watch = None;
    m.moves += 1;
}
