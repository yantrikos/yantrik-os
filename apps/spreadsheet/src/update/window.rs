//! The window's own actions: a click on a cell, the formula bar, the header's buttons and keys, and
//! the dialogs. While a dialog is up only its answers are taken.
//!
//! Every one that carries the formula bar's text commits it to the selected cell first, as
//! clicking away does in any sheet: whatever is in the bar is the selected cell's.

use std::path::PathBuf;

use yantrik_app::effects::Effects;
use yantrik_app::for_mind::READ_ONLY;

use super::{cells, closing, opening, picking, saving, say};
use crate::address::Cell;
use crate::command::Cause;
use crate::files::names::resolve;
use crate::model::{Cue, Dialog, Model, Pending, Said};
use crate::YSheets;

/// A cell was clicked: what was typed in the one being left stays in it.
pub fn clicked(m: &mut Model, _: &mut Effects<YSheets>, row: i32, col: i32, typed: &str) {
    if m.dialog_up() {
        return;
    }
    cells::commit(m, typed);
    cells::moved(m, Cell::new(row.max(0) as usize, col.max(0) as usize));
}

/// Enter, Tab or an arrow in the formula bar.
pub fn submitted(m: &mut Model, _: &mut Effects<YSheets>, typed: &str, direction: &str) {
    if m.dialog_up() {
        return;
    }
    cells::commit(m, typed);
    // The selection moves at once. If the cell typed into comes out an error, the sheet worked
    // out, the selection comes back to it unless the person has moved on (`update::solved`).
    cells::step(m, direction);
    if let Some(watch) = m.watch.as_mut() {
        watch.moves = m.moves;
    }
}

/// One of the header's buttons or keys.
pub fn header(m: &mut Model, fx: &mut Effects<YSheets>, id: &str, typed: &str) {
    // The window handing over what the formula bar holds, so a close does not lose it.
    if id == "commit" {
        if !m.dialog_up() {
            cells::commit(m, typed);
        }
        if std::mem::take(&mut m.closing_after_commit) {
            closing::window(m, fx);
        }
        return;
    }
    if m.dialog_up() {
        return;
    }
    cells::commit(m, typed);
    match id {
        "open" => open(m, fx),
        "save" => save(m, fx),
        "save-as" => picking::pick(m, fx, true),
        "undo" => {
            cells::undo(m);
        }
        "redo" => {
            cells::redo(m);
        }
        other => {
            cells::failed(m, format!("The window asked for `{other}`, which this app does not have."));
        }
    }
    // After a header command nothing is being typed: the bar shows the selected cell.
    m.cues.push(Cue::LoadField);
}

/// The header's Open: the unsaved-changes question first, when there is anything unsaved.
fn open(m: &mut Model, fx: &mut Effects<YSheets>) {
    if m.busy.is_some() {
        return say(m, FINISH_FIRST);
    }
    if m.modified() {
        return closing::ask(m, Pending::Open);
    }
    picking::pick(m, fx, false);
}

/// The header's Save: over the sheet's own file, or the picker when it has none, or when its file
/// is gone or was changed by another program (then a name is chosen for it).
fn save(m: &mut Model, fx: &mut Effects<YSheets>) {
    if m.busy.is_some() {
        return say(m, FINISH_FIRST);
    }
    match own_file(m) {
        Some(path) => saving::start_window(m, fx, path, false, false),
        None => picking::pick(m, fx, true),
    }
}

/// The sheet's own file, when Save can write to it: it has one, it is there, and nothing but this
/// window has changed it.
pub fn own_file(m: &Model) -> Option<PathBuf> {
    use yantrik_app::documents::OutsideState;
    match (m.path(), &m.d().outside) {
        (Some(path), OutsideState::Clean | OutsideState::Changed { .. }) => Some(path.to_path_buf()),
        _ => None,
    }
}

const FINISH_FIRST: &str = "Finish the current file operation first.";

/// The dialog's main button: Open or Save for a typed path, Save for the unsaved-changes question.
pub fn confirm(m: &mut Model, fx: &mut Effects<YSheets>, path: String) {
    let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let typed = resolve(&path, &base);
    match m.dialog {
        Dialog::OpenPath => {
            m.dialog = Dialog::None;
            opening::start(m, fx, typed, Cause::Window, crate::caller::Caller::person());
        }
        Dialog::SavePath => {
            // A typed path is not the picker: nobody has asked about replacing what is there. The
            // first Save finds out; the second, for the same path, is the answer.
            let replace = m.confirm_replace.as_deref() == Some(typed.as_path());
            let prompted = m.asking();
            saving::start_window(m, fx, typed, replace, prompted);
        }
        // The question's *Save*. A sheet that cannot be written to its own file is asked where to go.
        Dialog::None if m.asking() => match own_file(m) {
            Some(own) => saving::start_window(m, fx, own, false, true),
            None => picking::pick(m, fx, true),
        },
        Dialog::None => {}
    }
}

/// Cancel: the dialog and the question go, and nothing changes.
pub fn cancel(m: &mut Model) {
    m.dialog = Dialog::None;
    m.pending = None;
    m.confirm_replace = None;
    m.discarding = false;
    m.said = Some(Said { text: "Nothing changed.".into(), problem: false, quotes_cells: false });
}

/// Discard: the unsaved changes are thrown away on purpose, and what waited on the question goes on.
pub fn discard(m: &mut Model, fx: &mut Effects<YSheets>) {
    match m.pending.take() {
        Some(Pending::Open) => {
            m.discarding = true;
            picking::pick(m, fx, false);
        }
        Some(Pending::Close) => closing::close_discarding(m, fx),
        None => {}
    }
}

/// A Mind View window was typed into. The typing is refused next (`mind_view::refuses`); this is
/// what the window shows meanwhile: the notice, and the mask in the formula bar, not what was typed.
pub fn masked(m: &mut Model) {
    say(m, READ_ONLY);
    m.said = Some(Said { text: READ_ONLY.into(), problem: false, quotes_cells: false });
    m.cues.push(Cue::LoadField);
}
