//! The draft: a document with anything unsaved has its text in the framework's draft lane, so a
//! crash, a `kill -9` or a logout loses at most the last moment of typing (the lane's debounce),
//! and the next start opens it again. A document saved, or typed back to its file's text, has no
//! draft.

use yantrik_app::documents::DraftState;
use yantrik_app::effects::Effects;

use super::say;
use crate::command::Command;
use crate::files::drafts::{now_ms, DocDraft, VERSION};
use crate::files::legacy;
use crate::model::{Legacy, Model};
use crate::YDoc;

/// What the draft holds now.
fn draft_of(m: &Model) -> DocDraft {
    DocDraft {
        version: VERSION,
        path: m.path().map(Into::into),
        text: m.text().to_string(),
        on_disk: m.doc.on_disk.clone(),
        at: now_ms(),
    }
}

/// After every command: a draft asked for when the document is unsaved and its draft is older
/// than its edits, and removed when nothing is unsaved. A draft that could not be written is
/// tried again at the document's next edit, not at every command.
pub fn sync(m: &mut Model, fx: &mut Effects<YDoc>) {
    let (seq, state) = (m.d().seq, m.d().draft.clone());
    let name = m.doc.draft.clone();
    if m.modified() {
        let owed = match state {
            DraftState::Kept { seq: kept } => kept < seq,
            DraftState::Failed { .. } => m.draft_failed_at.get(&name).is_none_or(|at| *at < seq),
            DraftState::None => true,
        };
        if owed {
            fx.save_draft(&name, draft_of(m).bytes());
            m.draft_failed_at.remove(&name);
            m.d_mut().draft_asked();
        }
    } else if state != DraftState::None {
        fx.discard_draft(&name);
        m.d_mut().draft_discarded();
        // Saved before its draft landed: the old file's text is in the document's own file now.
        settle_legacy(m, fx, &name);
    }
}

/// The draft written now, not after its debounce: yDoc is about to end. Whether there was
/// anything unsaved to keep.
pub fn checkpoint_now(m: &mut Model, fx: &mut Effects<YDoc>) -> bool {
    if !m.modified() {
        return false;
    }
    let name = m.doc.draft.clone();
    fx.checkpoint_draft(&name, draft_of(m).bytes());
    m.d_mut().draft_asked();
    true
}

/// A draft reached the disk.
pub fn saved(m: &mut Model, fx: &mut Effects<YDoc>, name: &str) {
    settle_legacy(m, fx, name);
}

/// A draft could not be written: recovery after a crash is not protected, and the person is told.
pub fn not_saved(m: &mut Model, name: &str, why: &str) {
    say(m, format!("Draft recovery failed: {why}"));
    if m.doc.draft == name {
        let seq = m.d().seq;
        m.draft_failed_at.insert(name.to_string(), seq);
        m.d_mut().draft_failed(why);
    }
}

/// The document is gone: its draft with it.
pub fn forget(m: &mut Model, fx: &mut Effects<YDoc>, name: &str) {
    fx.discard_draft(name);
    m.draft_failed_at.remove(name);
    settle_legacy(m, fx, name);
}

/// The first command: the document's file watched, and the old build's draft handed to the lane
/// at once, so its file can be set aside.
pub fn started(m: &mut Model, fx: &mut Effects<YDoc>) {
    if let Some((key, path, seen)) = m.d().watch_entry() {
        fx.watch(&key, path, seen);
    }
    match m.legacy.clone() {
        Legacy::Waiting(waiting) if waiting == m.doc.draft => {
            let name = m.doc.draft.clone();
            fx.checkpoint_draft(&name, draft_of(m).bytes());
            m.d_mut().draft_asked();
        }
        Legacy::Owed => set_aside(m, fx),
        _ => {}
    }
}

/// `name` is on disk, or its document is gone: the old file waits on it no more.
fn settle_legacy(m: &mut Model, fx: &mut Effects<YDoc>, name: &str) {
    if !matches!(&m.legacy, Legacy::Waiting(w) if w == name) {
        return;
    }
    set_aside(m, fx);
}

/// The old file renamed, in a job.
fn set_aside(m: &mut Model, fx: &mut Effects<YDoc>) {
    m.legacy = Legacy::Archiving;
    let dir = legacy::old_dir(&m.state_dir);
    let lost = Command::LegacyArchived(Err("the job did not finish".into()));
    fx.spawn_or(lost, move |_| Command::LegacyArchived(legacy::archive(&dir)));
}

/// The old file was set aside, or could not be: then it stays as it is, and its document is the
/// lane's draft all the same, so the next start opens it once.
pub fn legacy_archived(m: &mut Model, result: Result<(), String>) {
    m.legacy = Legacy::Done;
    if let Err(why) = result {
        // Left in place, it is offered again at the next start unless the lane still holds it as
        // it was (same path, same text), so a recovered draft that was edited or saved since comes
        // back once more.
        tracing::warn!(
            error = %why,
            "the old recovery file could not be set aside; it is left as it was, and the next start may offer its \
             text again as a draft from a previous session"
        );
    }
}
