//! Which note the pane shows: opening one from the library, making one, copying one. Every open
//! note is a document; its file is watched while it is open and no longer once it closes.

use yantrik_app::documents::{Fingerprint, OutsideState, Source};
use yantrik_app::effects::Effects;

use super::{drafts, saving, show};
use crate::model::{Model, NoteFile};
use crate::note::{Kept, Note};
use crate::pane::Cue;
use crate::store::{validate, MAX_NOTES, VAULT_LIMIT};
use crate::view::render::{CHROME, CUES, STATUS};
use crate::Notes;

/// What a new note may be given before its body: room for its heading.
const HEADING_ROOM: usize = 1024;

/// Said when a copy of the open note would pass the library's limits.
pub const LIBRARY_FULL: &str = "Library reached its note or text limit (2,000 notes / 32 MiB).";

/// The focused window shows `note` (or nothing): as the document another window has open for it,
/// or as a document of its own. The document it showed is closed, and its file no longer watched,
/// once no window shows it. A note whose file was gone is then gone from the library too: nothing
/// of it is on disk, and its text was let go or kept as another note before it could close.
pub fn show_note(m: &mut Model, fx: &mut Effects<Notes>, note: Option<Note>) {
    // What the pane held about the last note's outside change is not about this one.
    m.pane.outside_later = false;
    m.pane.reread_owed = false;
    m.pane.theirs = None;
    if m.pane.dialog == super::outside::COMPARE_DIALOG {
        m.pane.dialog = 0;
        fx.changed(CHROME);
    }
    if let Some(view) = m.pane.view.take() {
        if let Some(doc) = m.documents.forget_view(view) {
            if let Some(closed) = m.documents.close_document(doc) {
                if closed.source.path().is_some() {
                    fx.unwatch(&closed.source.key());
                }
                drafts::closing(&closed, fx);
                if let Some(gone) = m.files.get(&doc).filter(|_| closed.outside == OutsideState::Deleted) {
                    let id = gone.id.clone();
                    m.library.retain(|n| n.trash || n.id != id);
                }
            }
            m.files.remove(&doc);
        }
    }
    let Some(note) = note else { return };
    // A note in the Trash has no file of its own: its record is the Trash's, and it is not watched.
    let source = if note.trash {
        Source::New(format!("trash-{}", note.id))
    } else {
        Source::File(m.vault.dir().join(&note.id))
    };
    // A note another window shows is that window's document: this view joins it, unsaved edits,
    // conflict and draft and all, rather than reading it again from the library.
    if let Some(doc) = m.documents.find(&source) {
        m.pane.view = Some(m.documents.show(doc, m.pane.window));
        return;
    }
    let on_disk = note.stored.then(|| note.content.clone());
    let baseline = on_disk
        .as_ref()
        .filter(|_| !note.trash)
        .map(|d| Fingerprint::of_bytes(d.text.as_bytes()));
    let doc = m.documents.open(source, note.content.clone(), baseline);
    if let Some(d) = m.documents.get_mut(doc) {
        if on_disk.is_none() {
            // A note made here is not on disk until its first save lands: it starts unsaved.
            d.edit(|_| ());
        }
        if let Some((key, path, seen)) = d.watch_entry() {
            fx.watch(&key, path, seen);
        }
    }
    m.files.insert(
        doc,
        NoteFile { id: note.id, trash: note.trash, on_disk, meta_unsure: None, modified: note.modified, gone_at: None },
    );
    m.pane.view = Some(m.documents.show(doc, m.pane.window));
}

/// Opens a library row. A file name no longer in the library opens nothing.
pub fn open(m: &mut Model, fx: &mut Effects<Notes>, id: &str, trash: bool) {
    let found = m.library.iter().find(|n| n.id == id && n.trash == trash).cloned();
    if found.is_some() {
        show_note(m, fx, found);
        show(m, fx);
    }
}

/// Starts a note titled `title`, with `body` under the heading, opens it and saves it. Returns
/// its file name; a refusal is said in the notice too.
pub fn new_note(m: &mut Model, fx: &mut Effects<Notes>, title: &str, body: &str) -> Result<String, String> {
    if m.library.len() >= MAX_NOTES || m.library_text() + HEADING_ROOM + body.len() > VAULT_LIMIT {
        return Err(refuse(
            m,
            fx,
            "The library supports up to 2,000 notes and 32 MiB of text. Nothing was created.".into(),
        ));
    }
    let mut note = Note::blank(&title.chars().take(160).collect::<String>());
    if !body.is_empty() {
        note.content.text.push_str(body);
        if !note.content.text.ends_with('\n') {
            note.content.text.push('\n');
        }
    }
    validate(&note.content.text).map_err(|e| refuse(m, fx, e))?;
    let id = note.id.clone();
    let caret = note.content.text.len() as i32;
    m.library.push(note.clone());
    show_note(m, fx, Some(note));
    m.pane.folder = "all".into();
    m.pane.query.clear();
    m.pane.preview = false;
    fx.changed(CHROME);
    show(m, fx);
    m.pane.cues.push(Cue::CaretAt(caret));
    fx.changed(CUES);
    saving::save(m, fx, saving::How::Asked);
    Ok(id)
}

/// The open note, unsaved edits and all, as a new note under its own file name; the original
/// file is left as it is. The way out of "this note changed on disk". The unsaved text is the
/// copy's now, so the original's draft goes with it (`drafts::moved`).
pub fn copy(m: &mut Model, fx: &mut Effects<Notes>) {
    // As the window shows it: an offered text waiting on the person is copied, offer and all.
    let Some(mut note) = m.shown_note() else { return };
    if m.library.len() >= MAX_NOTES || m.library_text() + note.content.text.len() > VAULT_LIMIT {
        m.notice = LIBRARY_FULL.into();
        fx.changed(STATUS);
        return;
    }
    note.id = Kept::Copy.new_id();
    note.stored = false;
    note.trash = false;
    let from = m.current_id().and_then(|id| m.documents.get(id)).and_then(drafts::kept_name);
    m.library.push(note.clone());
    show_note(m, fx, Some(note));
    drafts::moved(m, fx, from);
    show(m, fx);
    saving::save(m, fx, saving::How::Asked);
}

fn refuse(m: &mut Model, fx: &mut Effects<Notes>, sentence: String) -> String {
    m.notice = sentence.clone();
    fx.changed(STATUS);
    sentence
}
