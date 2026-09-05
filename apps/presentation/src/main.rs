//! Yantrik Presentation — standalone app binary.
//!
//! Slide deck editor with themes, layouts, presenter mode, speaker notes, AI assist.

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::rc::Rc;
use yantrik_app_runtime::prelude::*;

slint::include_modules!();

fn main() {
    init_tracing("yantrik-presentation");

    // One window per app: a second launch defers to the running one (the shell focuses it).
    let Some(_instance) = instance::claim("presentation") else { return };

    let app = PresentationApp::new().unwrap();

    // Same dark/accent choice as the shell, read from the shell's settings file.
    let theme = theme::load();
    app.global::<ThemeMode>().set_dark(theme.dark);
    app.global::<AccentPreset>().set_index(theme.accent_index);

    wire(&app);
    app.run().unwrap();
}

/// A deck always has at least one slide; a brand new one is a title slide.
fn new_slide(number: i32, layout: i32) -> SlideData {
    SlideData {
        title: if number == 1 { "Title Slide".into() } else { format!("Slide {number}").into() },
        body: if number == 1 { "Click to add subtitle".into() } else { "".into() },
        notes: "".into(),
        layout,
        slide_number: number,
    }
}

/// Slide numbers are positions, so they are re-derived after every reorder or removal.
fn renumber(model: &Rc<VecModel<SlideData>>) {
    for i in 0..model.row_count() {
        if let Some(mut s) = model.row_data(i) {
            s.slide_number = i as i32 + 1;
            model.set_row_data(i, s);
        }
    }
}

/// The canvas edits `current-*` in place, so those values have to be written back into the
/// model before the selection moves or they are lost.
fn commit_current(ui: &PresentationApp, model: &Rc<VecModel<SlideData>>) {
    let idx = ui.get_current_slide_index();
    if idx < 0 || idx as usize >= model.row_count() {
        return;
    }
    if let Some(mut s) = model.row_data(idx as usize) {
        s.title = ui.get_current_title();
        s.body = ui.get_current_body();
        s.notes = ui.get_current_notes();
        s.layout = ui.get_current_layout();
        model.set_row_data(idx as usize, s);
    }
}

fn show(ui: &PresentationApp, model: &Rc<VecModel<SlideData>>, idx: i32) {
    let idx = idx.clamp(0, model.row_count() as i32 - 1);
    ui.set_current_slide_index(idx);
    ui.set_slide_count(model.row_count() as i32);
    if let Some(s) = model.row_data(idx as usize) {
        ui.set_current_title(s.title);
        ui.set_current_body(s.body);
        ui.set_current_notes(s.notes);
        ui.set_current_layout(s.layout);
    }
}

fn wire(app: &PresentationApp) {
    // ── The deck ──
    //
    // The VecModel is the source of truth: the panel, the counter and the canvas all read it.
    let slides: Rc<VecModel<SlideData>> = Rc::new(VecModel::from(vec![new_slide(1, 0)]));
    app.set_slides(ModelRc::from(slides.clone()));
    show(app, &slides, 0);

    {
        let weak = app.as_weak();
        let model = slides.clone();
        app.on_add_slide(move || {
            let Some(ui) = weak.upgrade() else { return };
            commit_current(&ui, &model);
            let at = (ui.get_current_slide_index() + 1).max(0) as usize;
            let at = at.min(model.row_count());
            model.insert(at, new_slide(at as i32 + 1, 1));
            renumber(&model);
            show(&ui, &model, at as i32);
            tracing::info!("Added slide, total: {}", model.row_count());
        });
    }

    {
        let weak = app.as_weak();
        let model = slides.clone();
        app.on_delete_slide(move || {
            let Some(ui) = weak.upgrade() else { return };
            // A deck with no slides has nothing to show, so the last one stays.
            if model.row_count() <= 1 {
                tracing::info!("Refusing to delete the only slide");
                return;
            }
            let idx = ui.get_current_slide_index().max(0) as usize;
            if idx < model.row_count() {
                model.remove(idx);
                renumber(&model);
                show(&ui, &model, idx as i32);
            }
        });
    }

    {
        let weak = app.as_weak();
        let model = slides.clone();
        app.on_duplicate_slide(move || {
            let Some(ui) = weak.upgrade() else { return };
            commit_current(&ui, &model);
            let idx = ui.get_current_slide_index().max(0) as usize;
            if let Some(s) = model.row_data(idx) {
                model.insert(idx + 1, s);
                renumber(&model);
                show(&ui, &model, idx as i32 + 1);
            }
        });
    }

    {
        let weak = app.as_weak();
        let model = slides.clone();
        app.on_move_slide_up(move || {
            let Some(ui) = weak.upgrade() else { return };
            commit_current(&ui, &model);
            let idx = ui.get_current_slide_index();
            if idx > 0 {
                let i = idx as usize;
                if let (Some(a), Some(b)) = (model.row_data(i - 1), model.row_data(i)) {
                    model.set_row_data(i - 1, b);
                    model.set_row_data(i, a);
                    renumber(&model);
                    show(&ui, &model, idx - 1);
                }
            }
        });
    }

    {
        let weak = app.as_weak();
        let model = slides.clone();
        app.on_move_slide_down(move || {
            let Some(ui) = weak.upgrade() else { return };
            commit_current(&ui, &model);
            let idx = ui.get_current_slide_index();
            if (idx as usize) + 1 < model.row_count() {
                let i = idx as usize;
                if let (Some(a), Some(b)) = (model.row_data(i), model.row_data(i + 1)) {
                    model.set_row_data(i, b);
                    model.set_row_data(i + 1, a);
                    renumber(&model);
                    show(&ui, &model, idx + 1);
                }
            }
        });
    }

    {
        let weak = app.as_weak();
        let model = slides.clone();
        app.on_select_slide(move |idx| {
            let Some(ui) = weak.upgrade() else { return };
            commit_current(&ui, &model);
            show(&ui, &model, idx);
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
