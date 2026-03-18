//! Yantrik Text Editor — standalone app binary.
//!
//! Multi-tab code/text editor with file open/save, find/replace, and AI assist.

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-text-editor");

    let app = TextEditorApp::new().unwrap();
    wire(&app);
    app.run().unwrap();
}

fn wire(app: &TextEditorApp) {
    // ── Save file ──
    {
        let weak = app.as_weak();
        app.on_save_file(move || {
            let Some(ui) = weak.upgrade() else { return };
            let content = ui.get_file_content().to_string();
            let name = ui.get_file_name().to_string();
            if name.is_empty() || name == "untitled" {
                tracing::info!("No file path set; use Save As");
                return;
            }
            match std::fs::write(&name, &content) {
                Ok(_) => {
                    ui.set_is_modified(false);
                    tracing::info!("Saved {name}");
                }
                Err(e) => tracing::error!("Save failed: {e}"),
            }
        });
    }

    // ── Save file as ──
    {
        let weak = app.as_weak();
        app.on_save_file_as(move |dir, filename| {
            let Some(ui) = weak.upgrade() else { return };
            let path = format!("{}/{}", dir.to_string().trim_end_matches('/'), filename);
            let content = ui.get_file_content().to_string();
            match std::fs::write(&path, &content) {
                Ok(_) => {
                    ui.set_file_name(path.into());
                    ui.set_is_modified(false);
                    ui.set_show_save_dialog(false);
                    tracing::info!("Saved as {}", ui.get_file_name());
                }
                Err(e) => {
                    ui.set_save_error(format!("Save failed: {e}").into());
                }
            }
        });
    }

    // ── Content changed ──
    {
        let weak = app.as_weak();
        app.on_content_changed(move |content| {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_is_modified(true);
            let lines = content.as_str().lines().count().max(1);
            ui.set_total_lines(lines as i32);
            let line_nums: String = (1..=lines).map(|n| format!("{:>4}", n)).collect::<Vec<_>>().join("\n");
            ui.set_line_numbers_text(line_nums.into());
        });
    }

    // ── Tab management ──
    app.on_switch_tab(|idx| { tracing::info!("Switch to tab {idx}"); });
    app.on_new_tab(|| { tracing::info!("New tab requested"); });
    app.on_close_tab(|idx| { tracing::info!("Close tab {idx}"); });

    // ── Go-to-line ──
    app.on_goto_line(|line| { tracing::info!("Go to line {line}"); });

    // ── Find & Replace ──
    app.on_find_next(|| { tracing::info!("Find next"); });
    app.on_find_prev(|| { tracing::info!("Find prev"); });
    app.on_replace_current(|| { tracing::info!("Replace current"); });
    app.on_replace_all(|| { tracing::info!("Replace all"); });
    app.on_find_query_changed(|_q| {});

    // ── AI assist ──
    app.on_ai_request(|prompt| { tracing::info!("AI request: {prompt} (standalone mode)"); });
    app.on_ai_insert(|| { tracing::info!("AI insert"); });
    app.on_ai_dismiss(|| { tracing::info!("AI dismiss"); });

    // ── Encoding / line ending / minimap ──
    app.on_editor_set_encoding(|enc| { tracing::info!("Set encoding: {enc}"); });
    app.on_editor_set_line_ending(|le| { tracing::info!("Set line ending: {le}"); });
    app.on_editor_toggle_minimap(|| { tracing::info!("Toggle minimap"); });
}
