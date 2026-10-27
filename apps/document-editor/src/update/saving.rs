//! Saving the document: one file operation at a time, in a job, with how much of a failure
//! reached the disk said honestly, another program's edit never written over, and what a replace
//! took the place of kept (files/save.rs). A save that lands moves the document to the file it
//! wrote, and goes on with what waited for it: the prompt that asked, or the end of a linger.
//!
//! An export is the same write of a copy: the document stays where it is.

use std::path::PathBuf;

use serde_json::json;
use yantrik_app::documents::{Durability, Fingerprint, OutsideState, SaveError, SaveOutcome, SaveState, Source};
use yantrik_app::effects::Effects;

use super::{closing, say, surface};
use crate::agent_rule::HIDDEN;
use crate::command::{AfterExport, AfterSave, Command};
use crate::files::kept;
use crate::files::save::{save, SaveFailed, SaveJob, Wrote};
use crate::markdown;
use crate::model::{Dialog, Model, Work};
use crate::YDoc;

/// Said when a save ended without an answer: it may have written anything, so nothing is called
/// saved.
const JOB_LOST: &str = "Save failed: the save did not finish. Your draft is still open.";

/// Said when a save's landing is no longer the one its document expects. Nothing else changes the
/// document's save while it is in flight, so this is a guard, not a path: the write is not called
/// saved.
const OVERTAKEN: &str = "Save failed: the save was overtaken before it landed. Your draft is still open.";

/// What an agent reads of a failed save of a document it is not shown, when the reason cannot be
/// said without a path.
const NOT_SAVED_HIDDEN: &str = "The document's file was not saved, and its draft is intact.";

/// Writes the document to `target` (its own file when `None`) in a job.
pub fn start(
    m: &mut Model,
    fx: &mut Effects<YDoc>,
    target: Option<PathBuf>,
    overwrite: bool,
    then: AfterSave,
    agent: bool,
) {
    let own = m.path().map(PathBuf::from);
    let Some(target) = target.or_else(|| own.clone()) else { return };
    let job = SaveJob {
        text: m.text().to_string(),
        target,
        own,
        on_disk: m.doc.on_disk.clone(),
        unsure: m.doc.unsure.clone(),
        inode: m.doc.inode,
        overwrite,
        agent,
        others: Vec::new(),
        elsewhere: m.state_dir.join("kept"),
    };
    let d = m.d_mut();
    let seq = d.seq;
    // One save in flight, judged by the document when it lands, whatever the save was for.
    d.save = SaveState::Saving { seq };
    m.busy = Some(Work::Save);
    m.dialog_error.clear();
    say(m, "Saving…");
    let written = Fingerprint::of_bytes(job.text.as_bytes());
    let lost = Command::SaveLanded {
        seq,
        then: then.clone(),
        result: Err(Box::new(SaveFailed {
            error: SaveError::after_writing(Durability::Unknown, written, JOB_LOST),
            kept: None,
        })),
    };
    let disk = m.disk.clone();
    fx.spawn_or(lost, move |_| Command::SaveLanded {
        seq,
        then,
        result: save(&job, disk.as_ref()).map(Box::new),
    });
}

/// A save at `seq` landed. Whatever it was, the call it was for is answered, and a prompt or a
/// linger waiting on it goes on.
pub fn landed(
    m: &mut Model,
    fx: &mut Effects<YDoc>,
    seq: u64,
    then: AfterSave,
    result: Result<Box<Wrote>, Box<SaveFailed>>,
) {
    if m.busy == Some(Work::Save) {
        m.busy = None;
    }
    match result {
        Ok(wrote) => {
            let outcome = m.d_mut().save_landed_witnessed(seq, Ok(wrote.written.clone()), wrote.witnessed);
            if !matches!(outcome, SaveOutcome::Saved { .. }) {
                m.doc.unsure = Some(wrote.written.clone());
                return not_saved(m, fx, &then, OVERTAKEN.into(), NOT_SAVED_HIDDEN.into());
            }
            let hidden = m.doc.hidden || wrote.file.hidden;
            moved_to_file(m, fx, &wrote);
            super::outside::after_save(m, fx, true);
            match &wrote.kept {
                Some(k) => m.notice.set_about(hidden, kept::said(k, &wrote.file.path), || kept::said_hidden(k)),
                None => m.notice.clear(),
            }
            m.dialog = Dialog::None;
            // The call this save was for is answered first: what follows may end yDoc, and its
            // shutdown refuses whatever is still owed.
            let answer = json!({ "saved": wrote.file.path, "bytes": wrote.bytes, "modified_unix": wrote.modified_unix });
            match then {
                AfterSave::Window => {}
                AfterSave::Save(_) | AfterSave::SaveAs(_) => surface::answer(m, fx, answer),
            }
            closing::saved(m, fx);
        }
        Err(failed) => {
            let stale = matches!(m.d_mut().save_landed(seq, Err(failed.error.clone())), SaveOutcome::Stale);
            if let Some(landed) = failed.error.landed.clone() {
                m.doc.unsure = Some(landed);
            }
            let path = m.path().map(PathBuf::from);
            let kept = failed.kept.as_ref().zip(path.as_deref()).map(|(k, p)| (kept::said(k, p), kept::said_hidden(k)));
            let sentence = if stale { OVERTAKEN.to_string() } else { failed.error.sentence.clone() };
            let (window, agents) = match kept {
                Some((said, safe)) => (format!("{said} {sentence}"), Some(safe)),
                None => (sentence, None),
            };
            let failure = unnamed(&failed.error.sentence, path.as_deref());
            let for_agents = agents.map_or(failure.clone(), |safe| format!("{safe} {failure}"));
            not_saved(m, fx, &then, window, for_agents);
        }
    }
}

/// A failed save's sentence as an agent may read it about a document it is not shown: the file's
/// path, its folder and its name each said as `(hidden)`; or, when any other path is left in it
/// (where a moved file looks to be), the reason without one.
fn unnamed(sentence: &str, path: Option<&std::path::Path>) -> String {
    let Some(path) = path else { return NOT_SAVED_HIDDEN.into() };
    let mut said = sentence.replace(&path.display().to_string(), HIDDEN);
    if let Some(folder) = path.parent().filter(|f| !f.as_os_str().is_empty()) {
        said = said.replace(&folder.display().to_string(), HIDDEN);
    }
    if let Some(name) = path.file_name() {
        said = said.replace(&*name.to_string_lossy(), HIDDEN);
    }
    if said.contains('/') {
        NOT_SAVED_HIDDEN.into()
    } else {
        said
    }
}

/// A save that is not called saved: said (in its agent-safe form for a document an agent is not
/// shown), its prompt or linger told, its call refused.
fn not_saved(m: &mut Model, fx: &mut Effects<YDoc>, then: &AfterSave, sentence: String, for_agents: String) {
    let hidden = m.doc.hidden;
    m.notice.set_about(hidden, sentence.clone(), || for_agents);
    m.dialog_error = sentence.clone();
    super::outside::after_save(m, fx, false);
    closing::not_saved(m, fx, &sentence);
    match then {
        AfterSave::Window => {}
        AfterSave::Save(caller) | AfterSave::SaveAs(caller) => {
            let said = m.notice.read(caller.agent).to_string();
            surface::settle_refused(fx, said);
        }
    }
}

/// The document now is the file the save wrote: its source, its baseline, its watch, what an
/// agent is shown of it. The save checked the file held what the document expected before it
/// replaced it, so an outside change seen before is answered.
fn moved_to_file(m: &mut Model, fx: &mut Effects<YDoc>, wrote: &Wrote) {
    let d = m.d_mut();
    let source = Source::File(wrote.file.path.clone());
    if d.source != source {
        fx.unwatch(&d.source.key());
        d.source = source;
    }
    d.outside = OutsideState::Clean;
    if let Some((key, path, seen)) = d.watch_entry() {
        fx.watch(&key, path, seen);
    }
    m.doc.on_disk = Some(wrote.file.text.clone());
    m.doc.recovered = false;
    m.doc.hidden = wrote.file.hidden;
    m.doc.inode = wrote.file.inode;
    m.doc.unsure = None;
}

/// Writes a copy of the document to `target` in a job, leaving the document in its own file. HTML
/// is the Markdown rendered; Markdown is the text as it is.
pub fn export(
    m: &mut Model,
    fx: &mut Effects<YDoc>,
    target: PathBuf,
    html: bool,
    then: AfterExport,
    agent: bool,
) {
    let text = if html { markdown::to_html(m.text(), &m.title()) } else { m.text().to_string() };
    // A copy has no home: nothing it is compared with, and a file already there is never replaced.
    let job = SaveJob {
        text,
        target,
        own: None,
        on_disk: None,
        unsure: None,
        inode: None,
        overwrite: false,
        agent,
        others: Vec::new(),
        elsewhere: m.state_dir.join("kept"),
    };
    m.busy = Some(Work::Export);
    m.dialog_error.clear();
    say(m, "Exporting…");
    let lost = Command::Exported {
        then: then.clone(),
        html,
        result: Err(Box::new(SaveFailed { error: SaveError::new(JOB_LOST), kept: None })),
    };
    let disk = m.disk.clone();
    fx.spawn_or(lost, move |_| Command::Exported { then, html, result: save(&job, disk.as_ref()).map(Box::new) });
}

/// An export landed.
pub fn exported(
    m: &mut Model,
    fx: &mut Effects<YDoc>,
    then: AfterExport,
    html: bool,
    result: Result<Box<Wrote>, Box<SaveFailed>>,
) {
    if m.busy == Some(Work::Export) {
        m.busy = None;
    }
    match result {
        Ok(wrote) => {
            m.notice.clear();
            m.dialog = Dialog::None;
            if let AfterExport::Surface(_) = then {
                let format = if html { "html" } else { "markdown" };
                let answer = json!({
                    "exported": wrote.file.path, "format": format,
                    "bytes": wrote.bytes, "modified_unix": wrote.modified_unix,
                });
                surface::answer(m, fx, answer);
            }
        }
        Err(failed) => {
            let sentence = failed.error.sentence.clone();
            m.dialog_error = sentence.clone();
            say(m, sentence.clone());
            // A close that waited on this export stays undone: the failure is what the person reads.
            closing::not_closed(m);
            if let AfterExport::Surface(_) = then {
                surface::settle_refused(fx, sentence);
            }
        }
    }
}
