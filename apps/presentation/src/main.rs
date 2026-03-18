//! Yantrik Presentation — standalone app binary.
//!
//! Slide deck editor with themes, layouts, presenter mode, speaker notes, AI assist.

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-presentation");

    let app = PresentationApp::new().unwrap();
    wire(&app);
    app.run().unwrap();
}

fn wire(app: &PresentationApp) {
    // ── Slide management ──
    {
        let weak = app.as_weak();
        app.on_add_slide(move || {
            let Some(ui) = weak.upgrade() else { return };
            let count = ui.get_slide_count() + 1;
            ui.set_slide_count(count);
            tracing::info!("Added slide, total: {count}");
        });
    }

    app.on_delete_slide(|| { tracing::info!("Delete slide"); });
    app.on_duplicate_slide(|| { tracing::info!("Duplicate slide"); });
    app.on_move_slide_up(|| { tracing::info!("Move slide up"); });
    app.on_move_slide_down(|| { tracing::info!("Move slide down"); });

    {
        let weak = app.as_weak();
        app.on_select_slide(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_current_slide_index(idx);
            let slides = ui.get_slides();
            if (idx as usize) < slides.row_count() {
                if let Some(slide) = slides.row_data(idx as usize) {
                    ui.set_current_title(slide.title);
                    ui.set_current_body(slide.body);
                    ui.set_current_notes(slide.notes);
                    ui.set_current_layout(slide.layout);
                }
            }
        });
    }

    // ── Layout / Theme ──
    {
        let weak = app.as_weak();
        app.on_set_layout(move |layout| {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_current_layout(layout);
        });
    }
    app.on_set_theme(|idx| { tracing::info!("Set theme {idx}"); });

    // ── Presenter mode ──
    {
        let weak = app.as_weak();
        app.on_toggle_present(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_is_presenting(!ui.get_is_presenting());
        });
    }

    // ── Navigation ──
    {
        let weak = app.as_weak();
        app.on_next_slide(move || {
            let Some(ui) = weak.upgrade() else { return };
            let idx = ui.get_current_slide_index();
            if idx < ui.get_slide_count() - 1 {
                ui.set_current_slide_index(idx + 1);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_prev_slide(move || {
            let Some(ui) = weak.upgrade() else { return };
            let idx = ui.get_current_slide_index();
            if idx > 0 {
                ui.set_current_slide_index(idx - 1);
            }
        });
    }

    // ── Editing ──
    app.on_title_edited(|_t| {});
    app.on_body_edited(|_b| {});
    app.on_notes_edited(|_n| {});

    // ── Save / Load ──
    app.on_save_presentation(|| { tracing::info!("Save presentation"); });
    app.on_load_presentation(|| { tracing::info!("Load presentation"); });

    // ── Export ──
    app.on_export_pdf(|| { tracing::info!("Export PDF"); });
    app.on_export_markdown(|| { tracing::info!("Export markdown"); });
    app.on_export_outline(|| { tracing::info!("Export outline"); });

    // ── Timer ──
    app.on_toggle_timer(|| { tracing::info!("Toggle timer"); });
    app.on_reset_timer(|| { tracing::info!("Reset timer"); });

    // ── Keyboard ──
    app.on_key_pressed(|key| { tracing::info!("Key pressed: {key}"); });

    // ── Templates ──
    app.on_use_template(|idx| { tracing::info!("Use template {idx}"); });

    // ── Search ──
    app.on_search_slides(|q| { tracing::info!("Search slides: {q}"); });
    app.on_search_next(|| { tracing::info!("Search next"); });

    // ── Transition ──
    app.on_set_transition(|t| { tracing::info!("Set transition {t}"); });

    // ── Undo / Redo ──
    app.on_undo(|| { tracing::info!("Undo"); });
    app.on_redo(|| { tracing::info!("Redo"); });

    // ── Formatting ──
    app.on_generate_notes(|| { tracing::info!("Generate notes"); });
    app.on_format_bold(|| { tracing::info!("Format bold"); });
    app.on_format_italic(|| { tracing::info!("Format italic"); });
    app.on_set_font_size(|size| { tracing::info!("Set font size {size}"); });
    app.on_set_text_color(|color| { tracing::info!("Set text color {color}"); });
    app.on_insert_object(|obj| { tracing::info!("Insert object: {obj}"); });

    // ── AI assist ──
    app.on_pres_ai_generate_deck(|topic| { tracing::info!("AI generate deck: {topic}"); });
    app.on_pres_ai_structure_text(|text| { tracing::info!("AI structure text: {text}"); });
    app.on_pres_ai_generate_all_notes(|| { tracing::info!("AI generate all notes"); });
    app.on_pres_ai_improve_slide(|| { tracing::info!("AI improve slide"); });
    app.on_pres_ai_generate_notes(|| { tracing::info!("AI generate notes"); });
    app.on_pres_ai_simplify(|text| { tracing::info!("AI simplify: {text}"); });
    app.on_pres_ai_split_slide(|| { tracing::info!("AI split slide"); });
    app.on_pres_ai_suggest_layout(|| { tracing::info!("AI suggest layout"); });
    app.on_pres_ai_freeform(|prompt| { tracing::info!("AI freeform: {prompt}"); });
    app.on_pres_ai_apply(|| { tracing::info!("AI apply"); });
    app.on_pres_ai_dismiss(|| { tracing::info!("AI dismiss"); });
    app.on_pres_ai_preview_slide(|idx| { tracing::info!("AI preview slide {idx}"); });
    app.on_pres_ai_regenerate(|| { tracing::info!("AI regenerate"); });

    // ── Template gallery / speaker notes ──
    app.on_pres_open_template_gallery(|| { tracing::info!("Open template gallery"); });
    app.on_pres_select_template(|idx| { tracing::info!("Select template {idx}"); });
    app.on_pres_save_speaker_note(|note| { tracing::info!("Save speaker note"); });
}
