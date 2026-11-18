//! Opening a file as the document, in a job: from the Open dialog, a launch, the unsaved prompt's
//! answer, or the surface. Nothing is replaced until the file has been read; one that cannot be
//! read leaves the document as it was.

use std::path::PathBuf;

use yantrik_app::effects::Effects;

use super::{document, say, surface};
use crate::caller::Caller;
use crate::command::{AfterOpen, Command};
use crate::files::open::{open, Opened};
use crate::markdown;
use crate::model::{Dialog, Model, Work};
use crate::prompt;
use crate::YDoc;

/// Said when a job ended without an answer (it panicked, or the pool refused it).
const JOB_LOST: &str = "The file could not be read; try opening it again.";

/// Opens the file at `path`, read in a job.
pub fn start(m: &mut Model, fx: &mut Effects<YDoc>, path: PathBuf, then: AfterOpen, caller: Caller) {
    m.busy = Some(Work::Open);
    say(m, "Opening file…");
    let agent = caller.agent;
    let lost = Command::Opened { then: then.clone(), result: Err(JOB_LOST.into()) };
    fx.spawn_or(lost, move |_| Command::Opened { then, result: open(&path, agent) });
}

/// The Open dialog's Open: not while a file is read or written, or while the prompt asks. A
/// document with unsaved changes asks first; the path rides on the prompt.
pub fn from_dialog(m: &mut Model, fx: &mut Effects<YDoc>, path: PathBuf) {
    if m.busy.is_some() {
        return say(m, "Finish the current file operation first.");
    }
    if prompt::asking(m) {
        return say(m, "Resolve the unsaved changes dialog first.");
    }
    m.dialog_error.clear();
    if m.modified() {
        // There is work on screen that is in no file. Ask about that before the file being
        // opened takes the window; answering the prompt does not mean typing the path again.
        m.dialog = Dialog::None;
        return super::closing::ask(m, prompt::Intent::Open(path));
    }
    start(m, fx, path, AfterOpen::Dialog, Caller::person());
}

/// A file a launch asked to open, once yDoc is free and nothing is asked.
pub fn launch(m: &mut Model, fx: &mut Effects<YDoc>, path: PathBuf) {
    m.queue = Some(path);
    next(m, fx);
}

/// The file a launch asked for, when yDoc is free. A launch is an `open` without `discard`: with
/// unsaved changes on screen it is refused in the notice and the draft stays, as a second launch
/// handing its file to the running window always was.
pub fn next(m: &mut Model, fx: &mut Effects<YDoc>) {
    if m.busy.is_some() || prompt::dialog(m) != prompt::NONE || m.lingering {
        return;
    }
    if let Some(path) = m.queue.take() {
        if let Some(lost) = m.unsaved(false) {
            let hidden = m.unsaved(true).unwrap_or_default();
            let hide = m.doc.hidden;
            return m.notice.set_about(hide, surface::would_throw_away(&path, &lost), || {
                surface::would_throw_away(&path, &hidden)
            });
        }
        start(m, fx, path, AfterOpen::Launch, Caller::person());
    }
}

/// An open landed.
pub fn landed(m: &mut Model, fx: &mut Effects<YDoc>, then: AfterOpen, result: Result<Opened, String>) {
    if m.busy == Some(Work::Open) {
        m.busy = None;
    }
    match result {
        Ok(opened) => {
            let words = markdown::word_count(&opened.text);
            let headings = markdown::outline(&opened.text).len();
            let bytes = opened.text.len();
            let path = opened.path.clone();
            document::of_file(m, fx, opened);
            m.dialog = Dialog::None;
            m.notice.clear();
            // Last, because a successful open clears the notice and a draft held aside is the one
            // thing a person opening a file still needs to be told.
            if let (AfterOpen::Launch, Some(said)) = (&then, &m.held_aside) {
                m.notice.set(said.clone());
            }
            if let AfterOpen::Surface(caller) = &then {
                // A file an agent is not shown (a link out of the places it may name) is opened
                // for the window, and the agent learns nothing of it.
                let answer = if caller.agent && m.doc.hidden {
                    serde_json::json!({ "opened": crate::agent_rule::HIDDEN })
                } else {
                    serde_json::json!({ "opened": path, "bytes": bytes, "words": words, "headings": headings })
                };
                surface::answer(m, fx, answer);
            }
        }
        Err(e) => match &then {
            AfterOpen::Surface(_) => surface::refuse(m, fx, e),
            AfterOpen::Dialog => {
                m.dialog_error = e.clone();
                say(m, e);
            }
            AfterOpen::Launch | AfterOpen::Prompted => say(m, e),
        },
    }
    next(m, fx);
}
