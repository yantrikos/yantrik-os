//! Drafts: every tab with anything unsaved has its text in the framework's draft lane, so a crash,
//! a `kill -9` or a logout loses at most the last moment of typing (the lane's debounce), and the
//! next start opens it again. A tab saved, or typed back to its file's text, has no draft.
//!
//! The footer's recovery line says how that stands, as it always has: *Protecting draft…* while
//! a draft is on its way to the disk, *Draft recovery up to date* once every one has landed, and
//! *Recovery unavailable*, with the reason in the notice, when one could not be written.

use yantrik_app::documents::DraftState;
use yantrik_app::effects::Effects;

use super::say;
use crate::command::Command;
use crate::files::drafts::{now_ms, TabDraft, VERSION};
use crate::files::legacy;
use crate::model::{Legacy, Model};
use crate::tab::Tab;
use crate::TextEditor;

pub const READY: &str = "Draft recovery ready";
pub const PROTECTING: &str = "Protecting draft…";
pub const UP_TO_DATE: &str = "Draft recovery up to date";
pub const UNAVAILABLE: &str = "Recovery unavailable";

/// What tab `t`'s draft holds now.
fn draft_of(m: &Model, t: &Tab) -> TabDraft {
    TabDraft {
        version: VERSION,
        path: m.path(t).map(Into::into),
        text: m.text(t).to_string(),
        on_disk: t.on_disk.clone(),
        opened_at: t.opened_at,
        at: now_ms(),
    }
}

/// After every command: a draft asked for every unsaved tab whose draft is older than its edits,
/// and removed for every tab with nothing unsaved. A draft that could not be written is tried
/// again at the tab's next edit, not at every command.
pub fn sync(m: &mut Model, fx: &mut Effects<TextEditor>) {
    let mut asked = false;
    let mut discarded = false;
    for i in 0..m.tabs.len() {
        let t = &m.tabs[i];
        let doc = m.doc(t);
        let (seq, state) = (doc.seq, doc.draft.clone());
        if m.modified(t) {
            let owed = match state {
                DraftState::Kept { seq: kept } => kept < seq,
                DraftState::Failed { .. } => m.draft_failed_at.get(&t.draft).is_none_or(|at| *at < seq),
                DraftState::None => true,
            };
            if owed {
                let (name, bytes, doc) = (t.draft.clone(), draft_of(m, t).bytes(), t.doc);
                fx.save_draft(&name, bytes);
                m.drafts_pending.insert(name.clone());
                m.draft_failed_at.remove(&name);
                if let Some(d) = m.doc_mut(doc) {
                    d.draft_asked();
                }
                asked = true;
            }
        } else if state != DraftState::None {
            let (name, doc) = (t.draft.clone(), t.doc);
            fx.discard_draft(&name);
            m.drafts_pending.remove(&name);
            if let Some(d) = m.doc_mut(doc) {
                d.draft_discarded();
            }
            discarded = true;
        }
    }
    if asked {
        m.recovery = PROTECTING.into();
    } else if discarded && m.drafts_pending.is_empty() {
        m.recovery = UP_TO_DATE.into();
    }
}

/// Every unsaved tab's draft written now, not after its debounce: the editor is about to end.
/// How many there were.
pub fn checkpoint_all(m: &mut Model, fx: &mut Effects<TextEditor>) -> usize {
    let mut kept = 0;
    for i in 0..m.tabs.len() {
        let t = &m.tabs[i];
        if !m.modified(t) {
            continue;
        }
        let (name, bytes, doc) = (t.draft.clone(), draft_of(m, t).bytes(), t.doc);
        fx.checkpoint_draft(&name, bytes);
        m.drafts_pending.insert(name);
        if let Some(d) = m.doc_mut(doc) {
            d.draft_asked();
        }
        kept += 1;
    }
    kept
}

/// A draft reached the disk.
pub fn saved(m: &mut Model, fx: &mut Effects<TextEditor>, name: &str) {
    m.drafts_pending.remove(name);
    if m.drafts_pending.is_empty() {
        m.recovery = UP_TO_DATE.into();
    }
    settle_legacy(m, fx, name);
}

/// A draft could not be written: recovery after a crash is not protected, and the person is told.
pub fn not_saved(m: &mut Model, name: &str, why: &str) {
    m.drafts_pending.remove(name);
    m.recovery = UNAVAILABLE.into();
    say(m, format!("Draft recovery failed: {why}"));
    if let Some(t) = m.tabs.iter().find(|t| t.draft == name).cloned() {
        let seq = m.doc(&t).seq;
        m.draft_failed_at.insert(name.to_string(), seq);
        if let Some(d) = m.doc_mut(t.doc) {
            d.draft_failed(why);
        }
    }
}

/// A tab is gone: its draft with it.
pub fn forget(m: &mut Model, fx: &mut Effects<TextEditor>, name: &str) {
    fx.discard_draft(name);
    m.drafts_pending.remove(name);
    m.draft_failed_at.remove(name);
    settle_legacy(m, fx, name);
}

/// The first command: every tab with a file watched, and the old build's tabs handed to the lane
/// at once, so its file can be set aside.
pub fn started(m: &mut Model, fx: &mut Effects<TextEditor>) {
    for t in &m.tabs {
        if let Some((key, path, seen)) = m.doc(t).watch_entry() {
            fx.watch(&key, path, seen);
        }
    }
    let Legacy::Waiting(waiting) = m.legacy.clone() else { return };
    for i in 0..m.tabs.len() {
        let t = &m.tabs[i];
        if waiting.contains(&t.draft) {
            let (name, bytes, doc) = (t.draft.clone(), draft_of(m, t).bytes(), t.doc);
            fx.checkpoint_draft(&name, bytes);
            m.drafts_pending.insert(name);
            if let Some(d) = m.doc_mut(doc) {
                d.draft_asked();
            }
        }
    }
    if !waiting.is_empty() {
        m.recovery = PROTECTING.into();
    }
    archive_when_done(m, fx);
}

/// `name` is on disk, or its tab is gone: the old file waits on it no more.
fn settle_legacy(m: &mut Model, fx: &mut Effects<TextEditor>, name: &str) {
    if let Legacy::Waiting(waiting) = &mut m.legacy {
        waiting.remove(name);
    }
    archive_when_done(m, fx);
}

/// The old file set aside, once every tab it gave is the lane's.
fn archive_when_done(m: &mut Model, fx: &mut Effects<TextEditor>) {
    if !matches!(&m.legacy, Legacy::Waiting(w) if w.is_empty()) {
        return;
    }
    m.legacy = Legacy::Archiving;
    let dir = m.state_dir.clone();
    let lost = Command::LegacyArchived(Err("the job did not finish".into()));
    fx.spawn_or(lost, move |_| Command::LegacyArchived(legacy::archive(&dir)));
}

/// The old file was set aside, or could not be: then it stays as it is, and its tabs are the
/// lane's drafts all the same, so the next start opens each once.
pub fn legacy_archived(m: &mut Model, result: Result<(), String>) {
    m.legacy = Legacy::Done;
    if let Err(why) = result {
        tracing::warn!(error = %why, "the old recovery file could not be set aside; it is left as it was");
    }
}
