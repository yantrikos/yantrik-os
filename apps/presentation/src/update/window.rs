//! The window's buttons and keys.

use yantrik_app::effects::Effects;

use super::{canvas_for, clock, notes, opening, say, saving, search, slides};
use crate::command::WindowAction;
use crate::deck::Edits;
use crate::model::{Cue, Model};
use crate::YPresent;

/// Said when a file button is pressed while another file is being read or written.
const FILE_BUSY: &str = "Finish the current file operation first.";

/// Says what a change could not do, or clears the notice when it did: a notice on screen is
/// always about the last thing that happened rather than about something three actions ago.
fn settle<T>(m: &mut Model, outcome: Result<T, String>) {
    match outcome {
        Ok(_) => m.notice.clear(),
        Err(message) => {
            tracing::warn!(error = %message, "yPresent could not do that");
            say(m, message);
        }
    }
}

pub fn act(m: &mut Model, fx: &mut Effects<YPresent>, action: WindowAction, held: Edits) {
    let mut canvas = canvas_for(m, held);
    match action {
        WindowAction::AddSlide => {
            let slide = slides::next_blank(m);
            let outcome = slides::add(m, &canvas, None, slide);
            settle(m, outcome);
        }
        WindowAction::DeleteSlide => {
            let at = m.deck().current();
            let outcome = slides::delete(m, &canvas, at);
            settle(m, outcome);
        }
        WindowAction::DuplicateSlide => {
            let outcome = slides::duplicate(m, &canvas);
            settle(m, outcome);
        }
        WindowAction::MoveUp => {
            let at = m.deck().current();
            let outcome = if at > 0 { slides::move_slide(m, &canvas, at, at - 1) } else { Ok(at) };
            m.reload = true;
            settle(m, outcome);
        }
        WindowAction::MoveDown => {
            let (at, len) = (m.deck().current(), m.deck().len());
            let outcome = if at + 1 < len { slides::move_slide(m, &canvas, at, at + 1) } else { Ok(at) };
            m.reload = true;
            settle(m, outcome);
        }
        WindowAction::Select(index) => {
            slides::go_to(m, &canvas, index);
        }
        WindowAction::Next => step(m, &canvas, 1),
        WindowAction::Previous => step(m, &canvas, -1),
        WindowAction::First => {
            slides::go_to(m, &canvas, 0);
        }
        WindowAction::Last => {
            let last = m.deck().len().saturating_sub(1) as i64;
            slides::go_to(m, &canvas, last);
        }
        WindowAction::Escape => {
            m.cues.push(Cue::CloseOverlays);
            if m.presenting {
                m.presenting = false;
                m.reload = true;
            }
        }
        WindowAction::SetLayout(layout) => {
            canvas.layout = layout;
            slides::typed(m, &canvas);
            m.reload = true;
        }
        WindowAction::SetTheme(index) => {
            m.deck_mut().set_theme(index.max(0) as usize);
        }
        WindowAction::OpenGallery => m.cues.push(Cue::OpenGallery),
        WindowAction::TogglePresent => {
            // Through the same commit as everything else, so entering and leaving both save what
            // is on the canvas: the presenter view reads the model, not the editor's properties.
            let at = m.deck().current() as i64;
            slides::go_to(m, &canvas, at);
            m.presenting = !m.presenting;
        }
        WindowAction::Save => file_button(m, |m| saving::window_save(m, fx, &canvas)),
        WindowAction::OpenRecent => file_button(m, |m| opening::recent(m, fx)),
        WindowAction::ExportMarkdown => file_button(m, |m| saving::export_window(m, fx, "md", &canvas)),
        WindowAction::ExportOutline => file_button(m, |m| saving::export_window(m, fx, "outline.md", &canvas)),
        WindowAction::Search(query) => search::run(m, &canvas, &query),
        WindowAction::SearchNext => search::next(m, &canvas),
        WindowAction::Undo => history(m, &canvas, false),
        WindowAction::Redo => history(m, &canvas, true),
        WindowAction::GenerateNotes => notes::ask(m, fx, &canvas),
        WindowAction::ToggleTimer => clock::toggle(m, fx),
        WindowAction::ResetTimer => clock::reset(m, fx),
    }
}

/// A button that reads or writes a file: not while another file is being read or written.
fn file_button(m: &mut Model, press: impl FnOnce(&mut Model)) {
    if m.busy.is_some() {
        return say(m, FILE_BUSY);
    }
    press(m);
}

fn step(m: &mut Model, canvas: &Edits, delta: i64) {
    let at = m.deck().current() as i64;
    slides::go_to(m, canvas, at + delta);
}

fn history(m: &mut Model, canvas: &Edits, redo: bool) {
    if slides::undo(m, canvas, redo) {
        m.notice.clear();
    } else {
        say(m, if redo { "Nothing to redo." } else { "Nothing to undo." });
    }
}
