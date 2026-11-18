//! Saving the deck: one file operation at a time, in a job, with how much of a failure reached the
//! disk said honestly, another program's edit never written over, and what a replace took the
//! place of kept (files/save.rs). A save that lands moves the deck to the file it wrote, and goes
//! on with what waited for it: the deck it was written to make room for, or the end of a linger.
//!
//! An export is the same write of a copy: the deck stays where it is.

use std::path::{Path, PathBuf};

use serde_json::json;
use yantrik_app::documents::{Durability, Fingerprint, OutsideState, SaveError, SaveOutcome, SaveState, Source};
use yantrik_app::effects::Effects;

use super::{closing, keep_in_step, opening, say, slides, surface};
use crate::agent_rule::{HIDDEN, HIDDEN_REFUSAL};
use crate::command::{AfterSave, Away, Cause, Command, Next};
use crate::deck::{encode, Content, Edits};
use crate::files::kept;
use crate::files::names::export_path;
use crate::files::export::{export_copy, DefaultName};
use crate::files::save::{save_deck, SaveFailed, SaveJob, Unnamed, Wrote};
use crate::model::{Model, Work};
use crate::YPresent;

/// Said when a save ended without an answer: it may have written anything, so nothing is called
/// saved.
const JOB_LOST: &str = "Save failed: the save did not finish. Your draft is still open.";

/// Said when a save's landing is no longer the one its document expects. Nothing else changes the
/// document's save while it is in flight, so this is a guard, not a path: the write is not called
/// saved.
const OVERTAKEN: &str = "Save failed: the save was overtaken before it landed. Your draft is still open.";

/// What an agent reads of a failed save of a deck it is not shown, when the reason cannot be said
/// without a path.
const NOT_SAVED_HIDDEN: &str = "The deck's file was not saved, and its draft is intact.";

/// Said when edits were made while the deck was being written to make room for another.
const CHANGED_MEANWHILE: &str =
    "The deck changed while it was being saved, so it was not replaced. Nothing is lost; try again.";

/// Where a save goes.
#[derive(Debug, Clone)]
pub enum Target {
    /// The deck's own file.
    Own,
    /// A path the caller named.
    Path(PathBuf),
    /// A file of the app's own choosing in the default folder, named after the deck.
    Unnamed,
}

/// Writes the deck to `target` in a job.
pub fn start(m: &mut Model, fx: &mut Effects<YPresent>, target: Target, overwrite: bool, then: AfterSave, agent: bool) {
    // The document knows of every change made so far, so the save is of the newest edit.
    keep_in_step(m);
    let own = m.path().map(PathBuf::from);
    let (path, unnamed) = match target {
        Target::Own => (own.clone().unwrap_or_default(), None),
        Target::Path(path) => (path, None),
        Target::Unnamed => (PathBuf::new(), Some(Unnamed { title: m.title().to_string(), dir: m.deck_dir.clone() })),
    };
    let written = m.deck().content().clone();
    let job = SaveJob {
        text: encode(&written),
        target: path,
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
    say(m, "Saving…");
    let fingerprint = Fingerprint::of_bytes(job.text.as_bytes());
    let failed = SaveFailed { error: SaveError::after_writing(Durability::Unknown, fingerprint, JOB_LOST), kept: None };
    let lost = Command::SaveLanded {
        seq,
        then: then.clone(),
        written: Box::new(written.clone()),
        result: Err(Box::new(failed)),
    };
    let disk = m.disk.clone();
    fx.spawn_or(lost, move |_| Command::SaveLanded {
        seq,
        then,
        written: Box::new(written),
        result: save_deck(job, unnamed, disk.as_ref()).map(Box::new),
    });
}

/// The window's Save. The window has no Save As, so a deck whose file is gone or was changed by
/// another program is kept in a file of its own beside it, never written over theirs and never
/// left unsaved; the notice says where.
pub fn window_save(m: &mut Model, fx: &mut Effects<YPresent>, canvas: &Edits) {
    slides::typed(m, canvas);
    let (target, away) = match (m.path(), &m.d().outside) {
        (None, _) => (Target::Unnamed, None),
        (Some(_), OutsideState::Deleted) => (Target::Unnamed, Some(Away::Gone)),
        (Some(_), OutsideState::Conflict { .. }) => (Target::Unnamed, Some(Away::Changed)),
        (Some(_), _) if m.doc.on_disk.is_none() => (Target::Unnamed, Some(Away::Unchecked)),
        (Some(_), _) => (Target::Own, None),
    };
    start(m, fx, target, false, AfterSave::Window(away), false);
}

/// The deck about to be replaced, written to a file of its own first.
pub fn rescue(m: &mut Model, fx: &mut Effects<YPresent>, next: Next, then: Cause, agent: bool) {
    // A deck with no baseline (it came back from the old build's recovery file) is not written
    // onto its file: nothing says the file is what it was typed against.
    let target = match (m.path(), &m.d().outside) {
        (Some(_), OutsideState::Clean | OutsideState::Changed { .. }) if m.doc.on_disk.is_some() => Target::Own,
        _ => Target::Unnamed,
    };
    // A deck an agent is not shown is never copied somewhere it can read: the file the rule
    // judges is the new one, not where the content came from. The deck stays in front, and its
    // draft is kept.
    if agent && m.doc.hidden && !matches!(target, Target::Own) {
        return opening::refuse(m, fx, &then, HIDDEN_REFUSAL.into());
    }
    start(m, fx, target, false, AfterSave::Rescue { next, cause: then }, agent);
}

/// A save at `seq` landed. Whatever it was, the call it was for is answered, and what waited on it
/// goes on.
pub fn landed(
    m: &mut Model,
    fx: &mut Effects<YPresent>,
    seq: u64,
    then: AfterSave,
    written: Content,
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
            let away_from = m.path().map(Path::to_path_buf);
            moved_to_file(m, fx, &wrote, written.clone());
            super::outside::after_save(m, fx, true);
            match &wrote.kept {
                Some(k) => m.notice.set_about(hidden, kept::said(k, &wrote.file.path), || kept::said_hidden(k)),
                None => m.notice.clear(),
            }
            // The call this save was for is answered first: what follows may end yPresent, and
            // its shutdown refuses whatever is still owed.
            let path = wrote.file.path.display().to_string();
            let answer = json!({
                "path": path, "bytes": wrote.bytes, "slides": written.slides.len(), "dirty": m.modified(),
            });
            match then {
                AfterSave::Window(away) => said_away(m, away, away_from.as_deref(), &wrote.file.path),
                AfterSave::Save(_) | AfterSave::SaveAs(_) => surface::answer(m, fx, answer),
                AfterSave::Rescue { next, cause } => {
                    if m.lingering {
                        // No window to put it in; the deck is saved, and that was all that was owed.
                    } else if m.deck().content() != &written {
                        opening::refuse(m, fx, &cause, CHANGED_MEANWHILE.into());
                    } else {
                        opening::put_in_front(m, fx, next, cause, Some(wrote.file.path.clone()));
                    }
                }
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

/// Says where a deck went when the window's Save could not write its own file.
fn said_away(m: &mut Model, away: Option<Away>, was: Option<&Path>, now: &Path) {
    let (Some(away), Some(was)) = (away, was) else { return };
    let why = match away {
        Away::Gone => "was moved or deleted",
        Away::Changed => "was changed by another program",
        Away::Unchecked => "may have changed since this deck was typed (it came back from an older recovery file)",
    };
    let said = |name: &str| format!("{name} {why}, so the deck was saved as a new file: {}.", now.display());
    let hidden = m.doc.hidden;
    m.notice.set_about(hidden, said(&was.display().to_string()), || {
        "A deck an agent is not shown was saved as a new file.".into()
    });
}

/// A failed save's sentence as an agent may read it about a deck it is not shown: the file's
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

/// A save that is not called saved: said (in its agent-safe form for a deck an agent is not
/// shown), what waited on it told, its call refused.
fn not_saved(m: &mut Model, fx: &mut Effects<YPresent>, then: &AfterSave, sentence: String, for_agents: String) {
    let hidden = m.doc.hidden;
    let replacing = matches!(then, AfterSave::Rescue { .. });
    let (sentence, for_agents) = if replacing {
        (not_replaced(&sentence), not_replaced(&for_agents))
    } else {
        (sentence, for_agents)
    };
    m.notice.set_about(hidden, sentence.clone(), || for_agents);
    super::outside::after_save(m, fx, false);
    closing::not_saved(m, fx, &sentence);
    match then {
        AfterSave::Window(_) => {}
        AfterSave::Save(caller) | AfterSave::SaveAs(caller) | AfterSave::Rescue { cause: Cause::Surface(caller), .. } => {
            let said = m.notice.read(caller.agent).to_string();
            surface::settle_refused(fx, said);
        }
        AfterSave::Rescue { .. } => {}
    }
}

/// A reason a save failed, with what that meant for the deck it was written to make room for.
fn not_replaced(sentence: &str) -> String {
    let stop = if sentence.ends_with(['.', '!', '?']) { "" } else { "." };
    format!("{sentence}{stop} The deck was not replaced.")
}

/// The deck now is the file the save wrote: its source, its baseline, its watch, what an agent is
/// shown of it. The save checked the file held what the deck expected before it replaced it, so
/// an outside change seen before is answered.
fn moved_to_file(m: &mut Model, fx: &mut Effects<YPresent>, wrote: &Wrote, written: Content) {
    let d = m.d_mut();
    let source = Source::File(wrote.file.path.clone());
    if d.source != source {
        fx.unwatch(&d.source.key());
        d.source = source;
    }
    d.outside = OutsideState::Clean;
    d.content.saved_as(written);
    if let Some((key, path, seen)) = d.watch_entry() {
        fx.watch(&key, path, seen);
    }
    m.doc.on_disk = Some(wrote.file.text.clone());
    m.doc.recovered = false;
    m.doc.hidden = wrote.file.hidden;
    m.doc.inode = wrote.file.inode;
    m.doc.unsure = None;
}

/// The window's export buttons: a copy beside the deck, with the deck's own stem.
pub fn export_window(m: &mut Model, fx: &mut Effects<YPresent>, extension: &str, canvas: &Edits) {
    slides::typed(m, canvas);
    let target = export_path(m.path(), &m.deck_dir, m.title(), extension);
    export(m, fx, target, extension, true, Cause::Window, false);
}

/// Writes a copy of the deck to `target` in a job, leaving the deck in its own file. A name this
/// app made from the deck's own (`default_name`) is replaced only by an export of this deck that
/// nothing has changed since; if it is taken by anything else the window counts up (`talk-2.md`)
/// and an agent is refused. A name the caller gave is never replaced.
pub fn export(
    m: &mut Model,
    fx: &mut Effects<YPresent>,
    target: PathBuf,
    extension: &str,
    default_name: bool,
    then: Cause,
    agent: bool,
) {
    let count_up = default_name && matches!(then, Cause::Window | Cause::Launch);
    // A copy never replaces the document it is a copy of: a deck saved as `talk.md` has the
    // default export's very name. (The window counts up past it.)
    if !count_up && m.path() == Some(target.as_path()) {
        let sentence = "An export never replaces the deck it comes from; give it another path.";
        return opening::refuse(m, fx, &then, sentence.into());
    }
    let text = if extension == "md" { m.deck().to_markdown() } else { m.deck().to_outline() };
    // A copy has no home: nothing it is compared with.
    let job = SaveJob {
        text,
        target: target.clone(),
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
    say(m, "Exporting\u{2026}");
    let failed = SaveFailed { error: SaveError::new(JOB_LOST), kept: None };
    let slides = m.deck().len();
    // What an earlier export of this deck begins with: the Markdown's header line, or the
    // outline's title and first number.
    let header = match extension {
        "md" => format!("<!-- yPresent deck: {} -->\n", m.title()),
        _ => format!("# {}\n\n1. ", m.title()),
    };
    let stem = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .and_then(|n| n.strip_suffix(&format!(".{extension}")).map(str::to_string))
        .unwrap_or_default();
    let default = default_name.then(|| DefaultName {
        dir: target.parent().map(PathBuf::from).unwrap_or_default(),
        stem,
        extension: extension.to_string(),
        header,
        remembered: m.doc.exports.clone(),
        count_up,
    });
    let remember = default_name;
    let lost = Command::Exported { then: then.clone(), slides, remember, result: Err(Box::new(failed)) };
    let disk = m.disk.clone();
    fx.spawn_or(lost, move |_| Command::Exported {
        then,
        slides,
        remember,
        result: export_copy(job, default.as_ref(), disk.as_ref()).map(Box::new),
    });
}

/// An export landed.
pub fn exported(
    m: &mut Model,
    fx: &mut Effects<YPresent>,
    then: Cause,
    slides: usize,
    remember: bool,
    result: Result<Box<Wrote>, Box<SaveFailed>>,
) {
    if m.busy == Some(Work::Export) {
        m.busy = None;
    }
    match result {
        Ok(wrote) => {
            let path = wrote.file.path.display().to_string();
            if remember {
                // The next default export may replace this file, and only while it is still this.
                let at = wrote.file.path.clone();
                m.doc.exports.retain(|(p, _)| *p != at);
                m.doc.exports.push((at, wrote.written.clone()));
            }
            m.notice.clear();
            match then {
                Cause::Surface(_) => {
                    let answer = json!({ "path": path, "bytes": wrote.bytes, "slides": slides });
                    surface::answer(m, fx, answer);
                }
                Cause::Window | Cause::Launch => {
                    let hidden = m.doc.hidden;
                    m.notice.set_about(hidden, format!("Exported to {path}."), || format!("Exported to {HIDDEN}."));
                }
            }
        }
        Err(failed) => {
            let sentence = failed.error.sentence.clone();
            // Said to the person with the file named; an agent reads it for a deck it is not shown
            // without the name.
            let hidden = m.doc.hidden;
            m.notice.set_about(hidden, sentence.clone(), || "The export was not written.".to_string());
            // A close that waited on this export stays undone: the failure is what the person reads.
            closing::not_closed(m);
            if let Cause::Surface(_) = then {
                surface::settle_refused(fx, sentence);
            }
        }
    }
}
