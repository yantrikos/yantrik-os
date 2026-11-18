//! A call from the surface, carried out, and its answer.
//!
//! Every answer is built from that call's own work, when it committed: the file it read or wrote,
//! the text it put in. Nothing the person does meanwhile, and no other call, becomes this call's
//! answer: a call that starts a job is settled when that job lands, with what the job found. A
//! refusal is said twice, as it always was: to the caller, and in the window's notice, which is
//! `describe.notice`.

use std::path::Path;

use serde_json::{json, Value};
use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use super::{document, editing, find, opening, saving};
use crate::agent_rule::HIDDEN_REFUSAL;
use crate::caller::Caller;
use crate::command::{Act, AfterExport, AfterOpen, AfterSave};
use crate::files::expanded;
use crate::markdown;
use crate::model::{Cue, Model};
use crate::prompt;
use crate::YDoc;

type Done = Result<Option<Value>, String>;

/// What a call is refused with while the window is asking the person a question.
const ASKING: &str = "yDoc is asking the person about a file. Answer that prompt first.";

pub fn act(m: &mut Model, fx: &mut Effects<YDoc>, act: Act, caller: Caller) {
    // `show` brings the window forward and reads nothing: it is the one call that goes through
    // while a prompt is up.
    if prompt::dialog(m) != prompt::NONE && !matches!(act, Act::Show) {
        return settle_refused(fx, ASKING);
    }
    let done = match act {
        Act::Open { path, discard } => open(m, fx, path, discard, caller),
        Act::New => new(m, fx),
        Act::Save => save(m, fx, caller),
        Act::SaveAs { path, overwrite } => save_as(m, fx, path, overwrite, caller),
        Act::SetContent { text } => set_content(m, fx, text, caller),
        Act::Append { text } => append(m, fx, &text, caller),
        Act::Find { query } => find_text(m, query, caller),
        Act::ReplaceAll { find, with } => replace_all(m, fx, find, with, caller),
        Act::ExportMarkdown { path } => export_markdown(m, fx, path, caller),
        Act::Show => show(m),
    };
    match done {
        Ok(Some(value)) => answer(m, fx, value),
        // Answered when its job lands.
        Ok(None) => {}
        Err(sentence) => refuse(m, fx, sentence),
    }
}

/// The call answered with `value`, in the same command that committed its work.
pub fn answer(m: &mut Model, fx: &mut Effects<YDoc>, value: Value) {
    m.answer = value;
    fx.settle(Ok(()));
}

/// A refusal the person at the window sees too: the notice, and the call's error.
pub fn refuse(m: &mut Model, fx: &mut Effects<YDoc>, sentence: impl Into<String>) {
    let sentence = sentence.into();
    m.notice.set(sentence.clone());
    settle_refused(fx, sentence);
}

/// The call refused with `sentence`, when the notice already says it in the form its caller may
/// read.
pub fn settle_refused(fx: &mut Effects<YDoc>, sentence: impl Into<String>) {
    fx.settle(Err(Refused::new("refused", sentence.into())));
}

/// Refuse an agent anything that reads or changes the document in front when it is one the
/// person opened from a place an agent may not read (#443). Hiding `content` in `describe` was
/// not enough: `find` counts matches, `save_as` copies the text somewhere readable, and `append`
/// then `save` writes it back. The refusal does not name the file: the notice it leaves is in
/// `describe` too.
fn not_hidden(m: &Model, caller: Caller) -> Result<(), String> {
    if caller.agent && m.doc.hidden {
        return Err(HIDDEN_REFUSAL.into());
    }
    Ok(())
}

/// What a refused open says it would have cost.
pub fn would_throw_away(target: &Path, lost: &str) -> String {
    format!(
        "Opening {} would throw away {}. Save this document first, or open again with discard=true.",
        target.display(),
        lost
    )
}

/// `open`: the file read in a job, and the answer built from what it found. Refused, with the
/// size of what it would lose in it, while the document has unsaved changes, unless `discard`.
fn open(m: &mut Model, fx: &mut Effects<YDoc>, path: String, discard: bool, caller: Caller) -> Done {
    let target = expanded(path.trim());
    if !discard {
        if let Some(lost) = m.unsaved(false) {
            let hidden = m.unsaved(true).unwrap_or_default();
            let hide = m.doc.hidden;
            m.notice.set_about(hide, would_throw_away(&target, &lost), || would_throw_away(&target, &hidden));
            settle_refused(fx, m.notice.read(caller.agent).to_string());
            return Ok(None);
        }
    }
    opening::start(m, fx, target, AfterOpen::Surface(caller), caller);
    Ok(None)
}

/// `new`: a blank document, refused while there are unsaved changes: a mind asking for a blank
/// document is told it is about to throw one away rather than have it happen.
fn new(m: &mut Model, fx: &mut Effects<YDoc>) -> Done {
    if m.modified() {
        return Err("This document has unsaved changes. Save it first, or discard them in the window.".into());
    }
    document::blank(m, fx);
    m.notice.clear();
    Ok(Some(json!({ "new": true, "path": Value::Null })))
}

/// `save`: the document written back to its file. A document with no file is refused with a
/// sentence, never in silence.
fn save(m: &mut Model, fx: &mut Effects<YDoc>, caller: Caller) -> Done {
    if m.path().is_none() {
        return Err(format!(
            "This document has no file yet, so there is nowhere to save it. Use Save As, or `save_as` with a path \
             \u{2014} {} would do.",
            m.home().display()
        ));
    }
    saving::start(m, fx, None, false, AfterSave::Save(caller), caller.agent);
    Ok(None)
}

/// `save_as`: the document written to this path, and kept there from then on.
fn save_as(m: &mut Model, fx: &mut Effects<YDoc>, path: String, overwrite: bool, caller: Caller) -> Done {
    // The text written is the document's: saving one the person opened from ~/.ssh into
    // ~/Documents would put it where `describe` reads it back.
    not_hidden(m, caller)?;
    saving::start(m, fx, Some(expanded(path.trim())), overwrite, AfterSave::SaveAs(caller), caller.agent);
    Ok(None)
}

/// `set_content`: the document replaced with this Markdown, unsaved.
fn set_content(m: &mut Model, fx: &mut Effects<YDoc>, text: String, caller: Caller) -> Done {
    not_hidden(m, caller)?;
    let (words, chars) = (markdown::word_count(&text), markdown::char_count(&text));
    editing::commit(m, fx, text, Some((0, 0)))?;
    Ok(Some(json!({ "words": words, "chars": chars, "dirty": true })))
}

/// `append`: Markdown added to the end of the document, unsaved.
fn append(m: &mut Model, fx: &mut Effects<YDoc>, addition: &str, caller: Caller) -> Done {
    not_hidden(m, caller)?;
    let mut text = m.text().to_string();
    // A document is lines. Appending to one that does not end in one would glue the new paragraph
    // onto the end of the last, which for Markdown changes what it means.
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(addition);
    let end = text.len();
    let (words, chars) = (markdown::word_count(&text), markdown::char_count(&text));
    editing::commit(m, fx, text, Some((end, end)))?;
    Ok(Some(json!({ "appended_chars": markdown::char_count(addition), "words": words, "chars": chars })))
}

/// `find`: the find bar opened on this text, and the first match selected.
fn find_text(m: &mut Model, query: String, caller: Caller) -> Done {
    not_hidden(m, caller)?;
    m.find.query = query.clone();
    m.cues.push(Cue::ShowFind);
    find::search(m, true);
    Ok(Some(json!({ "query": query, "matches": m.find.matches.len() })))
}

/// `replace_all`: every match replaced, and how many there were.
fn replace_all(m: &mut Model, fx: &mut Effects<YDoc>, query: String, with: String, caller: Caller) -> Done {
    not_hidden(m, caller)?;
    let (out, count) = markdown::replace_all(m.text(), &query, &with)?;
    m.find.query = query;
    m.find.replacement = with;
    m.cues.push(Cue::FindFields);
    editing::commit(m, fx, out, None)?;
    find::search(m, false);
    Ok(Some(json!({ "replaced": count })))
}

/// `export_markdown`: a copy written to a path, in a job, the document left in its own file.
fn export_markdown(m: &mut Model, fx: &mut Effects<YDoc>, path: String, caller: Caller) -> Done {
    not_hidden(m, caller)?;
    saving::export(m, fx, expanded(path.trim()), false, AfterExport::Surface(caller), caller.agent);
    Ok(None)
}

/// `show`: the window is the framework's to bring forward (`present`); this answers that it is
/// there, or says it is not.
fn show(m: &Model) -> Done {
    if m.lingering {
        return Err("yDoc's window is closed while it finishes a save; launch it again to bring the window back.".into());
    }
    Ok(Some(json!({ "shown": true })))
}
