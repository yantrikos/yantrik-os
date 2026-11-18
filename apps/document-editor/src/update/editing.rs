//! Changing the document's text: typing, a replace, a format button, `append`, `set_content`.
//! Every edit is held to one document's bounds; a refused edit leaves the document as it was and
//! the window shows it again.

use yantrik_app::effects::Effects;

use super::{find, say};
use crate::limits::validate;
use crate::model::{Cue, Model};
use crate::view::render::TEXT;
use crate::YDoc;

/// Said when an edit arrives while a file is read or written.
const BUSY: &str = "yDoc is reading or writing a file; the document is unchanged.";

/// The person typed: the page already shows `text`.
pub fn typed(m: &mut Model, fx: &mut Effects<YDoc>, text: String) {
    // A refusal is in the notice, and the page is put back.
    let _ = put(m, fx, text);
}

/// Puts `text` in the document, or says why not: the refusal is left in the notice, the page is
/// put back to what is stored, and the same sentence is returned for a caller to pass on.
pub fn put(m: &mut Model, fx: &mut Effects<YDoc>, text: String) -> Result<(), String> {
    let refused = if m.busy.is_some() { Some(BUSY.to_string()) } else { validate(&text).err() };
    if let Some(sentence) = refused {
        m.cues.push(Cue::ResetContent);
        fx.changed(TEXT);
        say(m, sentence.clone());
        return Err(sentence);
    }
    if m.text() == text {
        return Ok(());
    }
    m.d_mut().edit(|t| *t = text);
    find::search(m, false);
    fx.changed(TEXT);
    Ok(())
}

/// Replaces the text from somewhere other than the keyboard, and puts the caret where the change
/// left it (`selection`), or leaves it.
pub fn commit(
    m: &mut Model,
    fx: &mut Effects<YDoc>,
    text: String,
    selection: Option<(usize, usize)>,
) -> Result<(), String> {
    put(m, fx, text)?;
    if let Some((a, z)) = selection {
        m.cues.push(Cue::Select(a, z));
        m.cues.push(Cue::FocusBody);
    }
    m.notice.clear();
    Ok(())
}
