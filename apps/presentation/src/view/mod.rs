//! The window: its callbacks bound to commands, and how it is drawn (render.rs).

pub mod render;

use slint::{ComponentHandle as _, SharedString};
use yantrik_app::cx::Cx;

use crate::command::{Command, WindowAction};
use crate::deck::Edits;
use crate::{PresentationApp, YPresent};

/// What the canvas holds now: the live text of the slide on screen, which the screen binds
/// two-way to `current-title` and its neighbours. The model is told it with every command that
/// could need it, and never reads the window itself.
pub(crate) fn canvas(ui: &PresentationApp) -> Edits {
    Edits {
        title: ui.get_current_title().to_string(),
        body: ui.get_current_body().to_string(),
        notes: ui.get_current_notes().to_string(),
        layout: ui.get_current_layout(),
    }
}

/// A button: the action it presses, with the canvas as it was pressed.
fn button(cx: &Cx<YPresent>, ui: &PresentationApp, action: WindowAction) -> impl Fn() + 'static {
    let (run, weak) = (cx.clone(), ui.as_weak());
    move || {
        if let Some(ui) = weak.upgrade() {
            run.run(Command::Window { action: action.clone(), canvas: canvas(&ui) });
        }
    }
}

/// A callback that carries one value: the action it presses, with the canvas as it was pressed.
fn press<T: 'static>(
    cx: &Cx<YPresent>,
    ui: &PresentationApp,
    action: impl Fn(T) -> WindowAction + 'static,
) -> impl Fn(T) + 'static {
    let (run, weak) = (cx.clone(), ui.as_weak());
    move |value| {
        if let Some(ui) = weak.upgrade() {
            run.run(Command::Window { action: action(value), canvas: canvas(&ui) });
        }
    }
}

/// Binds the window's callbacks.
pub fn wire(cx: &Cx<YPresent>, ui: &PresentationApp) {
    // The deck.
    ui.on_add_slide(button(cx, ui, WindowAction::AddSlide));
    ui.on_delete_slide(button(cx, ui, WindowAction::DeleteSlide));
    ui.on_duplicate_slide(button(cx, ui, WindowAction::DuplicateSlide));
    ui.on_move_slide_up(button(cx, ui, WindowAction::MoveUp));
    ui.on_move_slide_down(button(cx, ui, WindowAction::MoveDown));
    ui.on_select_slide(press(cx, ui, |index: i32| WindowAction::Select(i64::from(index))));

    // Navigation. Every path ends in the one move that commits the canvas first.
    ui.on_next_slide(button(cx, ui, WindowAction::Next));
    ui.on_prev_slide(button(cx, ui, WindowAction::Previous));
    let (run, weak) = (cx.clone(), ui.as_weak());
    ui.on_key_pressed(move |key: SharedString| {
        let action = match key.as_str() {
            "next" => WindowAction::Next,
            "previous" => WindowAction::Previous,
            "first" => WindowAction::First,
            "last" => WindowAction::Last,
            "escape" => WindowAction::Escape,
            other => return tracing::warn!(key = other, "unknown key name from the screen"),
        };
        if let Some(ui) = weak.upgrade() {
            run.run(Command::Window { action, canvas: canvas(&ui) });
        }
    });

    // Editing. The text is already on the canvas; the commands tell the model.
    let edited = |cx: &Cx<YPresent>, ui: &PresentationApp| {
        let (run, weak) = (cx.clone(), ui.as_weak());
        move |_: SharedString| {
            if let Some(ui) = weak.upgrade() {
                run.run(Command::Canvas(canvas(&ui)));
            }
        }
    };
    ui.on_title_edited(edited(cx, ui));
    ui.on_body_edited(edited(cx, ui));
    ui.on_notes_edited(edited(cx, ui));
    ui.on_set_layout(press(cx, ui, WindowAction::SetLayout));

    // Themes.
    ui.on_set_theme(press(cx, ui, WindowAction::SetTheme));
    ui.on_pres_select_template(press(cx, ui, WindowAction::SetTheme));
    ui.on_pres_open_template_gallery(button(cx, ui, WindowAction::OpenGallery));

    // Present, and the clock.
    ui.on_toggle_present(button(cx, ui, WindowAction::TogglePresent));
    ui.on_toggle_timer(button(cx, ui, WindowAction::ToggleTimer));
    ui.on_reset_timer(button(cx, ui, WindowAction::ResetTimer));

    // Files.
    ui.on_save_presentation(button(cx, ui, WindowAction::Save));
    ui.on_load_presentation(button(cx, ui, WindowAction::OpenRecent));
    ui.on_export_markdown(button(cx, ui, WindowAction::ExportMarkdown));
    ui.on_export_outline(button(cx, ui, WindowAction::ExportOutline));

    // Search, and history.
    ui.on_search_slides(press(cx, ui, |query: SharedString| WindowAction::Search(query.to_string())));
    ui.on_search_next(button(cx, ui, WindowAction::SearchNext));
    ui.on_undo(button(cx, ui, WindowAction::Undo));
    ui.on_redo(button(cx, ui, WindowAction::Redo));

    // Speaker notes, from the companion.
    ui.on_generate_notes(button(cx, ui, WindowAction::GenerateNotes));
}
