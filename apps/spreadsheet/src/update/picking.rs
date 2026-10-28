//! Asking for a file: the desktop's file picker, and the typed path when there is none to ask.
//!
//! The picker waits for the person, so it is asked in a job; its answer comes back as a command.
//! What it answers is the person's own choice, so a file it names is opened or written as the
//! person would, and a save onto a file that is there has already been agreed to in the picker.

use std::path::PathBuf;

use yantrik_app::effects::Effects;

use super::{opening, saving};
use crate::caller::Caller;
use crate::command::{Cause, Command};
use crate::files::names::suggested;
use crate::files::picker::Choice;
use crate::model::{Dialog, Model, Said};
use crate::YSheets;

/// Said when the job that asks ended without an answer.
const NO_ANSWER: &str = "the file picker did not answer";

/// Asks the picker for a file to open, or where to save, in a job.
pub fn pick(m: &mut Model, fx: &mut Effects<YSheets>, saving: bool) {
    if m.picking {
        return;
    }
    m.picking = true;
    let text = if saving { "Choosing where to save…" } else { "Choosing a file to open…" };
    m.said = Some(Said { text: text.into(), problem: false, quotes_cells: false });
    let current = m.path().map(PathBuf::from);
    let ask = m.ask.clone();
    let name = "sheet.csv";
    let lost = Command::Picked {
        saving,
        choice: Choice::Unavailable(NO_ANSWER.into()),
        suggested: suggested(None, name),
    };
    fx.spawn_or(lost, move |_| {
        let choice = ask(saving, current.clone());
        // The place to suggest is only wanted when the picker could not be asked; the disk is read
        // here, in the job, either way.
        let suggested = suggested(current.as_deref(), name);
        Command::Picked { saving, choice, suggested }
    });
}

/// The picker answered.
pub fn picked(m: &mut Model, fx: &mut Effects<YSheets>, saving: bool, choice: Choice, suggested: PathBuf) {
    m.picking = false;
    match choice {
        // The picker has already asked the person about replacing a file that is there.
        Choice::Chosen(path) if saving => {
            let prompted = m.pending.is_some();
            saving::start_window(m, fx, path, true, prompted);
        }
        Choice::Chosen(path) => opening::start(m, fx, path, Cause::Window, Caller::person()),
        Choice::Cancelled => {
            m.pending = None;
            m.discarding = false;
            m.said = Some(Said { text: "Nothing was chosen, so nothing changed.".into(), problem: false, quotes_cells: false });
        }
        Choice::Unavailable(why) => {
            tracing::warn!(%why, "No file picker; asking for a typed path");
            m.dialog = if saving { Dialog::SavePath } else { Dialog::OpenPath };
            m.dialog_message =
                format!("The desktop's file picker could not be asked: {why}. Type the path instead (/ or ~/).");
            m.dialog_path = suggested.display().to_string();
        }
    }
}
