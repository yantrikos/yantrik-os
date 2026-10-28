//! The draft: a sheet with anything unsaved has its text in the framework's draft lane, so a
//! crash, a `kill -9` or a logout loses at most the last moment of typing, and the next start opens
//! it again. A sheet saved, or typed back to its file's content, has no draft.
//!
//! Writing a draft means encoding the whole sheet, which for a large sheet is not something to do
//! at every keystroke. So an edit only arms a short timer (arming it again restarts it), and the
//! draft is made when typing pauses; the lane writes it after its own debounce. A close or a
//! logout makes it at once, in the command that answers.

use std::time::Duration;

use yantrik_app::documents::DraftState;
use yantrik_app::effects::Effects;
use yantrik_app::timers::TimerId;

use super::say;
use crate::command::Command;
use crate::files::drafts::{now_ms, SheetDraft, VERSION};
use crate::model::Model;
use crate::YSheets;

/// How long the sheet stands still before its draft is made.
pub const PAUSE: Duration = Duration::from_millis(250);

const TIMER: TimerId = TimerId("ysheets.draft");

/// What the draft holds now, with the text in the formula bar if it differs from its cell: the
/// edit nobody entered goes into the cell in the draft, so a start gives it back.
fn draft_of(m: &Model, bar: Option<&str>) -> SheetDraft {
    let mut rows = m.sheet().used_rows();
    let typing = bar.filter(|text| *text != m.sheet().raw(m.active)).map(|text| {
        let (r, c) = (m.active.row, m.active.col);
        let width = rows.first().map_or(0, Vec::len).max(c + 1);
        rows.resize(rows.len().max(r + 1), Vec::new());
        rows.iter_mut().for_each(|row| row.resize(width, String::new()));
        rows[r][c] = text.to_string();
        m.active.name()
    });
    SheetDraft {
        version: VERSION,
        path: m.path().map(Into::into),
        rows,
        on_disk: m.doc.on_disk.clone(),
        active: m.active.name(),
        typing,
        at: now_ms(),
    }
}

/// Whether the sheet's draft is behind its edits, and no encode of this very state is on its way.
fn owed(m: &Model) -> bool {
    let (seq, name) = (m.d().seq, &m.doc.draft);
    if m.encoding == Some(seq) {
        return false;
    }
    match &m.d().draft {
        DraftState::Kept { seq: kept } => *kept < seq,
        DraftState::Failed { .. } => m.draft_failed_at.get(name).is_none_or(|at| *at < seq),
        DraftState::None => true,
    }
}

/// After every command: a draft asked for when the sheet is unsaved and its draft is older than its
/// edits, and removed when nothing is unsaved. A draft that could not be written is tried again at
/// the sheet's next edit, not at every command.
pub fn sync(m: &mut Model, fx: &mut Effects<YSheets>) {
    if m.given_up || m.ending {
        return;
    }
    if m.modified() {
        if owed(m) {
            fx.after(TIMER, PAUSE, Command::DraftDue);
        }
    } else if m.d().draft != DraftState::None {
        let name = m.doc.draft.clone();
        fx.stop_timer(TIMER);
        fx.discard_draft(&name);
        m.draft_discarded();
    }
}

/// The sheet has stood still: its draft is encoded now, in a job, if it is still owed. Encoding a
/// large sheet is not for the window's thread; the job is given the sheet's rows as they are (shared,
/// not copied) and the lane gets the bytes when they land, if the sheet is still the one they are of.
pub fn due(m: &mut Model, fx: &mut Effects<YSheets>) {
    if m.given_up || !m.modified() || !owed(m) {
        return;
    }
    let (name, seq) = (m.doc.draft.clone(), m.d().seq);
    m.encoding = Some(seq);
    let snapshot = m.sheet().snapshot();
    let (path, on_disk, active) = (m.path().map(Into::into), m.doc.on_disk.clone(), m.active.name());
    let lost = Command::DraftEncoded { name: name.clone(), seq, bytes: None };
    fx.spawn_or(lost, move |_| {
        let draft = SheetDraft { version: VERSION, path, rows: snapshot.used_rows(), on_disk, active, typing: None, at: now_ms() };
        Command::DraftEncoded { name, seq, bytes: Some(draft.bytes()) }
    });
}

/// A draft was encoded. It is handed to the lane only if the sheet has not been typed into since
/// (the newer state has its own pause and its own encode) and is still unsaved; otherwise it is
/// dropped, as a stale recalculation is.
pub fn encoded(m: &mut Model, fx: &mut Effects<YSheets>, name: String, seq: u64, bytes: Option<Vec<u8>>) {
    if m.encoding == Some(seq) {
        m.encoding = None;
    }
    let Some(bytes) = bytes else { return };
    if m.given_up || m.doc.draft != name || m.d().seq != seq || !m.modified() {
        return;
    }
    fx.save_draft(&name, bytes);
    m.draft_failed_at.remove(&name);
    m.draft_asked();
}

/// The draft written now, not after its pause: ySheets is about to end. Whether there was anything
/// unsaved to keep.
pub fn checkpoint_now(m: &mut Model, fx: &mut Effects<YSheets>, bar: Option<&str>) -> bool {
    let unentered = !m.for_mind && bar.is_some_and(|text| text != m.sheet().raw(m.active));
    if m.given_up || !(m.modified() || unentered) {
        return false;
    }
    let name = m.doc.draft.clone();
    fx.checkpoint_draft(&name, draft_of(m, bar.filter(|_| !m.for_mind)).bytes());
    m.draft_asked();
    true
}

/// A draft reached the disk. The lane says so; nothing waits on it.
pub fn saved(_: &mut Model, _: &str) {}

/// A draft could not be written: recovery after a crash is not protected, and the person is told.
pub fn not_saved(m: &mut Model, name: &str, why: &str) {
    say(m, format!("Draft recovery failed: {why}"));
    if m.doc.draft == name {
        let seq = m.d().seq;
        m.draft_failed_at.insert(name.to_string(), seq);
        m.draft_failed(why);
    }
}

/// The sheet is gone: its draft with it.
pub fn forget(m: &mut Model, fx: &mut Effects<YSheets>, name: &str) {
    fx.stop_timer(TIMER);
    fx.discard_draft(name);
    m.draft_failed_at.remove(name);
}

/// The first command: the sheet's file watched, when it came back from a draft with one.
pub fn started(m: &mut Model, fx: &mut Effects<YSheets>) {
    if let Some((key, path, seen)) = m.d().watch_entry() {
        fx.watch(&key, path, seen);
    }
}
