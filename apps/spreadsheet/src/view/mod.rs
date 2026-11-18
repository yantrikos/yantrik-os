//! The window: its callbacks bound to commands, and how it is drawn (render.rs).
//!
//! Every callback that carries the formula bar's text sends it with the command, and the model is
//! told what the window shows and does not read the window. The one exception is a logout
//! (`Model::bar`, used by `update::closing::logging_out`): the bar has no change callback, the
//! logout must be answered in a single command, and text typed and not entered has to reach the
//! draft in it. The model keeps a weak handle on the window for that read and nothing else; routing
//! it through a hook in the framework would need a round trip a logout does not have time for.
//!
//! In a window drawn for a Mind (Mind View), typing into the bar is refused; the notice and the mask in the bar are put up
//! first (`Command::Masked`), so what was typed does not stay in the field.

pub mod render;

use slint::{ComponentHandle as _, SharedString};
use yantrik_app::cx::Cx;

use crate::command::Command;
use crate::mind_view;
use crate::{SpreadsheetApp, YSheets};

/// What the formula bar holds now.
fn typed(ui: &SpreadsheetApp) -> String {
    ui.get_cell_data().to_string()
}

/// Runs `command` for the person at the window. One a Mind's window would refuse is preceded by
/// the notice and the mask, which the refusal itself cannot draw.
fn raise(cx: &Cx<YSheets>, command: Command) {
    if cx.read(|m| m.for_mind && mind_view::refuses(m, &command)) {
        cx.run(Command::Masked);
    }
    cx.run(command);
}

/// Binds the window's callbacks.
pub fn wire(cx: &Cx<YSheets>, ui: &SpreadsheetApp) {
    // The model keeps a handle on the window for the one thing it cannot be told in time: what the
    // formula bar holds when the session ends (`Model::bar`).
    cx.read(|m| *m.window.borrow_mut() = Some(ui.as_weak()));
    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_cell_clicked(move |row, col| {
        if let Some(ui) = weak.upgrade() {
            raise(&run, Command::Cell { row, col, typed: typed(&ui) });
        }
    });

    ui.on_formula_submitted({
        let run = cx.clone();
        move |typed: SharedString, direction: SharedString| {
            raise(&run, Command::Formula { typed: typed.to_string(), direction: direction.to_string() });
        }
    });

    ui.on_formula_cancelled(cx.cmd(|| Command::Cancelled));

    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_command(move |id: SharedString| {
        if let Some(ui) = weak.upgrade() {
            raise(&run, Command::Header { id: id.to_string(), typed: typed(&ui) });
        }
    });

    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_dialog_confirm(move || {
        if let Some(ui) = weak.upgrade() {
            raise(&run, Command::Confirm { path: ui.get_dialog_path().to_string() });
        }
    });
    ui.on_dialog_cancel(cx.cmd(|| Command::Cancel));
    ui.on_dialog_discard(cx.cmd(|| Command::Discard));
}
