//! Changing the open note: typing, its sidecar fields, pinning, undo and redo. Every change is
//! an edit of its document; a note in the Trash is never changed.

use yantrik_app::effects::Effects;

use super::{refreshed, said, saving};
use crate::model::Model;
use crate::pane::Cue;
use crate::store::{validate, VAULT_LIMIT};
use crate::undo::Stepped;
use crate::view::render::{CONTENT, CUES, STATUS};
use crate::Notes;

/// How much of a field the sidecar keeps.
const NOTEBOOK_CHARS: usize = 80;
const TAGS_CHARS: usize = 512;

/// The editor's text is now `text`. Refused, with the editor put back, when the library would
/// pass its limit or the text is not one a note may hold.
pub fn edit(m: &mut Model, fx: &mut Effects<Notes>, text: String) {
    let refusal = if m.text_beside_current() + text.len() > VAULT_LIMIT {
        Some("Library reached its 32 MiB text limit.".to_string())
    } else {
        validate(&text).err()
    };
    if let Some(sentence) = refusal {
        m.notice = sentence;
        fx.changed(STATUS);
        if m.current().is_some() {
            fx.changed(CONTENT);
        }
        return;
    }
    let Some((doc, file)) = m.current_mut() else { return };
    if file.trash || doc.content.text == text {
        return;
    }
    let mut previous = String::new();
    doc.edit(|c| previous = std::mem::replace(&mut c.text, text));
    // This window's history, against the text as this edit left it.
    if let Some(d) = m.current_id().and_then(|id| m.documents.get(id)) {
        m.pane.history.edited(previous, &d.content.text);
    }
    refreshed(m, fx);
}

/// The notebook and tags fields say these now.
pub fn metadata(m: &mut Model, fx: &mut Effects<Notes>, notebook: String, tags: String) {
    let notebook_kept: String = notebook.chars().take(NOTEBOOK_CHARS).collect();
    let tags_kept: String = tags.chars().take(TAGS_CHARS).collect();
    m.pane.notebook = notebook;
    m.pane.tags = tags;
    let Some((doc, file)) = m.current_mut() else { return };
    if file.trash {
        return;
    }
    doc.edit(|c| {
        c.set_field("notebook", &notebook_kept);
        c.set_field("tags", &tags_kept);
    });
    refreshed(m, fx);
}

/// Pins the open note, or unpins it, and saves it.
pub fn pin(m: &mut Model, fx: &mut Effects<Notes>) {
    if let Some((doc, file)) = m.current_mut() {
        if file.trash {
            return;
        }
        let pin = !doc.content.pinned();
        doc.edit(|c| c.set_field("pinned", if pin { "true" } else { "false" }));
    }
    refreshed(m, fx);
    saving::save(m, fx, saving::How::Asked);
}

/// What an undo or redo came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Undone {
    Stepped,
    /// Nothing to take back (or put back) in this window.
    Nothing,
    /// The note was changed in another window since: this window's history was dropped.
    Dropped,
}

/// One step back through this view's history of the open note, or forward again. A history the
/// other window's edits overtook is dropped, and said, rather than stepped over them.
pub fn undo(m: &mut Model, fx: &mut Effects<Notes>, redo: bool) -> Undone {
    let Some(now) = m.current().filter(|c| !c.trash()).map(|c| c.text().to_string()) else { return Undone::Nothing };
    let text = match m.pane.history.step(redo, now) {
        Stepped::To(text) => text,
        Stepped::Nothing => return Undone::Nothing,
        Stepped::Dropped => {
            m.notice = said::UNDO_DROPPED.into();
            m.lasting_notice = false;
            fx.changed(STATUS);
            return Undone::Dropped;
        }
    };
    let caret = text.len() as i32;
    if let Some((doc, _)) = m.current_mut() {
        doc.edit(|c| c.text = text);
    }
    m.pane.cues.push(Cue::CaretAt(caret));
    fx.changed(CONTENT);
    fx.changed(CUES);
    refreshed(m, fx);
    Undone::Stepped
}
