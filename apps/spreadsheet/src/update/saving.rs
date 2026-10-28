//! Saving the sheet: one file operation at a time, in a job, with how much of a failure reached the
//! disk said honestly, another program's edit never written over, and what a replace took the
//! place of kept (files/save.rs). A save that lands moves the sheet to the file it wrote, and goes
//! on with what waited for it: the question that asked, or the end of a linger.
//!
//! What is written is the sheet as it was when the save began. The sheet may be typed into while
//! the file is written; the landing marks the state that was written as saved (`Book::saved_at`),
//! so those edits stay unsaved.

use std::path::{Path, PathBuf};

use serde_json::json;
use yantrik_app::documents::{Durability, Fingerprint, OutsideState, SaveError, SaveOutcome, SaveState, Source};
use yantrik_app::effects::Effects;

use super::{cells, closing, keep_in_step, say, surface};
use crate::agent_rule::HIDDEN;
use crate::caller::Caller;
use crate::command::{AfterSave, Command, Write};
use crate::files::kept;
use crate::files::save::{save_csv, Refusal, Saved, SaveFailed, SaveJob};
use crate::history::Mark;
use crate::model::{Dialog, Model, Work};
use crate::YSheets;

/// Said when a save ended without an answer: it may have written anything, so nothing is called
/// saved.
const JOB_LOST: &str = "Save failed: the save did not finish. Your sheet is still open.";

/// Said when a save's landing is no longer the one its document expects. Nothing else changes the
/// document's save while it is in flight, so this is a guard, not a path: the write is not called
/// saved.
const OVERTAKEN: &str = "Save failed: the save was overtaken before it landed. Your sheet is still open.";

/// What an agent reads of a failed save of a sheet it is not shown, when the reason cannot be said
/// without a path.
const NOT_SAVED_HIDDEN: &str = "The sheet's file was not saved, and its draft is intact.";

/// Said when a save is asked for while a file is being read or written.
const FINISH_FIRST: &str = "Finish the current file operation first.";

/// The window's Save or Save As onto `target`, with the question's *Save* if `prompted`. A file
/// already at `target` is replaced only if `overwrite` says so: the picker has asked the person
/// about it, and a typed path is asked once (`there`).
pub fn start_window(m: &mut Model, fx: &mut Effects<YSheets>, target: PathBuf, overwrite: bool, prompted: bool) {
    start(m, fx, target, overwrite, false, AfterSave::Window { prompted }, false);
}

/// Writes the sheet to `target` in a job. With `must_be_new` (the surface's `save`) nothing is
/// replaced, the sheet's own file included.
pub fn start(
    m: &mut Model,
    fx: &mut Effects<YSheets>,
    target: PathBuf,
    overwrite: bool,
    must_be_new: bool,
    then: AfterSave,
    agent: bool,
) {
    if m.busy.is_some() {
        return say(m, FINISH_FIRST);
    }
    // The document knows of every change made so far, so the save is of the newest edit.
    keep_in_step(m);
    let own = m.path().map(PathBuf::from);
    let job = SaveJob {
        text: m.book().csv(),
        target: target.clone(),
        own,
        on_disk: m.doc.on_disk.clone(),
        unsure: m.doc.unsure.clone(),
        inode: m.doc.inode,
        overwrite,
        agent,
        others: Vec::new(),
        elsewhere: m.state_dir.join("kept"),
    };
    let (rows, cols) = m.sheet().used();
    let mark = m.book().mark();
    let d = m.d_mut();
    let seq = d.seq;
    // One save in flight, judged by the document when it lands, whatever the save was for.
    d.save = SaveState::Saving { seq };
    m.busy = Some(Work::Save);
    say(m, "Saving…");
    let written = Fingerprint::of_bytes(job.text.as_bytes());
    let failed = SaveFailed { error: SaveError::after_writing(Durability::Unknown, written, JOB_LOST), kept: None };
    let write = Box::new(Write { seq, mark, target, shape: (rows, cols) });
    let lost = Command::SaveLanded {
        write: write.clone(),
        then: then.clone(),
        result: Err(Box::new(Refusal { failed, there: false })),
    };
    let disk = m.disk.clone();
    fx.spawn_or(lost, move |_| Command::SaveLanded {
        write,
        then,
        result: save_csv(job, must_be_new, disk.as_ref()).map(Box::new),
    });
}

/// A save at `seq` landed. Whatever it was, the call it was for is answered, and what waited on it
/// goes on.
pub fn landed(
    m: &mut Model,
    fx: &mut Effects<YSheets>,
    write: Write,
    then: AfterSave,
    result: Result<Box<Saved>, Box<Refusal>>,
) {
    if m.busy == Some(Work::Save) {
        m.busy = None;
    }
    match result {
        Ok(saved) => wrote(m, fx, write, then, *saved),
        Err(refusal) => not_written(m, fx, write, then, *refusal),
    }
}

fn wrote(m: &mut Model, fx: &mut Effects<YSheets>, write: Write, then: AfterSave, saved: Saved) {
    let Write { seq, mark, shape: (rows, cols), .. } = write;
    let Saved { wrote, replaced } = saved;
    let outcome = m.d_mut().save_landed_witnessed(seq, Ok(wrote.written.clone()), wrote.witnessed);
    if !matches!(outcome, SaveOutcome::Saved { .. }) {
        m.doc.unsure = Some(wrote.written.clone());
        return not_saved(m, fx, &then, OVERTAKEN.into(), NOT_SAVED_HIDDEN.into());
    }
    let hidden = m.doc.hidden || wrote.file.hidden;
    moved_to_file(m, fx, &wrote, mark);
    super::outside::after_save(m, fx, true);
    m.dialog = Dialog::None;
    m.confirm_replace = None;
    let path = wrote.file.path.display().to_string();
    cells::done(
        m,
        format!(
            "Saved {path} — {rows} rows × {cols} columns, {} bytes{}.",
            wrote.bytes,
            if replaced { ", replacing what was there" } else { "" }
        ),
    );
    match &wrote.kept {
        Some(k) => m.notice.set_about(hidden, kept::said(k, &wrote.file.path), || kept::said_hidden(k)),
        None => m.notice.clear(),
    }
    // The call this save was for is answered first: what follows may end ySheets, and its
    // shutdown refuses whatever is still owed.
    let answer = json!({
        "path": path, "bytes": wrote.bytes, "rows": rows, "cols": cols, "replaced": replaced, "dirty": m.modified(),
    });
    let prompted = match then {
        AfterSave::Window { prompted } => prompted,
        AfterSave::Surface(_) => {
            surface::answer(m, fx, answer);
            false
        }
    };
    if prompted {
        closing::continue_pending(m, fx);
    }
    closing::saved(m, fx);
}

fn not_written(m: &mut Model, fx: &mut Effects<YSheets>, write: Write, then: AfterSave, refusal: Refusal) {
    let Write { seq, target, .. } = write;
    let Refusal { failed, there } = refusal;
    let stale = matches!(m.d_mut().save_landed(seq, Err(failed.error.clone())), SaveOutcome::Stale);
    if let Some(landed) = failed.error.landed.clone() {
        m.doc.unsure = Some(landed);
    }
    // A typed path onto a file that is there: asked about once, in the dialog that asked for it.
    if there && m.dialog == Dialog::SavePath && matches!(then, AfterSave::Window { .. }) {
        m.dialog_message = format!("{} is already there. Press Save again to replace it.", target.display());
        m.confirm_replace = Some(target);
        m.notice.clear();
        m.said = None;
        return;
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
    if m.dialog == Dialog::SavePath {
        m.dialog_message = window.clone();
    }
    not_saved(m, fx, &then, window, for_agents);
}

/// A failed save's sentence as an agent may read it about a sheet it is not shown: the file's
/// path, its folder and its name each said as `(hidden)`; or, when any other path is left in it
/// (where a moved file looks to be), the reason without one.
fn unnamed(sentence: &str, path: Option<&Path>) -> String {
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

/// A save that is not called saved: said (in its agent-safe form for a sheet an agent is not
/// shown), what waited on it told, its call refused.
fn not_saved(m: &mut Model, fx: &mut Effects<YSheets>, then: &AfterSave, sentence: String, for_agents: String) {
    let hidden = m.doc.hidden;
    m.notice.set_about(hidden, sentence.clone(), || for_agents);
    m.said = Some(crate::model::Said { text: sentence.clone(), problem: true, quotes_cells: false });
    super::outside::after_save(m, fx, false);
    closing::not_saved(m, fx, &sentence);
    if let AfterSave::Surface(Caller { agent }) = then {
        let said = m.notice.read(*agent).to_string();
        surface::settle_refused(fx, said);
    }
}

/// The sheet now is the file the save wrote: its source, its baseline, its watch, what an agent is
/// shown of it. The save checked the file held what the sheet expected before it replaced it, so
/// an outside change seen before is answered.
fn moved_to_file(m: &mut Model, fx: &mut Effects<YSheets>, wrote: &crate::files::save::Wrote, mark: Mark) {
    let d = m.d_mut();
    let source = Source::File(wrote.file.path.clone());
    if d.source != source {
        fx.unwatch(&d.source.key());
        d.source = source;
    }
    d.outside = OutsideState::Clean;
    d.content.saved_at(mark);
    if let Some((key, path, seen)) = d.watch_entry() {
        fx.watch(&key, path, seen);
    }
    m.doc.on_disk = Some(wrote.file.text.clone());
    m.doc.recovered = false;
    m.doc.hidden = wrote.file.hidden;
    m.doc.inode = wrote.file.inode;
    m.doc.unsure = None;
}
