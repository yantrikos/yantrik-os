//! What a call is answered with: the tab it acted on as `document_now` reports it, read back off
//! the disk, and for the calls whose work was a job (`open`, `save`, `save_as`), built when that
//! job lands, from what it did.

use std::path::PathBuf;

use serde_json::{json, Value};
use yantrik_app::effects::Effects;

use super::answer_with;
use crate::caller::Caller;
use crate::command::AfterSave;
use crate::files::answer::{Fill, ReadBack, Require};
use crate::model::{Model, HIDDEN_TAB};
use crate::reading;
use crate::tab::{Tab, TabId};
use crate::text::{language::language, lines, viewport};
use crate::TextEditor;

/// The tab's name as this caller may see it.
pub fn shown_title(m: &Model, tab: &Tab, caller: &Caller) -> String {
    if caller.agent && tab.hidden {
        HIDDEN_TAB.into()
    } else {
        m.title(tab)
    }
}

/// The tab's path as this caller may see it.
pub fn shown_path(m: &Model, tab: &Tab, caller: &Caller) -> Value {
    if caller.agent && tab.hidden {
        json!(HIDDEN_TAB)
    } else {
        json!(m.path(tab).map(|p| p.display().to_string()))
    }
}

/// What the tab in front now is: every action that touches a document answers with this, and the
/// disk is read into it, so "it worked" is never the editor's opinion of its own handler:
/// `matches_disk` is the file compared against the text in the tab.
pub fn document_now(m: &Model, caller: &Caller) -> (Value, Fill) {
    let tab = m.active_tab();
    let text = m.text(tab);
    let hidden = caller.agent && tab.hidden;
    let view = m.viewport();
    let value = json!({
        "title": shown_title(m, tab, caller),
        "path": shown_path(m, tab, caller),
        "tab": m.active,
        "tabs": m.tabs.len(),
        // How long a hidden file is, is something about it too.
        "lines": (!hidden).then(|| lines(text)),
        "characters": (!hidden).then(|| text.chars().count()),
        "bytes": (!hidden).then_some(text.len()),
        "modified": m.modified(tab),
        "opened_by": tab.opened_by.label(),
        "opened_by_you": caller.opener.owns(&tab.opened_by),
        "on_disk": false,
        "matches_disk": false,
        "language": language(m.path(tab)),
        "notice": m.notice.read(caller.agent),
        // Empty unless the tab is too big to draw whole and the person sees only its first lines
        // (#328); the counts above are the whole document's either way.
        "view_status": if hidden { String::new() } else { viewport::status(view, text) },
    });
    let fill = Fill::Tab { at: None, path: m.path(tab).map(PathBuf::from), text: text.to_string() };
    (value, fill)
}

/// `document_now`, read back, as the whole answer.
pub fn now(m: &Model, caller: &Caller) -> ReadBack {
    let (value, fill) = document_now(m, caller);
    ReadBack::new(value).fill(fill)
}

/// `value` with `document_now` under `now`.
pub fn with_now(m: &Model, caller: &Caller, mut value: Value) -> ReadBack {
    let (now, fill) = document_now(m, caller);
    value["now"] = now;
    let Fill::Tab { path, text, .. } = fill else { unreachable!("document_now reads a tab back") };
    ReadBack::new(value).fill(Fill::Tab { at: Some("now"), path, text })
}

/// `open` landed: the tab it opened, and when its text is longer than `describe.content` holds,
/// the call that reads the rest (VM 520, 4 October: the 9.5 KB spec).
pub fn opened(m: &mut Model, fx: &mut Effects<TextEditor>, caller: &Caller) {
    let mut answer = now(m, caller);
    if !(caller.agent && m.active_tab().hidden) {
        let cut = reading::describe_cut(m.active_text());
        answer.value["content_cut"] = json!(cut.is_some());
        answer.value["read_with"] = json!(cut);
    }
    answer_with(m, fx, answer, caller.agent);
}

/// A save a call asked for landed: what is on disk, read back.
pub fn saved(m: &mut Model, fx: &mut Effects<TextEditor>, then: &AfterSave, id: TabId) {
    match then {
        AfterSave::Window => {}
        AfterSave::Save { caller, closing } => {
            // Read back through the tab that held that path: a save that closed its tab has moved
            // the one in front, and answering about it would be a different document.
            let path = m.tab(id).and_then(|t| m.path(t)).map(PathBuf::from).or_else(|| saved_path(m, id));
            let Some(path) = path else { return };
            let value = json!({
                "path": path.display().to_string(),
                "saved": true,
                "closed_the_tab": closing,
                "tabs": m.tabs.len(),
            });
            let answer = with_now(m, caller, value)
                .fill(Fill::Written { path })
                .require(Require::Written("The file was not written; the draft is still open.".into()));
            answer_with(m, fx, answer, caller.agent);
        }
        AfterSave::SaveAs { caller, path } => {
            let answer = now(m, caller).require(Require::Matches(format!("Nothing was written to {}.", path.display())));
            answer_with(m, fx, answer, caller.agent);
        }
    }
}

/// The path a tab was saved to before its close removed it: kept by the close for this answer.
fn saved_path(m: &Model, id: TabId) -> Option<PathBuf> {
    m.last_closed.as_ref().filter(|(t, _)| *t == id).map(|(_, p)| p.clone())
}
