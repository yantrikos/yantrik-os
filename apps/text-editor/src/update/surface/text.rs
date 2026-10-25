//! The calls about the tab in front's text: `find`, `find-next`, `find-prev`, `replace_text`,
//! `replace`, `replace-all`, `set_content`, `append`, `undo` and `redo`. Each refuses rather than
//! report a change that did not happen, and counts what did.

use serde_json::json;
use yantrik_app::effects::Effects;

use super::answers::now;
use super::{no_dialog, not_hidden};
use crate::caller::Caller;
use crate::files::answer::ReadBack;
use crate::model::{Cue, Model};
use crate::text::limits::validate;
use crate::update::{editing, find as bar};
use crate::TextEditor;

type Done = Result<Option<ReadBack>, String>;

/// `find`: the find bar opened on this text, and the first match selected.
pub fn find(m: &mut Model, text: String, caller: &Caller) -> Done {
    not_hidden(m, caller)?;
    no_dialog(m, "find")?;
    m.find.query = text.clone();
    m.cues.push(Cue::ShowFind);
    bar::search(m, true);
    Ok(Some(ReadBack::new(json!({
        "query": text,
        "matches": m.find.matches.len(),
        "at_match": bar::at_match(m),
        "in": m.title(m.active_tab()),
    }))))
}

/// `find-next`, `find-prev`: the selection moved to the next match, or the one before.
pub fn step(m: &mut Model, forward: bool, caller: &Caller) -> Done {
    let name = if forward { "find-next" } else { "find-prev" };
    not_hidden(m, caller)?;
    no_dialog(m, name)?;
    if m.find.matches.is_empty() {
        return Err(format!(
            "Nothing matches {:?} in this tab, so there is no match to step to. `find` sets the query.",
            m.find.query
        ));
    }
    bar::step(m, forward);
    Ok(Some(ReadBack::new(json!({
        "query": m.find.query,
        "at_match": bar::at_match(m),
        "matches": m.find.matches.len(),
        "in": m.title(m.active_tab()),
    }))))
}

/// `replace_text`: what `replace` and `replace-all` put in place of a match.
pub fn replace_text(m: &mut Model, text: String, caller: &Caller) -> Done {
    not_hidden(m, caller)?;
    m.find.replacement = text.clone();
    m.cues.push(Cue::Replacement);
    Ok(Some(ReadBack::new(json!({
        "replacement": text,
        "query": m.find.query,
        "matches_waiting": m.find.matches.len(),
        "in": m.title(m.active_tab()),
    }))))
}

/// `replace`, `replace-all`: the match the selection is on, or every match, replaced; the answer
/// counts what changed.
pub fn replace(m: &mut Model, fx: &mut Effects<TextEditor>, all: bool, caller: &Caller) -> Done {
    let name = if all { "replace-all" } else { "replace" };
    not_hidden(m, caller)?;
    no_dialog(m, name)?;
    let count = m.find.matches.len();
    let intended = if all { count } else { count.min(1) };
    if intended == 0 {
        return Err(format!(
            "Nothing matches {:?} in this tab, so there is nothing to replace. `find` sets the query and \
             `replace_text` sets what it becomes.",
            m.find.query
        ));
    }
    let before = m.active_text().to_string();
    let replaced = bar::replaced(m, all, &m.find.replacement)?;
    editing::put(m, fx, replaced, &caller.opener)?;
    if m.active_text() == before {
        return Err(if m.notice.is_empty() { "Nothing was replaced.".into() } else { m.notice.read(caller.agent).to_string() });
    }
    let mut answer = now(m, caller);
    answer.value["replaced"] = json!(intended);
    answer.value["with"] = json!(m.find.replacement);
    answer.value["matches_left"] = json!(m.find.matches.len());
    Ok(Some(answer))
}

/// `set_content`: everything in the tab in front replaced with this text.
pub fn set_content(m: &mut Model, fx: &mut Effects<TextEditor>, text: String, caller: &Caller) -> Done {
    not_hidden(m, caller)?;
    no_dialog(m, "set_content")?;
    validate(&text)?;
    editing::put(m, fx, text, &caller.opener)?;
    Ok(Some(now(m, caller)))
}

/// `append`: this text added to the end of the tab in front, nothing in it changed.
pub fn append(m: &mut Model, fx: &mut Effects<TextEditor>, add: String, caller: &Caller) -> Done {
    not_hidden(m, caller)?;
    no_dialog(m, "append")?;
    if add.is_empty() {
        return Err("`append` needs `text`: what to add to the end of the tab.".into());
    }
    let text = format!("{}{add}", m.active_text());
    validate(&text)?;
    editing::put(m, fx, text, &caller.opener)?;
    Ok(Some(now(m, caller)))
}

/// `undo`, `redo`: the tab in front stepped through its history; refused rather than report a step
/// that did not happen.
pub fn history(m: &mut Model, fx: &mut Effects<TextEditor>, redo: bool, caller: &Caller) -> Done {
    let name = if redo { "redo" } else { "undo" };
    not_hidden(m, caller)?;
    no_dialog(m, name)?;
    let before = m.active_text().to_string();
    let stepped = editing::history(m, fx, redo, &caller.opener)?;
    if !stepped || m.active_text() == before {
        return Err(format!("There is nothing left to {name} in this tab; it is unchanged."));
    }
    Ok(Some(now(m, caller)))
}
