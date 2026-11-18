//! The calls about tabs and files: `new`, `open`, `read`, `save`, `save_as`, `show`, `close`,
//! `cancel`, `discard` and `select_tab`, worded and answered as they always were.

use serde_json::json;
use yantrik_app::documents::Choice;
use yantrik_app::effects::Effects;

use super::answers::{now, shown_path, shown_title, with_now};
use super::{no_dialog, not_hidden};
use crate::caller::Caller;
use crate::command::{AfterOpen, AfterSave};
use crate::files::answer::ReadBack;
use crate::files::expanded;
use crate::model::Model;
use crate::owner;
use crate::prompt;
use crate::reading;
use crate::text::budget::room_for;
use crate::text::limits::validate;
use crate::update::{closing, editing, opening, saving, tabs, window};
use crate::TextEditor;

type Done = Result<Option<ReadBack>, String>;

/// `new`: a new tab in front, empty or holding the text given. The text is checked before the tab
/// opens, and counts against the budget, so text that is refused leaves no empty tab behind.
pub fn new(m: &mut Model, fx: &mut Effects<TextEditor>, text: String, caller: &Caller) -> Done {
    no_dialog(m, "new")?;
    validate(&text)?;
    if !room_for(&m.weighed(), text.len()) {
        return Err(owner::no_room(&m.facts(), &caller.opener));
    }
    let before = m.tabs.len();
    window::new_tab(m, fx, caller.opener.clone());
    if m.tabs.len() == before {
        // The notice the refusal left names the tab this caller can close to make room.
        return Err(m.notice.window().to_string());
    }
    if !text.is_empty() {
        editing::put(m, fx, text, &caller.opener).map_err(|e| format!("The new tab is open but empty: {e}"))?;
    }
    Ok(Some(now(m, caller)))
}

/// `open`: the file read in a job; answered when it lands.
pub fn open(m: &mut Model, fx: &mut Effects<TextEditor>, path: String, caller: Caller) -> Done {
    no_dialog(m, "open")?;
    let full = expanded(path.trim());
    opening::start(m, fx, full, AfterOpen::Surface(caller.clone()), &caller);
    Ok(None)
}

/// `read`: one page of an open tab's text, changing nothing, not even which tab is in front. It
/// takes no `path`: the dispatch refuses one, since a path would reach every text file in the home
/// and a safe read runs unasked.
pub fn read(m: &Model, from_line: usize, tab: Option<i64>, caller: &Caller) -> Done {
    let open = m.tabs.len();
    let index = match tab {
        None => m.active,
        Some(i) if i >= 0 && (i as usize) < open => i as usize,
        Some(i) => return Err(format!("There is no tab {i}; {open} are open, numbered 0 to {}.", open - 1)),
    };
    let t = &m.tabs[index];
    if caller.agent && t.hidden {
        return Err(match tab {
            None => super::HIDDEN_REFUSAL.to_string(),
            Some(i) => format!(
                "Tab {i} holds a file an agent is not shown, and its text is left alone; read another tab or \
                 `new` one."
            ),
        });
    }
    let call = tab.map(|i| format!("tab {i} and ")).unwrap_or_default();
    // Whether the tab holds unsaved changes: a page of an edited buffer is not the file on disk.
    let mut page = reading::page(m.text(t), shown_path(m, t, caller), from_line.max(1), &call);
    page["modified"] = json!(m.modified(t));
    Ok(Some(ReadBack::new(page)))
}

/// `save`: the tab in front written back to its file; while the prompt asks, the answer that
/// writes the tab and closes it.
pub fn save(m: &mut Model, fx: &mut Effects<TextEditor>, caller: Caller) -> Done {
    let answering = prompt::dialog(m) == prompt::PROMPT;
    if !answering {
        no_dialog(m, "save")?;
    }
    let id = m.closing.as_ref().filter(|_| answering).map(|c| c.tab).unwrap_or(m.active_tab().id);
    if m.tab(id).and_then(|t| m.path(t)).is_none() {
        return Err("This tab has never been written anywhere; `save_as` with a path decides where it goes.".into());
    }
    let then = AfterSave::Save { caller: caller.clone(), closing: answering };
    if answering {
        if let Some(c) = m.closing.as_mut() {
            let next = c.flow.answer(Choice::Save);
            closing::apply(m, fx, next, then);
        }
    } else {
        saving::start(m, fx, id, None, false, then, caller.agent);
    }
    Ok(None)
}

/// `save_as`: the tab in front written to this path, and kept on it from then on.
pub fn save_as(m: &mut Model, fx: &mut Effects<TextEditor>, path: String, overwrite: bool, caller: Caller) -> Done {
    if !matches!(prompt::dialog(m), 0 | prompt::SAVE_AS | prompt::PROMPT) {
        no_dialog(m, "save_as")?;
    }
    // The text written is the tab's: saving a tab the person opened from ~/.ssh into ~/Documents
    // would put it where `describe` reads it back.
    not_hidden(m, &caller)?;
    let full = expanded(path.trim());
    let id = m.active_tab().id;
    let then = AfterSave::SaveAs { caller: caller.clone(), path: full.clone() };
    saving::start(m, fx, id, Some(full), overwrite, then, caller.agent);
    Ok(None)
}

/// `show`: the window is the person's already; nothing in any tab is read or changed.
pub fn show(m: &mut Model, _: &mut Effects<TextEditor>, caller: &Caller) -> Done {
    if m.lingering {
        return Err("The editor's window is closed while it finishes a save; launch it again to bring the \
                    window back."
            .into());
    }
    Ok(Some(with_now(m, caller, json!({ "shown": true }))))
}

/// `close`: the tab in front closed, or asked about when it has unsaved changes.
pub fn close(m: &mut Model, fx: &mut Effects<TextEditor>, caller: &Caller) -> Done {
    no_dialog(m, "close")?;
    let tab = m.active_tab().clone();
    let facts = m.facts().swap_remove(m.active);
    owner::may_close(caller.agent, &facts, !tab.history.redo.is_empty())?;
    let (before, title) = (m.tabs.len(), shown_title(m, &tab, caller));
    let asking = closing::close_tab(m, fx, tab.id);
    let value = json!({
        "closed": !asking && m.tabs.len() < before,
        "was": title,
        "awaiting_answer": asking,
        "asking": if asking { prompt::label(m, caller.agent) } else { String::new() },
        "tabs": m.tabs.len(),
    });
    Ok(Some(with_now(m, caller, value)))
}

/// `cancel`: whichever dialog is open goes, and every tab stays exactly as it was.
pub fn cancel(m: &mut Model, _: &mut Effects<TextEditor>, caller: &Caller) -> Done {
    let was = prompt::dialog(m);
    if was == 0 {
        return Err("No dialog is open, so there is nothing to cancel.".into());
    }
    window::cancel(m);
    let value = json!({ "dialog_closed": prompt::dialog(m) == 0, "was_asking": was });
    Ok(Some(with_now(m, caller, value)))
}

/// `discard`: the unsaved changes the prompt asks about thrown away, and the tab closed.
pub fn discard(m: &mut Model, fx: &mut Effects<TextEditor>, caller: &Caller) -> Done {
    if prompt::dialog(m) != prompt::PROMPT {
        return Err("Nothing is waiting to be discarded; `close` asks first, and only a tab with unsaved changes \
                    is ever asked about."
            .into());
    }
    let Some(asked) = m.closing.as_ref().and_then(|c| m.tab(c.tab)).cloned() else {
        return Err("Nothing is waiting to be discarded.".into());
    };
    let (before, title) = (m.tabs.len(), shown_title(m, &asked, caller));
    closing::answer(m, fx, Choice::DontSave);
    let value = json!({
        "discarded": title,
        "closed": m.tabs.len() < before || before == 1,
        "tabs": m.tabs.len(),
    });
    Ok(Some(with_now(m, caller, value)))
}

/// `select_tab`: one of the open tabs brought forward, by its place in `describe.tabs`.
pub fn select_tab(m: &mut Model, fx: &mut Effects<TextEditor>, index: i64, caller: &Caller) -> Done {
    no_dialog(m, "select_tab")?;
    let open = m.tabs.len();
    if index < 0 || index as usize >= open {
        return Err(format!("There is no tab {index}; {open} are open, numbered 0 to {}.", open - 1));
    }
    tabs::select(m, fx, index as usize);
    Ok(Some(now(m, caller)))
}
