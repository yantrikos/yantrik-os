//! The window: its callbacks bound to commands, and how it is drawn (render.rs).

pub mod render;

use slint::{ComponentHandle as _, SharedString};
use yantrik_app::cx::Cx;

use crate::command::{Command, WindowAction};
use crate::{TextEditor, TextEditorApp};

/// Binds the window's callbacks. Where a callback carries no value, the window's own fields are
/// read here and the command carries them: the model is told what the window shows, and never
/// reads the window itself. Tabs are named to commands by their id, read from the model as the
/// strip was drawn from it.
pub fn wire(cx: &Cx<TextEditor>, ui: &TextEditorApp) {
    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_action(move |id: SharedString| {
        let Some(ui) = weak.upgrade() else { return };
        if id == "reflow" {
            // The font size decides how many lines fit inside the renderer-safe view (#328), so a
            // font change re-windows the document even while a dialog is up.
            let (font_px, limit_px) = (ui.get_font_pixels(), ui.get_render_limit());
            return run.run(Command::Reflow { font_px, limit_px });
        }
        let path = || ui.get_dialog_path().to_string();
        let replacement = || ui.get_replacement().to_string();
        if let Some(action) = WindowAction::parse(&id, path, replacement) {
            run.run(Command::Window(action));
        }
    });
    let run = cx.clone();
    ui.on_select_tab(move |index: i32| {
        if let Some(id) = run.read(|m| m.tabs.get(index.max(0) as usize).map(|t| t.id)) {
            run.run(Command::SelectTab(id));
        }
    });
    let run = cx.clone();
    ui.on_close_tab(move |index: i32| {
        if let Some(id) = run.read(|m| m.tabs.get(index.max(0) as usize).map(|t| t.id)) {
            run.run(Command::CloseTab(id));
        }
    });
    ui.on_edited(cx.cmd1(|text: SharedString| Command::Edit(text.to_string())));
    ui.on_cursor(cx.cmd1(Command::Cursor));
    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_search(move || {
        if let Some(ui) = weak.upgrade() {
            run.run(Command::Find { query: ui.get_query().to_string(), match_case: ui.get_match_case() });
        }
    });
}
