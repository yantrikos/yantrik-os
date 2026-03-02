//! Text Editor wiring — file loading and saving.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use slint::ComponentHandle;

use crate::app_context::AppContext;
use crate::App;

/// Wire text editor callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let editor_path = ctx.editor_file_path.clone();

    // Save file
    let ui_weak = ui.as_weak();
    let ep = editor_path.clone();
    ui.on_editor_save(move || {
        let path_str = ep.borrow().clone();
        if path_str.is_empty() {
            tracing::warn!("No file path set for editor save");
            return;
        }
        if let Some(ui) = ui_weak.upgrade() {
            let content = ui.get_editor_file_content().to_string();
            let path = PathBuf::from(&path_str);
            match std::fs::write(&path, &content) {
                Ok(()) => {
                    tracing::info!(path = %path.display(), "File saved");
                    ui.set_editor_is_modified(false);
                }
                Err(e) => {
                    tracing::error!(path = %path.display(), error = %e, "Failed to save file");
                }
            }
        }
    });

    // Content changed — mark as modified
    let ui_weak = ui.as_weak();
    ui.on_editor_content_changed(move |_text| {
        if let Some(ui) = ui_weak.upgrade() {
            ui.set_editor_is_modified(true);
        }
    });
}

/// Load a file into the editor. Call this when navigating to screen 12.
pub fn load_file(ui: &App, path: &PathBuf, editor_path: &Rc<RefCell<String>>) {
    let path_str = path.display().to_string();
    *editor_path.borrow_mut() = path_str;

    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    ui.set_editor_file_name(name.into());
    ui.set_editor_is_modified(false);

    match std::fs::read_to_string(path) {
        Ok(content) => {
            ui.set_editor_file_content(content.into());
            ui.set_editor_is_readonly(false);
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "Failed to read file for editor");
            ui.set_editor_file_content(format!("Error reading file: {}", e).into());
            ui.set_editor_is_readonly(true);
        }
    }
}
