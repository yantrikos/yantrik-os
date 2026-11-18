//! Saving a tab through its document: one file operation at a time, in a job, with how much of a
//! failure reached the disk said honestly, another program's edit never written over, and what a
//! replace took the place of kept (files/save.rs). A save that lands moves the tab to the file it
//! wrote, and goes on with what waited for it: the close that asked, or the editor's end.

use std::path::PathBuf;

use yantrik_app::documents::{Durability, Fingerprint, OutsideState, SaveError, SaveOutcome, SaveState, Source};
use yantrik_app::effects::Effects;

use super::{closing, say, surface};
use crate::command::{AfterSave, Command};
use crate::files::kept;
use crate::files::save::{save, SaveFailed, SaveJob, Wrote, CHANGED_ON_DISK};
use crate::model::{Dialog, Model, Work, HIDDEN_TAB};
use crate::tab::{Tab, TabId};
use crate::TextEditor;

/// Said when a save ended without an answer: it may have written anything, so nothing is called
/// saved.
const JOB_LOST: &str = "Save failed: the save did not finish. Your draft is still open.";

/// Writes tab `id` to `target` (its own file when `None`) in a job.
pub fn start(
    m: &mut Model,
    fx: &mut Effects<TextEditor>,
    id: TabId,
    target: Option<PathBuf>,
    overwrite: bool,
    then: AfterSave,
    agent: bool,
) {
    let Some(tab) = m.tab(id).cloned() else { return };
    let own = m.path(&tab).map(PathBuf::from);
    let Some(target) = target.or_else(|| own.clone()) else { return };
    let others = m
        .tabs
        .iter()
        .filter(|t| t.id != id)
        .filter_map(|t| m.path(t).map(|p| (p.to_path_buf(), if t.hidden { HIDDEN_TAB.into() } else { m.title(t) })))
        .collect();
    let elsewhere = m.state_dir.join("kept");
    let Some(doc) = m.doc_mut(tab.doc) else { return };
    let seq = doc.seq;
    let job = SaveJob {
        text: doc.content.clone(),
        target,
        own,
        on_disk: tab.on_disk.clone(),
        unsure: tab.unsure.clone(),
        inode: tab.inode,
        overwrite,
        agent,
        others,
        elsewhere,
    };
    // One save in flight, judged by the document when it lands, whatever the save was for.
    doc.save = SaveState::Saving { seq };
    m.busy = Some(Work::Save(id));
    m.dialog_error.clear();
    say(m, "Saving…");
    let written = Fingerprint::of_bytes(job.text.as_bytes());
    let lost = Command::SaveLanded {
        tab: id,
        seq,
        then: then.clone(),
        result: Err(Box::new(SaveFailed {
            error: SaveError::after_writing(Durability::Unknown, written, JOB_LOST),
            kept: None,
        })),
    };
    let disk = m.disk.clone();
    fx.spawn_or(lost, move |_| Command::SaveLanded { tab: id, seq, then, result: save(&job, disk.as_ref()) });
}

/// Said when a save's landing is no longer the one its document expects. Nothing else changes a
/// tab's save while it is in flight, so this is a guard, not a path: the write is not called saved.
const OVERTAKEN: &str = "Save failed: the save was overtaken before it landed. Your draft is still open.";

/// A save of tab `id` at `seq` landed. Whatever it was, the call it was for is answered, and a close
/// or a linger waiting on it goes on.
pub fn landed(
    m: &mut Model,
    fx: &mut Effects<TextEditor>,
    id: TabId,
    seq: u64,
    then: AfterSave,
    result: Result<Wrote, Box<SaveFailed>>,
) {
    if m.busy == Some(Work::Save(id)) {
        m.busy = None;
    }
    let Some(tab) = m.tab(id).cloned() else {
        say(m, "The tab was closed before its save landed.");
        return surface::refuse_if_owed(m, fx, &then);
    };
    match result {
        Ok(wrote) => {
            let Some(doc) = m.doc_mut(tab.doc) else { return };
            let outcome = doc.save_landed_witnessed(seq, Ok(wrote.written.clone()), wrote.witnessed);
            if !matches!(outcome, SaveOutcome::Saved { .. }) {
                if let Some(t) = m.tab_mut(id) {
                    t.unsure = Some(wrote.written.clone());
                }
                return not_saved(m, fx, &tab, &then, OVERTAKEN.into(), NOT_SAVED_HIDDEN.into());
            }
            let hidden = tab.hidden || wrote.file.hidden;
            moved_to_file(m, fx, id, &wrote);
            super::outside::after_save(m, fx, id, true);
            match &wrote.kept {
                Some(k) => m.notice.set_about(hidden, kept::said(k, &wrote.file.path), || kept::said_hidden(k)),
                None => say(m, "Saved"),
            }
            m.dialog = Dialog::None;
            closing::saved(m, fx, id);
            surface::answers::saved(m, fx, &then, id);
        }
        Err(failed) => {
            let Some(doc) = m.doc_mut(tab.doc) else { return };
            let stale = matches!(doc.save_landed(seq, Err(failed.error.clone())), SaveOutcome::Stale);
            if let (Some(t), Some(landed)) = (m.tab_mut(id), failed.error.landed.clone()) {
                t.unsure = Some(landed);
            }
            let kept = failed.kept.as_ref().zip(m.path(&tab)).map(|(k, p)| (kept::said(k, p), kept::said_hidden(k)));
            let sentence = if stale { OVERTAKEN.to_string() } else { failed.error.sentence.clone() };
            let (window, agents) = match kept {
                Some((said, safe)) => (format!("{said} {sentence}"), Some(safe)),
                None => (sentence, None),
            };
            let failure = if failed.error.conflict {
                CHANGED_ON_DISK.to_string()
            } else {
                unnamed(&failed.error.sentence, m.path(&tab))
            };
            let for_agents = agents.map_or(failure.clone(), |safe| format!("{safe} {failure}"));
            not_saved(m, fx, &tab, &then, window, for_agents);
        }
    }
}

/// What an agent reads of a failed save of a tab it is not shown, when the reason cannot be said
/// without a path.
const NOT_SAVED_HIDDEN: &str = "The tab's file was not saved, and its draft is intact.";

/// A failed save's sentence as an agent may read it about a tab it is not shown: the file's path,
/// its folder and its name each said as `(hidden)`; or, when any other path is left in it (where a
/// moved file looks to be), the reason without one.
fn unnamed(sentence: &str, path: Option<&std::path::Path>) -> String {
    let Some(path) = path else { return NOT_SAVED_HIDDEN.into() };
    let mut said = sentence.replace(&path.display().to_string(), HIDDEN_TAB);
    if let Some(folder) = path.parent().filter(|f| !f.as_os_str().is_empty()) {
        said = said.replace(&folder.display().to_string(), HIDDEN_TAB);
    }
    if let Some(name) = path.file_name() {
        said = said.replace(&*name.to_string_lossy(), HIDDEN_TAB);
    }
    if said.contains('/') {
        NOT_SAVED_HIDDEN.into()
    } else {
        said
    }
}

/// A save that is not called saved: said (in its agent-safe form for a tab an agent is not shown),
/// its close or linger told, its call refused.
fn not_saved(m: &mut Model, fx: &mut Effects<TextEditor>, tab: &Tab, then: &AfterSave, sentence: String, for_agents: String) {
    m.notice.set_about(tab.hidden, sentence.clone(), || for_agents);
    m.dialog_error = sentence.clone();
    super::outside::after_save(m, fx, tab.id, false);
    closing::not_saved(m, fx, tab.id, &sentence);
    surface::refuse_if_owed(m, fx, then);
}

/// The tab now is the file the save wrote: its source, its baseline, its watch, what an agent is
/// shown of it. The save checked the file held what the tab expected before it replaced it, so an
/// outside change seen before is answered.
fn moved_to_file(m: &mut Model, fx: &mut Effects<TextEditor>, id: TabId, wrote: &Wrote) {
    let Some(tab) = m.tab(id).cloned() else { return };
    let Some(doc) = m.doc_mut(tab.doc) else { return };
    let source = Source::File(wrote.file.path.clone());
    if doc.source != source {
        fx.unwatch(&doc.source.key());
        doc.source = source;
    }
    doc.outside = OutsideState::Clean;
    if let Some((key, path, seen)) = doc.watch_entry() {
        fx.watch(&key, path, seen);
    }
    if let Some(t) = m.tab_mut(id) {
        t.on_disk = Some(wrote.file.text.clone());
        t.recovered = false;
        t.hidden = wrote.file.hidden;
        t.inode = wrote.file.inode;
        t.unsure = None;
    }
}
