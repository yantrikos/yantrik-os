//! The window: its callbacks bound to commands, and how it is drawn.

pub mod listing;
pub mod preview;
pub mod render;

use slint::{ComponentHandle as _, SharedString};
use yantrik_app::app::WindowId;
use yantrik_app::cx::Cx;

use crate::command::{Command, WindowAction};
use crate::{Notes, NotesApp};

/// Binds a window's callbacks. Every command says which window it came from, so it acts on that
/// window's note and view. Where a callback carries no value, the window's own fields are read
/// here and the command carries them: the model is told what the window shows, and never reads
/// the window itself.
pub fn wire(cx: &Cx<Notes>, ui: &NotesApp, window: WindowId) {
    let here = move |command: Command| Command::From { window, command: Box::new(command) };
    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_action(move |id: SharedString| {
        let path = || weak.upgrade().map(|ui| ui.get_path().to_string()).unwrap_or_default();
        if let Some(action) = WindowAction::parse(&id, path) {
            run.run(here(Command::Window(action)));
        }
    });
    ui.on_choose(cx.cmd1(move |id: SharedString| here(Command::Window(WindowAction::open(&id)))));
    ui.on_edited(cx.cmd1(move |text: SharedString| here(Command::Edit(text.to_string()))));
    ui.on_activated(cx.cmd(move || here(Command::Activated)));
    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_filter(move || {
        if let Some(ui) = weak.upgrade() {
            let (folder, query) = (ui.get_folder().to_string(), ui.get_query().to_string());
            run.run(here(Command::Filter { folder, query }));
        }
    });
    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_metadata(move || {
        if let Some(ui) = weak.upgrade() {
            let (notebook, tags) = (ui.get_notebook().to_string(), ui.get_tags().to_string());
            run.run(here(Command::Metadata { notebook, tags }));
        }
    });
    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_view_changed(move || {
        if let Some(ui) = weak.upgrade() {
            run.run(here(Command::ViewSynced { focus_mode: ui.get_focus_mode(), dialog: ui.get_dialog() }));
        }
    });
}
