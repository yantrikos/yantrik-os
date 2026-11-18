//! Opening a file into a tab, one at a time, in a job: from the Open dialog, a launch, or the
//! surface. A file already open is brought forward instead of opened twice; the one empty tab a
//! new window has is replaced by the first file opened; and a tab opens only while there is room.

use std::path::PathBuf;

use yantrik_app::effects::Effects;

use super::{find, say, surface, tabs};
use crate::caller::Caller;
use crate::command::{AfterOpen, Command};
use crate::files::open::{open, Opened};
use crate::model::{Cue, Dialog, Model, Work};
use crate::owner::{self, Opener};
use crate::prompt;
use crate::text::budget::room_for;
use crate::view::render::TEXT;
use crate::TextEditor;

/// Said when a job ended without an answer (it panicked, or the pool refused it).
const JOB_LOST: &str = "The file could not be read; try opening it again.";

/// Opens the file at `path`, read in a job.
pub fn start(m: &mut Model, fx: &mut Effects<TextEditor>, path: PathBuf, then: AfterOpen, caller: &Caller) {
    m.busy = Some(Work::Open);
    say(m, "Opening file…");
    let agent = caller.agent;
    let lost = Command::Opened { then: then.clone(), result: Err(JOB_LOST.into()) };
    fx.spawn_or(lost, move |_| Command::Opened { then, result: open(&path, agent) });
}

/// The Open dialog's Open: not while a file is read or written, or while the close prompt asks.
pub fn from_dialog(m: &mut Model, fx: &mut Effects<TextEditor>, path: PathBuf) {
    if m.busy.is_some() {
        return say(m, "Finish the current file operation first.");
    }
    if prompt::asking(m) {
        return say(m, "Resolve the unsaved changes dialog first.");
    }
    m.dialog_error.clear();
    start(m, fx, path, AfterOpen::Dialog, &Caller::person());
}

/// Files a launch asked to open: each in turn, once the editor is free and nothing is asked.
pub fn launch(m: &mut Model, fx: &mut Effects<TextEditor>, files: Vec<PathBuf>) {
    m.queue.extend(files);
    next(m, fx);
}

/// The next file a launch asked for, when the editor is free.
pub fn next(m: &mut Model, fx: &mut Effects<TextEditor>) {
    if m.busy.is_some() || prompt::dialog(m) != 0 || m.lingering {
        return;
    }
    if let Some(path) = m.queue.pop_front() {
        start(m, fx, path, AfterOpen::Launch, &Caller::person());
    }
}

/// An open landed.
pub fn landed(m: &mut Model, fx: &mut Effects<TextEditor>, then: AfterOpen, result: Result<Opened, String>) {
    if m.busy == Some(Work::Open) {
        m.busy = None;
    }
    let opener = match &then {
        AfterOpen::Surface(caller) => caller.opener.clone(),
        _ => Opener::Person,
    };
    let placed = result.and_then(|opened| place(m, fx, opened, &opener));
    match (&then, placed) {
        (_, Ok(())) => {
            m.dialog = Dialog::None;
            m.notice.clear();
            m.cues.push(Cue::ResetPosition);
            m.cues.push(Cue::FocusEditor);
            find::search(m, false);
            fx.changed(TEXT);
            if let AfterOpen::Surface(caller) = &then {
                surface::answers::opened(m, fx, caller);
            }
        }
        (AfterOpen::Surface(_), Err(e)) => surface::refuse(m, fx, e),
        (_, Err(e)) => {
            m.dialog_error = e.clone();
            say(m, e);
        }
    }
    next(m, fx);
}

/// Puts what was opened in a tab and brings it forward, or says why there is no room for it.
fn place(m: &mut Model, fx: &mut Effects<TextEditor>, opened: Opened, opener: &Opener) -> Result<(), String> {
    if let Some(index) = m.tabs.iter().position(|t| m.path(t) == Some(opened.path.as_path())) {
        m.active = index;
        return Ok(());
    }
    if m.tabs.len() == 1 && m.is_blank(&m.tabs[0]) {
        let blank = m.tabs[0].id;
        tabs::of_file(m, fx, opened, opener.clone());
        tabs::drop_blank(m, fx, blank);
        m.active = 0;
        return Ok(());
    }
    if !room_for(&m.weighed(), opened.text.len()) {
        return Err(owner::no_room(&m.facts(), opener));
    }
    tabs::of_file(m, fx, opened, opener.clone());
    m.active = m.tabs.len() - 1;
    Ok(())
}
