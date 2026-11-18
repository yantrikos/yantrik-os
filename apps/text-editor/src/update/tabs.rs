//! Opening, choosing and removing tabs.

use yantrik_app::app::WindowId;
use yantrik_app::documents::{Fingerprint, Source};
use yantrik_app::effects::Effects;

use super::find;
use crate::files::drafts::now_ms;
use crate::files::open::Opened;
use crate::model::{Cue, Model};
use crate::owner::Opener;
use crate::prompt;
use crate::tab::{Tab, TabId};
use crate::view::render::TEXT;
use crate::TextEditor;

/// A new, empty tab with no file, at the end of the strip. Not brought forward.
pub fn blank(m: &mut Model, opener: Opener) -> TabId {
    let (id, draft) = m.next_ids();
    let doc = m.documents.open(Source::New(draft.clone()), String::new(), None);
    push(m, id, doc, None, opener, draft, false, None)
}

/// A tab on the file `opened` read, at the end of the strip, its file watched from what was read.
pub fn of_file(m: &mut Model, fx: &mut Effects<TextEditor>, opened: Opened, opener: Opener) -> TabId {
    let (id, draft) = m.next_ids();
    let baseline = Fingerprint::of_bytes(opened.text.as_bytes());
    let doc = m.documents.open(Source::File(opened.path.clone()), opened.text.clone(), Some(baseline));
    if let Some((key, path, seen)) = m.documents.get(doc).and_then(|d| d.watch_entry()) {
        fx.watch(&key, path, seen);
    }
    push(m, id, doc, Some(opened.text), opener, draft, opened.hidden, opened.inode)
}

#[allow(clippy::too_many_arguments)]
fn push(
    m: &mut Model,
    id: TabId,
    doc: yantrik_app::documents::DocumentId,
    on_disk: Option<String>,
    opener: Opener,
    draft: String,
    hidden: bool,
    inode: Option<crate::tab::Inode>,
) -> TabId {
    let view = m.documents.show(doc, WindowId::PRIMARY);
    m.tabs.push(Tab {
        id,
        doc,
        view,
        on_disk,
        recovered: false,
        opened_by: opener,
        history: Default::default(),
        hidden,
        draft,
        opened_at: now_ms(),
        inode,
        unsure: None,
    });
    id
}

/// Brings tab `index` forward, at its start, with the find bar's matches counted in it.
pub fn select(m: &mut Model, fx: &mut Effects<TextEditor>, index: usize) {
    if index >= m.tabs.len() {
        return;
    }
    m.active = index;
    m.cues.push(Cue::ResetPosition);
    m.cues.push(Cue::FocusEditor);
    find::search(m, false);
    fx.changed(TEXT);
}

/// The strip's tab, clicked: not while a file is read or written, or a dialog asks.
pub fn select_from_window(m: &mut Model, fx: &mut Effects<TextEditor>, id: TabId) {
    if m.busy.is_some() || prompt::dialog(m) != 0 {
        return;
    }
    if let Some(index) = m.index_of(id) {
        select(m, fx, index);
    }
}

/// The next tab, or the one before, round the strip.
pub fn step(m: &mut Model, fx: &mut Effects<TextEditor>, forward: bool) {
    let n = m.tabs.len();
    let index = (m.active + if forward { 1 } else { n - 1 }) % n;
    select(m, fx, index);
}

/// Takes away the empty tab a file opened in place of: it held nothing, so nothing goes with it.
pub fn drop_blank(m: &mut Model, fx: &mut Effects<TextEditor>, id: TabId) {
    let Some(index) = m.index_of(id) else { return };
    let tab = m.tabs.remove(index);
    m.documents.close_document(tab.doc);
    super::drafts::forget(m, fx, &tab.draft);
}

/// Removes a tab: its file no longer watched, its draft gone, its document closed. The last tab
/// closed leaves an empty one, as it always did.
pub fn remove(m: &mut Model, fx: &mut Effects<TextEditor>, id: TabId) {
    let Some(index) = m.index_of(id) else { return };
    let tab = m.tabs.remove(index);
    if let Some(d) = m.documents.close_document(tab.doc) {
        fx.unwatch(&d.source.key());
        m.last_closed = d.source.path().map(|p| (id, p.to_path_buf()));
    }
    super::drafts::forget(m, fx, &tab.draft);
    if index < m.active {
        m.active -= 1;
    }
    if m.tabs.is_empty() {
        blank(m, Opener::Person);
    }
    m.active = m.active.min(m.tabs.len() - 1);
    m.cues.push(Cue::ResetPosition);
    m.cues.push(Cue::FocusEditor);
    find::search(m, false);
    fx.changed(TEXT);
}
