//! The window's own actions: its header, its keys, its buttons and its dialogs. While a file is
//! read or written they do nothing, and while a dialog is up only its answers are taken.

use yantrik_app::documents::Choice;
use yantrik_app::effects::Effects;

use super::{closing, document, find, opening, say, saving};
use crate::command::{AfterExport, AfterSave, WindowAction};
use crate::files::expanded;
use crate::model::{Cue, Dialog, Model};
use crate::prompt::{self, Intent};
use crate::YDoc;

pub fn act(m: &mut Model, fx: &mut Effects<YDoc>, action: WindowAction) {
    if m.busy.is_some() {
        return;
    }
    if prompt::dialog(m) != prompt::NONE && !action.answers_a_dialog() {
        return;
    }
    match action {
        WindowAction::New => new_document(m, fx),
        WindowAction::Open => open_dialog(m, Dialog::Open, "~/Documents/".into()),
        WindowAction::Save => save(m, fx),
        WindowAction::SaveAs => open_dialog(m, Dialog::SaveAs, m.home().display().to_string()),
        WindowAction::ExportMarkdown => open_dialog(m, Dialog::ExportMarkdown, m.home().display().to_string()),
        WindowAction::ExportHtml => {
            let mut path = m.home();
            path.set_extension("html");
            open_dialog(m, Dialog::ExportHtml, path.display().to_string());
        }
        WindowAction::Confirm { path } => confirm(m, fx, path),
        WindowAction::Cancel => cancel(m),
        WindowAction::Discard => {
            if prompt::asking(m) && m.dialog == Dialog::None {
                closing::answer(m, fx, Choice::DontSave);
            }
        }
        WindowAction::FindNext => find::step(m, true),
        WindowAction::FindPrev => find::step(m, false),
        WindowAction::ReplaceOne { with } => replace(m, fx, false, with),
        WindowAction::ReplaceAll { with } => replace(m, fx, true, with),
    }
}

/// New: a blank document, or the prompt standing in front of it when anything is unsaved. The
/// window never drops a draft on the floor.
fn new_document(m: &mut Model, fx: &mut Effects<YDoc>) {
    if m.modified() {
        return closing::ask(m, Intent::New);
    }
    document::blank(m, fx);
    m.notice.clear();
}

fn open_dialog(m: &mut Model, dialog: Dialog, path: String) {
    m.dialog = dialog;
    m.dialog_path = path;
    m.dialog_error.clear();
    m.cues.push(Cue::Dialog);
}

/// Save: the document's own file, or the Save As dialog for a document that has none. Save used
/// to return in silence there; a person pressing it all day lost everything.
fn save(m: &mut Model, fx: &mut Effects<YDoc>) {
    if m.path().is_none() {
        return open_dialog(m, Dialog::SaveAs, m.home().display().to_string());
    }
    saving::start(m, fx, None, false, AfterSave::Window, false);
}

/// The dialog's Open, Save or Export; or, with the prompt up and no file dialog over it, its
/// *Save*.
fn confirm(m: &mut Model, fx: &mut Effects<YDoc>, path: String) {
    match m.dialog {
        Dialog::Open => opening::from_dialog(m, fx, expanded(&path)),
        Dialog::SaveAs => {
            // Asked from the prompt: this save is its answer, and its landing carries it out.
            closing::saving_as(m);
            saving::start(m, fx, Some(expanded(&path)), false, AfterSave::Window, false);
        }
        Dialog::ExportMarkdown => saving::export(m, fx, expanded(&path), false, AfterExport::Window, false),
        Dialog::ExportHtml => saving::export(m, fx, expanded(&path), true, AfterExport::Window, false),
        Dialog::None => {
            if prompt::asking(m) {
                closing::answer(m, fx, Choice::Save);
            }
        }
    }
}

/// Cancel, or Escape: the dialog goes, and a prompt that asked is called off.
pub fn cancel(m: &mut Model) {
    m.dialog = Dialog::None;
    m.dialog_error.clear();
    closing::cancel(m);
    m.cues.push(Cue::FocusBody);
}

/// Replace the current match, or every match, with `with`.
fn replace(m: &mut Model, fx: &mut Effects<YDoc>, all: bool, with: String) {
    m.find.replacement = with;
    if let Err(e) = find::replace_matches(m, fx, all) {
        say(m, e);
    }
}
