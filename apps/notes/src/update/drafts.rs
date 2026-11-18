//! Each open note's draft, kept while it has edits not on disk (design §2, "Drafts are per
//! document, ordered by `seq`"). A note two windows show is one document, so it has one draft.
//!
//! After every command each document is asked whether a draft is owed: one is, when it is dirty
//! and its last draft is older than its edits. The framework writes it through its drafts lane
//! after a debounce, so typing is not a write per key, and a newer draft of the same note takes
//! the place of one still waiting. It is removed once a save of the same edits or later ones has
//! landed. A draft that could not be written is said, because recovery after a crash is then not
//! protected.

use std::path::Path;

use yantrik_app::documents::draft::ENVELOPE_VERSION;
use yantrik_app::documents::{Document, DocumentId, DraftState, Envelope, Fingerprint, OutsideState, Source};
use yantrik_app::effects::Effects;

use crate::model::{Model, NoteFile};
use crate::note::NoteText;
use crate::recovery::base_of;
use crate::update::said;
use crate::view::render::STATUS;
use crate::Notes;

/// Brings every open note's draft up to date: kept when owed, removed when its edits are saved. A
/// draft that could not be written is tried again at the next edit, not at every command: the
/// failure itself arrives as a command, and must not ask for the same write again.
pub fn sync(m: &mut Model, fx: &mut Effects<Notes>) {
    let open: Vec<DocumentId> = m.documents.iter().map(|d| d.id).collect();
    for id in open {
        sync_one(m, fx, id);
    }
}

fn sync_one(m: &mut Model, fx: &mut Effects<Notes>, id: DocumentId) {
    let Some(d) = m.documents.get_mut(id) else { return };
    if d.draft_superseded() {
        fx.discard_draft(&d.source.key());
        d.draft_discarded();
        return;
    }
    if m.draft_failed_at == Some((id, d.seq)) {
        return;
    }
    let Some(file) = m.files.get(&id) else { return };
    if let Some((name, bytes)) = owed(d, file) {
        fx.save_draft(&name, bytes);
        d.draft_asked();
    }
}

/// The draft owed for this document now, if one is (the framework's rule, `draft_owed`).
///
/// Its baseline says what the draft was written over, and a start puts the draft back cleanly
/// only when the files still match it. While the document is in step with what `on_disk` says
/// (nothing outside it, and its own baseline is `on_disk`'s text), that is the note's text and
/// sidecar together (`recovery::base_of`), so the person's own sidecar change restores cleanly.
/// Otherwise `on_disk` is not what the edits were written over: a conflict held by
/// `follow::hold` records the other program's version there, and a file gone records none.
/// Then the document's own text-only baseline is kept, which never equals a `base_of` value, so
/// the draft comes back as the conflict (or the deletion) it was, never as a clean replace. A
/// save that left our sidecar unsure (`meta_unsure`) errs the same way: the file's sidecar is
/// not `on_disk`'s, so the two cannot match.
pub(super) fn owed(d: &Document<NoteText>, file: &NoteFile) -> Option<(String, Vec<u8>)> {
    let (name, bytes) = d.draft_owed(now_ms())?;
    let Some(path) = d.source.path() else { return Some((name, bytes)) };
    let text_of = |t: &NoteText| Fingerprint::of_bytes(t.text.as_bytes());
    let in_step = d.outside == OutsideState::Clean && d.baseline == file.on_disk.as_ref().map(text_of);
    let baseline = if in_step { base_of(file.on_disk.as_ref()) } else { d.baseline.clone() };
    envelope(path, &d.content, d.seq, baseline)
}

/// A document is closing: a draft it kept is removed when nothing in it is unsaved. A document
/// closed with edits not on disk keeps its draft, so a start can offer them back.
pub fn closing(document: &Document<NoteText>, fx: &mut Effects<Notes>) {
    if document.draft != DraftState::None && !document.dirty() {
        fx.discard_draft(&document.source.key());
    }
}

/// The person chose to drop a document's edits: its draft goes with them.
pub fn discard(document: &mut Document<NoteText>, fx: &mut Effects<Notes>) {
    fx.discard_draft(&document.source.key());
    document.draft_discarded();
}

/// The name of the draft a document has kept, if it has kept one: the draft to remove once its
/// text has moved to another note ([`moved`]).
pub fn kept_name(document: &Document<NoteText>) -> Option<String> {
    (document.draft != DraftState::None).then(|| document.source.key())
}

/// The text of the note whose draft is `from` now lives in the note the focused window shows (a
/// copy, *Save as new*, a recovered note): one copy of it is kept for a start to offer back, never
/// none. The focused note's draft is written at once when it has edits not on disk (the old draft
/// is, in effect, renamed to it), and the old draft is removed only once that one is on disk
/// (`DraftSaved`) or the note is saved ([`released`]): a draft write that fails leaves the old
/// one in place. When the focused note has nothing unsaved, its text is on disk already, and the
/// old draft goes now. A crash between the two leaves both, and a start offers the text once
/// (`recovery::weigh`). While another window still shows the old note, its document keeps its own
/// draft for the edits it holds.
pub fn moved(m: &mut Model, fx: &mut Effects<Notes>, from: Option<String>) {
    let mut taken_over_by = None;
    if let Some(id) = m.current_id() {
        if let (Some(d), Some(file)) = (m.documents.get_mut(id), m.files.get(&id)) {
            if let Some((name, bytes)) = owed(d, file) {
                fx.checkpoint_draft(&name, bytes);
                d.draft_asked();
                taken_over_by = Some(name);
            }
        }
    }
    let Some(from) = from.filter(|key| m.documents.by_key(key).is_none()) else { return };
    match taken_over_by {
        Some(name) => m.handed_over.push((name, from)),
        None => fx.discard_draft(&from),
    }
}

/// The note whose draft is `name` is safe: its draft reached the disk, or the note itself was
/// saved. The drafts it took over are removed now.
pub fn released(m: &mut Model, fx: &mut Effects<Notes>, name: &str) {
    let (done, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut m.handed_over).into_iter().partition(|(to, _)| to == name);
    m.handed_over = waiting;
    for (_, from) in done {
        if m.documents.by_key(&from).is_none() {
            fx.discard_draft(&from);
        }
    }
}

/// The latest draft of `name` reached the disk: a notice that said it could not is answered.
pub fn saved(m: &mut Model, fx: &mut Effects<Notes>, name: &str) {
    released(m, fx, name);
    let open = m.documents.by_key(name).is_some();
    if open && m.notice.starts_with(said::DRAFT_NOT_KEPT_START) {
        m.notice.clear();
        fx.changed(STATUS);
    }
}

/// A draft of `name` could not be written. Said when it is an open note's (in any window) and its
/// edits are still not on disk; a draft of a note already saved or closed protects nothing now.
pub fn not_saved(m: &mut Model, fx: &mut Effects<Notes>, name: &str, why: &str) {
    let Some(id) = m.documents.by_key(name) else { return };
    let Some(d) = m.documents.get_mut(id) else { return };
    if !d.dirty() {
        return;
    }
    let sentence = said::draft_not_kept(why);
    d.draft_failed(sentence.clone());
    m.draft_failed_at = Some((id, d.seq));
    m.notice = sentence;
    m.lasting_notice = false;
    fx.changed(STATUS);
}

/// A draft of the note at `path`, of edit `seq`, written over the files `baseline` names
/// (`None`: none, so putting it back creates the file): its name and bytes, in the envelope a
/// start reads. Also for a note that is no open document (another program's version that could
/// not be written as a note).
pub(super) fn envelope(
    path: &Path,
    content: &NoteText,
    seq: u64,
    baseline: Option<Fingerprint>,
) -> Option<(String, Vec<u8>)> {
    let source = Source::File(path.to_path_buf());
    let envelope = Envelope {
        version: ENVELOPE_VERSION,
        key: source.key(),
        path: Some(path.display().to_string()),
        seq,
        at: now_ms(),
        content: content.clone(),
        baseline,
    };
    Some((source.key(), serde_json::to_vec(&envelope).ok()?))
}

/// Milliseconds since the epoch, as a draft's envelope counts them.
pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
