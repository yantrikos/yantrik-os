//! The only code that writes the window.
//!
//! The canvas is given the slide's text only when the command asked for it (`Model::reload`): it
//! is bound both ways, and setting the text of the slide the person is typing into would put their
//! cursor back at the start of the field on every word. Everything else (the rail, the counts, the
//! bars, the themes, the notice) is derived from the model each time and written only where it
//! differs, so no two of them can disagree and a deck at rest draws nothing.
//!
//! The deck is untrusted: its titles and text reach the window as plain strings in `Text` and
//! `TextInput`, which draw them and interpret nothing.
//!
//! Drawn for a Mind (`Model::for_mind`), every slide's text is the framework's one sentence
//! instead, and the canvas takes no typing (mind_view.rs).

use slint::{Model as _, ModelRc, VecModel};
use yantrik_app::app::Changed;
use yantrik_app::for_mind::{shown_if_any, HIDDEN};
use yantrik_app::rows;

use crate::deck::{theme_spec, Slide, THEMES};
use crate::model::{Cue, Model};
use crate::update::clock;
use crate::{PresentTheme, PresentationApp, SlideData};

/// The slide on the canvas: its text and layout.
pub const CANVAS: &str = "canvas";
/// Everything else: the rail, the counts, the bars, the themes, the notice. Drawn after every
/// command.
pub const CHROME: &str = "chrome";
/// The presenter's clock alone, which ticks once a second.
pub const CLOCK: &str = "clock";

pub fn render(ui: &PresentationApp, m: &Model, changed: &Changed) {
    if changed.has(CANVAS) {
        canvas(ui, m);
    }
    if changed.has(CHROME) {
        chrome(ui, m);
    }
    if changed.has(CHROME) || changed.has(CLOCK) {
        ui.set_timer_text(clock::text(m.clock.seconds).into());
    }
    for cue in &m.cues {
        match cue {
            Cue::CloseOverlays => {
                ui.set_show_search(false);
                ui.set_pres_template_gallery_open(false);
            }
            Cue::OpenGallery => ui.set_pres_template_gallery_open(true),
            Cue::Query => ui.set_search_query(m.query.clone().into()),
        }
    }
}

fn masked(m: &Model, text: &str) -> slint::SharedString {
    shown_if_any(m.for_mind, text).as_ref().into()
}

fn row_of(m: &Model, slide: &Slide) -> SlideData {
    SlideData {
        title: masked(m, &slide.title),
        body: masked(m, &slide.body),
        notes: masked(m, &slide.notes),
        layout: slide.layout,
    }
}

fn brush(rgb: (u8, u8, u8)) -> slint::Brush {
    slint::Brush::SolidColor(slint::Color::from_rgb_u8(rgb.0, rgb.1, rgb.2))
}

/// One of the deck's themes in the shape the screen draws it.
fn present_theme(index: usize, active: bool) -> PresentTheme {
    let spec = theme_spec(index);
    PresentTheme {
        name: spec.name.into(),
        bg_color: brush(spec.bg),
        text_color: brush(spec.text),
        accent_color: brush(spec.accent),
        is_active: active,
    }
}

fn canvas(ui: &PresentationApp, m: &Model) {
    let slide = m.deck().current_slide();
    ui.set_current_title(masked(m, &slide.title));
    ui.set_current_body(masked(m, &slide.body));
    ui.set_current_notes(masked(m, &slide.notes));
    ui.set_current_layout(slide.layout);
}

/// The rail's rows. Changed in place in the list the window holds, so moving the selection does
/// not scroll the rail back to the top, and neither does adding or removing a slide.
fn rail(ui: &PresentationApp, rows: Vec<SlideData>) {
    let shown = ui.get_slides();
    if rows::same(&shown, &rows) {
        return;
    }
    match shown.as_any().downcast_ref::<VecModel<SlideData>>() {
        Some(list) if list.row_count() == rows.len() => {
            for (i, row) in rows.into_iter().enumerate() {
                if list.row_data(i).as_ref() != Some(&row) {
                    list.set_row_data(i, row);
                }
            }
        }
        Some(list) => list.set_vec(rows),
        None => ui.set_slides(ModelRc::new(VecModel::from(rows))),
    }
}

fn chrome(ui: &PresentationApp, m: &Model) {
    let deck = m.deck();
    let (count, at) = (deck.len(), deck.current());
    rail(ui, deck.slides().iter().map(|s| row_of(m, s)).collect());

    ui.set_slide_count(count as i32);
    ui.set_current_slide_index(at as i32);
    ui.set_slide_progress(format!("Slide {} of {}", at + 1, count).into());
    // A deck an agent is not shown has no name or path in a Mind's window either: the Mind reads
    // `(hidden)` for them through `describe`.
    let concealed = m.for_mind && m.doc.hidden;
    ui.set_presentation_title(if concealed { HIDDEN.into() } else { deck.title().into() });
    let file = m.path().map(|p| p.display().to_string()).unwrap_or_default();
    ui.set_file_path(if concealed && !file.is_empty() { HIDDEN.into() } else { file.into() });
    let modified = m.modified();
    ui.set_is_modified(modified);
    ui.set_save_status(
        match (m.path().is_some(), modified) {
            (false, _) => "Not saved yet",
            (true, true) => "Unsaved changes",
            (true, false) => "Saved",
        }
        .into(),
    );
    ui.set_is_presenting(m.presenting);

    // The presenter view's "Next Slide" card.
    match deck.slide(at + 1) {
        Some(next) => {
            ui.set_next_title(masked(m, &next.title));
            ui.set_next_body(masked(m, &next.body));
        }
        None => {
            ui.set_next_title("End of deck".into());
            ui.set_next_body("".into());
        }
    }

    // The screen reads `themes[0]` everywhere it draws a slide, so it is given the one that is
    // chosen; the gallery is given all of them, each in its own colours.
    let chosen = deck.theme() % THEMES.len();
    ui.set_current_theme(chosen as i32);
    rows::set(ui.get_themes(), vec![present_theme(chosen, true)], |l| ui.set_themes(l));
    let swatches = (0..THEMES.len()).map(|i| present_theme(i, i == chosen)).collect();
    rows::set(ui.get_theme_swatches(), swatches, |l| ui.set_theme_swatches(l));

    ui.set_search_count(m.hits.len() as i32);
    // A Mind's window draws what an agent would read of the notice, not the person's sentence.
    ui.set_notice(if m.for_mind { m.notice.read(true) } else { m.notice.window() }.into());
}
