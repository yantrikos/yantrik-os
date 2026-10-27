//! Putting a document in front: a file just read, or a new empty one. The one before it is
//! closed, its file no longer watched and its draft gone. Callers only do this when that is meant:
//! the person saved it, or gave it up on the unsaved prompt, or a call said `discard`.

use yantrik_app::documents::{Fingerprint, Source};
use yantrik_app::effects::Effects;

use super::{drafts, find};
use crate::files::open::Opened;
use crate::model::{self, Cue, Model};
use crate::view::render::TEXT;
use crate::YDoc;

/// The document in front closed: unwatched, its draft gone, its view and the table's entry too.
fn close_current(m: &mut Model, fx: &mut Effects<YDoc>) {
    let name = m.doc.draft.clone();
    if let Some(d) = m.documents.close_document(m.doc.id) {
        fx.unwatch(&d.source.key());
    }
    drafts::forget(m, fx, &name);
    m.reread_owed = false;
}

/// The cues every new document in front starts with: the page at its start, the caret there.
fn arrived(m: &mut Model, fx: &mut Effects<YDoc>) {
    m.cues.push(Cue::ResetContent);
    m.cues.push(Cue::Select(0, 0));
    m.cues.push(Cue::FocusBody);
    find::search(m, false);
    fx.changed(TEXT);
}

/// A new, empty document in front.
pub fn blank(m: &mut Model, fx: &mut Effects<YDoc>) {
    close_current(m, fx);
    let draft = m.next_draft();
    m.doc = model::blank(&mut m.documents, draft);
    arrived(m, fx);
}

/// The file `opened` in front, watched from what was read.
pub fn of_file(m: &mut Model, fx: &mut Effects<YDoc>, opened: Opened) {
    close_current(m, fx);
    let draft = m.next_draft();
    let baseline = Fingerprint::of_bytes(opened.text.as_bytes());
    let id = m.documents.open(Source::File(opened.path.clone()), opened.text.clone(), Some(baseline));
    if let Some((key, path, seen)) = m.documents.get(id).and_then(|d| d.watch_entry()) {
        fx.watch(&key, path, seen);
    }
    m.doc = model::place(&mut m.documents, id, Some(opened.text), draft, opened.hidden, opened.inode);
    arrived(m, fx);
}
