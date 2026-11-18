//! The window's own actions: its header, its keys, its buttons and its dialogs. While a file is
//! read or written they do nothing, and while a dialog is up only its answers are taken, as it
//! always was.

use yantrik_app::documents::Choice;
use yantrik_app::effects::Effects;

use super::{closing, editing, find, opening, say, saving, tabs};
use crate::command::{AfterSave, WindowAction};
use crate::files::expanded;
use crate::model::{Cue, Dialog, Model};
use crate::owner::{self, Opener};
use crate::prompt;
use crate::text::budget::room_for;
use crate::view::render::TEXT;
use crate::TextEditor;

pub fn act(m: &mut Model, fx: &mut Effects<TextEditor>, action: WindowAction) {
    if m.busy.is_some() {
        return;
    }
    if prompt::dialog(m) != 0 && !action.answers_a_dialog() {
        return;
    }
    match action {
        WindowAction::Undo | WindowAction::Redo => {
            // A refusal is in the notice already.
            let _ = editing::history(m, fx, action == WindowAction::Redo, &Opener::Person);
        }
        WindowAction::New => new_tab(m, fx, Opener::Person),
        WindowAction::Open => open_dialog(m, Dialog::Open, "~/".into()),
        WindowAction::SaveAs => {
            let path = m.path(m.active_tab()).map(|p| p.display().to_string());
            open_dialog(m, Dialog::SaveAs, path.unwrap_or_else(|| "~/untitled.txt".into()));
        }
        WindowAction::Save => save(m, fx),
        WindowAction::SaveClose => closing::answer(m, fx, Choice::Save),
        WindowAction::Goto => open_dialog(m, Dialog::Goto, m.cursor.0.to_string()),
        WindowAction::Confirm { path } => confirm(m, fx, path),
        WindowAction::Cancel => cancel(m),
        WindowAction::Discard => {
            if prompt::asking(m) && m.dialog == Dialog::None {
                closing::answer(m, fx, Choice::DontSave);
            }
        }
        WindowAction::Close => {
            let id = m.active_tab().id;
            closing::close_tab(m, fx, id);
        }
        WindowAction::NextTab => tabs::step(m, fx, true),
        WindowAction::PreviousTab => tabs::step(m, fx, false),
        WindowAction::FindNext => find::step(m, true),
        WindowAction::FindPrev => find::step(m, false),
        WindowAction::Replace { with } => replace(m, fx, false, with),
        WindowAction::ReplaceAll { with } => replace(m, fx, true, with),
    }
}

/// A new empty tab in front, while there is room for one.
pub fn new_tab(m: &mut Model, fx: &mut Effects<TextEditor>, opener: Opener) {
    if !room_for(&m.weighed(), 0) {
        let sentence = owner::no_room(&m.facts(), &opener);
        return say(m, sentence);
    }
    tabs::blank(m, opener);
    let last = m.tabs.len() - 1;
    m.notice.clear();
    tabs::select(m, fx, last);
}

fn open_dialog(m: &mut Model, dialog: Dialog, path: String) {
    m.dialog = dialog;
    m.dialog_path = path;
    m.dialog_error.clear();
    m.cues.push(Cue::Dialog);
}

/// Save: the tab's own file, or the Save As dialog for a tab that has none.
fn save(m: &mut Model, fx: &mut Effects<TextEditor>) {
    let id = m.active_tab().id;
    if m.path(m.active_tab()).is_none() {
        return open_dialog(m, Dialog::SaveAs, "~/untitled.txt".into());
    }
    saving::start(m, fx, id, None, false, AfterSave::Window, false);
}

/// The dialog's Open, Save or Go.
fn confirm(m: &mut Model, fx: &mut Effects<TextEditor>, path: String) {
    match m.dialog {
        Dialog::Open => opening::from_dialog(m, fx, expanded(&path)),
        Dialog::SaveAs => {
            // Asked from the close prompt: this save is its answer, and its landing closes the tab.
            closing::saving_as(m);
            let id = m.closing.as_ref().filter(|c| c.via_save_as).map(|c| c.tab).unwrap_or(m.active_tab().id);
            saving::start(m, fx, id, Some(expanded(&path)), false, AfterSave::Window, false);
        }
        Dialog::Goto => match path.trim().parse::<usize>() {
            Ok(line) => {
                let text = m.view_text();
                let offset = if line <= 1 {
                    0
                } else {
                    text.match_indices('\n').nth(line - 2).map(|(i, _)| i + 1).unwrap_or(text.len())
                };
                m.dialog = Dialog::None;
                m.cues.push(Cue::FocusEditor);
                m.cues.push(Cue::Select(offset, offset));
            }
            Err(_) => m.dialog_error = "Enter a whole line number.".into(),
        },
        Dialog::None => {}
    }
}

/// Cancel, or Escape: the dialog goes, and a close that asked is called off.
pub fn cancel(m: &mut Model) {
    m.dialog = Dialog::None;
    closing::cancel(m);
    m.cues.push(Cue::FocusEditor);
}

/// Replace the current match, or every match, with `with`.
fn replace(m: &mut Model, fx: &mut Effects<TextEditor>, all: bool, with: String) {
    m.find.replacement = with;
    match find::replaced(m, all, &m.find.replacement) {
        // `put` stores the whole replaced text, and the render shows its windowed view (#328).
        Ok(text) => {
            let _ = editing::put(m, fx, text, &Opener::Person);
        }
        Err(e) => say(m, e),
    }
    fx.changed(TEXT);
}
