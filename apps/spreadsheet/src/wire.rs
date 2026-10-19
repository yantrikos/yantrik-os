//! Every callback the window has, connected. Nothing here only logs.

use slint::ComponentHandle;

use crate::address::Cell;
use crate::files;
use crate::render;
use crate::session::Shared;
use crate::SpreadsheetApp;

pub fn wire(ui: &SpreadsheetApp, shared: &Shared) {
    {
        let (weak, s) = (ui.as_weak(), shared.clone());
        ui.on_cell_clicked(move |row, col| {
            let Some(ui) = weak.upgrade() else { return };
            let mut s = s.borrow_mut();
            // Leaving a cell keeps what was typed into it, as clicking away does in any sheet.
            let typed = ui.get_cell_data().to_string();
            s.commit(&typed);
            s.select(Cell::new(row.max(0) as usize, col.max(0) as usize));
            render::render(&ui, &s);
            render::load_formula_bar(&ui, &s);
        });
    }
    {
        let (weak, s) = (ui.as_weak(), shared.clone());
        ui.on_formula_submitted(move |typed, direction| {
            let Some(ui) = weak.upgrade() else { return };
            let mut s = s.borrow_mut();
            s.commit(&typed);
            // A cell that went wrong keeps the selection, so the problem stays in view.
            let went_wrong = s.said.as_ref().is_some_and(|(_, problem)| *problem);
            if !went_wrong {
                s.step(&direction);
            }
            render::render(&ui, &s);
            render::load_formula_bar(&ui, &s);
        });
    }
    {
        let (weak, s) = (ui.as_weak(), shared.clone());
        ui.on_formula_cancelled(move || {
            let Some(ui) = weak.upgrade() else { return };
            let s = s.borrow();
            render::load_formula_bar(&ui, &s);
        });
    }
    {
        let (weak, s) = (ui.as_weak(), shared.clone());
        ui.on_command(move |id| {
            let Some(ui) = weak.upgrade() else { return };
            let mut s = s.borrow_mut();
            // Whatever is in the formula bar is the selected cell's, before anything else runs.
            let typed = ui.get_cell_data().to_string();
            s.commit(&typed);
            match id.as_str() {
                "open" => files::open(&ui, &mut s),
                "save" => files::save(&ui, &mut s),
                "save-as" => files::save_as(&ui, &mut s),
                "undo" => {
                    s.undo();
                }
                "redo" => {
                    s.redo();
                }
                other => {
                    s.failed(format!("The window asked for `{other}`, which this app does not have."));
                }
            }
            files::redraw(&ui, &s);
        });
    }
    {
        let (weak, s) = (ui.as_weak(), shared.clone());
        ui.on_dialog_confirm(move || {
            let Some(ui) = weak.upgrade() else { return };
            files::confirm(&ui, &mut s.borrow_mut());
        });
    }
    {
        let (weak, s) = (ui.as_weak(), shared.clone());
        ui.on_dialog_cancel(move || {
            let Some(ui) = weak.upgrade() else { return };
            files::cancel(&ui, &mut s.borrow_mut());
        });
    }
    {
        let (weak, s) = (ui.as_weak(), shared.clone());
        ui.on_dialog_discard(move || {
            let Some(ui) = weak.upgrade() else { return };
            files::discard(&ui, &mut s.borrow_mut());
        });
    }
    {
        let (weak, s) = (ui.as_weak(), shared.clone());
        ui.window().on_close_requested(move || {
            let Some(ui) = weak.upgrade() else { return slint::CloseRequestResponse::HideWindow };
            let Ok(mut s) = s.try_borrow_mut() else { return slint::CloseRequestResponse::KeepWindowShown };
            let typed = ui.get_cell_data().to_string();
            s.commit(&typed);
            if files::may_close(&ui, &mut s) {
                slint::CloseRequestResponse::HideWindow
            } else {
                render::render(&ui, &s);
                slint::CloseRequestResponse::KeepWindowShown
            }
        });
    }
}
