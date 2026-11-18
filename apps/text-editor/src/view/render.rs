//! The only code that writes the window.
//!
//! The editor field is always given the windowed view of the tab in front, never the whole
//! document (#328), and only when it differs: the field is bound both ways, and setting the text
//! it already shows would move the caret of the person typing on every keystroke. The fields the
//! person types into (the dialog's path, the find bar) are set only when the model put something
//! there, by a cue, so a command arriving while they type never takes their text away.

use slint::{ModelRc, VecModel};
use yantrik_app::app::Changed;
use yantrik_app::for_mind::HIDDEN;

use crate::model::{Cue, Model};
use crate::prompt;
use crate::text::language::{highlight, language};
use crate::text::viewport;
use crate::update::find;
use crate::{DocumentTab, TextEditorApp};

/// The tab in front's text, or its view, changed: the field, the gutter and the colour layers.
pub const TEXT: &str = "text";
/// Everything else: the strip, the bars, the dialog, the footer. Drawn after every command but
/// the caret's.
pub const CHROME: &str = "chrome";
/// The caret moved: the footer's line and column.
pub const CARET: &str = "caret";

pub fn render(ui: &TextEditorApp, m: &Model, changed: &Changed) {
    let tab = m.active_tab();
    let text = m.text(tab);
    let view = m.viewport();
    let reset = m.cues.contains(&Cue::ResetContent);
    if changed.has(TEXT) || reset {
        // Where a Mind looks the field holds the mask, with no gutter and no colours: a gutter
        // is a count of the lines, and the colours are the text again.
        let shown = if m.for_mind { HIDDEN } else { &text[..view.end] };
        if reset || ui.get_content().as_str() != shown {
            ui.set_content(shown.into());
        }
        // The gutter and the colour layers cover the view, not the document: a gutter taller than
        // the clamped viewport would be laid out past the renderer's i16 coordinates again.
        let lines = if m.for_mind { 1 } else { view.lines };
        ui.set_numbers((1..=lines).map(|n| n.to_string()).collect::<Vec<_>>().join("\n").into());
        let layers = if m.for_mind { Default::default() } else { highlight(shown, language(m.path(tab))) };
        ui.set_has_highlights(!layers.is_empty());
        ui.set_keywords(layers.keywords.into());
        ui.set_strings(layers.strings.into());
        ui.set_comments(layers.comments.into());
    }
    if changed.has(CHROME) || changed.has(TEXT) {
        chrome(ui, m);
    }
    if changed.has(CARET) {
        let (line, column) = caret(m);
        ui.set_cursor_line(line);
        ui.set_cursor_column(column);
    }
    for cue in &m.cues {
        match *cue {
            Cue::FocusEditor => ui.invoke_focus_editor(),
            Cue::ResetPosition => ui.invoke_reset_position(),
            Cue::Select(..) if m.for_mind => {}
            Cue::Select(a, z) => ui.invoke_select_range(a as i32, z as i32),
            Cue::ShowFind => {
                ui.set_show_find(true);
                ui.set_query(m.find.query.clone().into());
            }
            Cue::Dialog => ui.set_dialog_path(m.dialog_path.clone().into()),
            Cue::Replacement => ui.set_replacement(m.find.replacement.clone().into()),
            Cue::ResetContent => {}
        }
    }
}

/// The caret as the footer shows it. Its offset is into the mask, so against the real text it
/// would say where the lines break; a Mind sees the start.
fn caret(m: &Model) -> (i32, i32) {
    if m.for_mind { (1, 1) } else { m.cursor }
}

fn chrome(ui: &TextEditorApp, m: &Model) {
    let tab = m.active_tab();
    let text = m.text(tab);
    let view = m.viewport();
    ui.set_tabs(ModelRc::new(VecModel::from(
        m.tabs
            .iter()
            .enumerate()
            .map(|(i, t)| DocumentTab { title: m.title(t).into(), active: i == m.active, modified: m.modified(t) })
            .collect::<Vec<_>>(),
    )));
    ui.set_active_tab(m.active as i32);
    ui.set_document_title(m.title(tab).into());
    ui.set_modified(m.modified(tab));
    ui.set_path_label(
        m.path(tab)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "Untitled · choose a home with Save As".into())
            .into(),
    );
    ui.set_language(language(m.path(tab)).into());
    // No measure of the text a Mind does not see: its lines, its line ending, or what was cut.
    let seen = !m.for_mind;
    ui.set_masked(m.for_mind);
    ui.set_line_count(if seen { crate::text::lines(text) as i32 } else { 0 });
    ui.set_ending(if seen && text.contains("\r\n") { "CRLF" } else { "LF" }.into());
    ui.set_view_limited(seen && view.limited);
    ui.set_view_status(if seen { viewport::status(view, text) } else { String::new() }.into());
    ui.set_busy(m.busy.is_some());
    ui.set_notice(m.notice.window().into());
    ui.set_recovery_status(m.recovery.clone().into());
    let (line, column) = caret(m);
    ui.set_cursor_line(line);
    ui.set_cursor_column(column);
    ui.set_match_case(m.find.match_case);
    // A Mind sees a mask in place of the text, so the window finds nothing in it; a search by a
    // call still counts as it always did, in the model.
    ui.set_match_count(if seen { m.find.matches.len() as i32 } else { 0 });
    ui.set_match_index(if seen { find::at_match(m) as i32 } else { 0 });
    ui.set_dialog(prompt::dialog(m));
    ui.set_dialog_error(m.dialog_error.clone().into());
    ui.set_close_label(prompt::label(m, false).into());
}
