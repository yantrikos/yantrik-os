//! Changing a tab's text: typing, a replace, `append`, `set_content`, undo and redo. Every edit is
//! held to one document's bounds and to the open tabs' budget, not just opening a tab (security
//! review of #620); a refused edit leaves the tab as it was and the window shows it again.

use yantrik_app::effects::Effects;

use super::{find, say};
use crate::model::{Cue, Model};
use crate::owner::{self, Opener};
use crate::text::budget::fits;
use crate::text::limits::validate;
use crate::view::render::TEXT;
use crate::TextEditor;

/// Said when an edit arrives while a file is read or written.
const BUSY: &str = "The editor is reading or writing a file; the tab is unchanged.";

/// The person typed: the editor field already shows `text`.
pub fn typed(m: &mut Model, fx: &mut Effects<TextEditor>, text: String) {
    // A refusal is in the notice, and the view is put back.
    let _ = put(m, fx, text, &Opener::Person);
}

/// Puts `text` in the tab in front, or says why not: the refusal is left in the notice, the view
/// is put back to what is stored, and the same sentence is returned for a caller to pass on.
pub fn put(m: &mut Model, fx: &mut Effects<TextEditor>, text: String, me: &Opener) -> Result<(), String> {
    let refused = if m.busy.is_some() {
        Some(BUSY.to_string())
    } else {
        validate(&text)
            .err()
            .or_else(|| (!fits(&m.weighed(), m.active, text.len())).then(|| owner::too_big(&m.facts(), me)))
    };
    if let Some(sentence) = refused {
        m.cues.push(Cue::ResetContent);
        fx.changed(TEXT);
        say(m, sentence.clone());
        return Err(sentence);
    }
    let tab = m.active_tab().clone();
    if m.text(&tab) == text {
        return Ok(());
    }
    let before = m.text(&tab).to_string();
    if let Some(t) = m.tab_mut(tab.id) {
        t.history.record(before);
    }
    if let Some(d) = m.doc_mut(tab.doc) {
        d.edit(|t| *t = text);
    }
    find::search(m, false);
    fx.changed(TEXT);
    Ok(())
}

/// Takes back the last change to the tab in front (`redo`: puts back what undo took). The text it
/// restores is held to the budget too, since it may be larger than the tab is now. `Ok(false)`
/// when there was nothing to step to.
pub fn history(m: &mut Model, fx: &mut Effects<TextEditor>, redo: bool, me: &Opener) -> Result<bool, String> {
    let tab = m.active_tab().clone();
    let Some(next) = tab.history.next(redo) else { return Ok(false) };
    if !fits(&m.weighed(), m.active, next.len()) {
        let sentence = owner::too_big(&m.facts(), me);
        say(m, sentence.clone());
        return Err(sentence);
    }
    let now = m.text(&tab).to_string();
    let Some(text) = m.tab_mut(tab.id).and_then(|t| t.history.step(redo, now)) else { return Ok(false) };
    if let Some(d) = m.doc_mut(tab.doc) {
        d.edit(|t| *t = text);
    }
    m.cues.push(Cue::ResetPosition);
    m.cues.push(Cue::FocusEditor);
    find::search(m, false);
    fx.changed(TEXT);
    Ok(true)
}

/// The caret moved to `offset` in the text on screen: its line and column, from 1.
pub fn cursor(m: &mut Model, offset: i32) {
    let text = m.view_text();
    let end = (offset.max(0) as usize).min(text.len());
    if let Some(prefix) = text.get(..end) {
        let line = prefix.bytes().filter(|b| *b == b'\n').count() as i32 + 1;
        let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() as i32 + 1;
        m.cursor = (line, column);
    }
}
