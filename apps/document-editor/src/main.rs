//! Yantrik Document Editor — standalone app binary.
//!
//! Rich document editing with comments, track changes, version history, AI assist.

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-document-editor");

    let app = DocumentEditorApp::new().unwrap();
    wire(&app);
    app.run().unwrap();
}

fn wire(app: &DocumentEditorApp) {
    // ── Document operations ──
    {
        let weak = app.as_weak();
        app.on_doc_new(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_doc_title("Untitled".into());
            ui.set_doc_content("".into());
            ui.set_doc_is_modified(false);
            ui.set_doc_word_count(0);
            ui.set_doc_char_count(0);
        });
    }

    {
        let weak = app.as_weak();
        app.on_doc_save(move || {
            let Some(ui) = weak.upgrade() else { return };
            let path = ui.get_doc_file_path().to_string();
            if path.is_empty() {
                tracing::info!("No file path set");
                return;
            }
            let content = ui.get_doc_content().to_string();
            match std::fs::write(&path, &content) {
                Ok(_) => {
                    ui.set_doc_is_modified(false);
                    ui.set_doc_save_status("Saved".into());
                    tracing::info!("Saved document to {path}");
                }
                Err(e) => {
                    ui.set_doc_save_status(format!("Save failed: {e}").into());
                }
            }
        });
    }

    app.on_doc_open(|| { tracing::info!("Open document"); });

    // ── Content changed ──
    {
        let weak = app.as_weak();
        app.on_doc_content_changed(move |content| {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_doc_is_modified(true);
            let text = content.to_string();
            ui.set_doc_word_count(text.split_whitespace().count() as i32);
            ui.set_doc_char_count(text.len() as i32);
        });
    }

    // ── Formatting ──
    app.on_doc_format_bold(|| { tracing::info!("Format bold"); });
    app.on_doc_format_italic(|| { tracing::info!("Format italic"); });
    app.on_doc_format_underline(|| { tracing::info!("Format underline"); });
    app.on_doc_format_heading(|level| { tracing::info!("Format heading level {level}"); });
    app.on_doc_format_bullet(|| { tracing::info!("Format bullet"); });
    app.on_doc_format_checklist(|| { tracing::info!("Format checklist"); });
    app.on_doc_format_quote(|| { tracing::info!("Format quote"); });
    app.on_doc_format_code(|| { tracing::info!("Format code"); });
    app.on_doc_format_divider(|| { tracing::info!("Format divider"); });
    app.on_doc_format_strikethrough(|| { tracing::info!("Format strikethrough"); });
    app.on_doc_format_highlight(|| { tracing::info!("Format highlight"); });
    app.on_doc_format_link(|url| { tracing::info!("Format link: {url}"); });
    app.on_doc_format_inline_code(|| { tracing::info!("Format inline code"); });

    // ── Undo / Redo ──
    app.on_doc_undo(|| { tracing::info!("Undo"); });
    app.on_doc_redo(|| { tracing::info!("Redo"); });

    // ── Find / Replace ──
    app.on_doc_find_next(|| { tracing::info!("Find next"); });
    app.on_doc_find_prev(|| { tracing::info!("Find prev"); });
    app.on_doc_replace_one(|| { tracing::info!("Replace one"); });
    app.on_doc_replace_all(|| { tracing::info!("Replace all"); });

    // ── Heading navigation ──
    app.on_doc_heading_clicked(|idx| { tracing::info!("Heading clicked: {idx}"); });

    // ── Import / Export ──
    app.on_doc_import_md(|| { tracing::info!("Import markdown"); });
    app.on_doc_export_md(|| { tracing::info!("Export markdown"); });
    app.on_doc_export_pdf(|| { tracing::info!("Export PDF"); });
    app.on_doc_export_html(|| { tracing::info!("Export HTML"); });

    // ── Comments ──
    app.on_doc_add_comment(|text| { tracing::info!("Add comment: {text}"); });
    app.on_doc_delete_comment(|id| { tracing::info!("Delete comment {id}"); });
    app.on_doc_resolve_comment(|id| { tracing::info!("Resolve comment {id}"); });

    // ── Track changes ──
    app.on_doc_toggle_track_changes(|| { tracing::info!("Toggle track changes"); });
    app.on_doc_accept_change(|id| { tracing::info!("Accept change {id}"); });
    app.on_doc_reject_change(|id| { tracing::info!("Reject change {id}"); });
    app.on_doc_accept_all_changes(|| { tracing::info!("Accept all changes"); });
    app.on_doc_reject_all_changes(|| { tracing::info!("Reject all changes"); });

    // ── Version history ──
    app.on_doc_save_version(|label| { tracing::info!("Save version: {label}"); });
    app.on_doc_list_versions(|| { tracing::info!("List versions"); });
    app.on_doc_restore_version(|idx| { tracing::info!("Restore version {idx}"); });

    // ── Tables ──
    app.on_doc_insert_table(|rows, cols| { tracing::info!("Insert table {rows}x{cols}"); });
    app.on_doc_add_table_row(|| { tracing::info!("Add table row"); });
    app.on_doc_add_table_col(|| { tracing::info!("Add table col"); });

    // ── Insert ──
    app.on_doc_insert_toc(|| { tracing::info!("Insert TOC"); });
    app.on_doc_insert_footnote(|| { tracing::info!("Insert footnote"); });
    app.on_doc_insert_image(|path| { tracing::info!("Insert image: {path}"); });

    // ── Templates ──
    app.on_doc_use_template(|idx| { tracing::info!("Use template {idx}"); });

    // ── Page layout ──
    app.on_doc_set_page_layout(|size| { tracing::info!("Set page layout: {size}"); });
    app.on_doc_print_preview(|| { tracing::info!("Print preview"); });

    // ── AI assist ──
    app.on_doc_ai_submit(|prompt| { tracing::info!("AI submit: {prompt} (standalone mode)"); });
    app.on_doc_ai_apply(|| { tracing::info!("AI apply"); });
    app.on_doc_ai_dismiss(|| { tracing::info!("AI dismiss"); });
    app.on_doc_ai_draft(|topic| { tracing::info!("AI draft: {topic}"); });
    app.on_doc_ai_summarize(|| { tracing::info!("AI summarize"); });
    app.on_doc_ai_improve(|| { tracing::info!("AI improve"); });
    app.on_doc_ai_translate(|lang| { tracing::info!("AI translate to {lang}"); });
    app.on_doc_ai_insights(|| { tracing::info!("AI insights"); });
}
