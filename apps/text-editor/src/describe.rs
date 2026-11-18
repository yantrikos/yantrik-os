//! What a mind reads first: one line that says which tab is in front, how long it is, whether it
//! is saved, how many tabs there are, and what is blocking the next call; and the state behind it,
//! with the same keys and values the editor has always published.
//!
//! A tab whose file is one no agent is shown is named `(hidden)` to an agent, with no text, no
//! length and no match count: a count over a hidden tab answers "does it contain X?" one query at
//! a time.

use serde_json::json;
use yantrik_app::control::View;

use crate::caller::Caller;
use crate::model::{Model, HIDDEN_TAB};
use crate::prompt;
use crate::reading;
use crate::text::{language::language, lines, viewport};
use crate::update::surface::answers::{shown_path, shown_title};

pub fn view(m: &Model) -> View {
    let caller = Caller::now();
    let tab = m.active_tab();
    let text = m.text(tab);
    let hidden = caller.agent && tab.hidden;
    let modified = m.modified(tab);
    let count = lines(text);
    let mut summary = if hidden {
        format!("Text Editor — {HIDDEN_TAB}, {}", if modified { "unsaved" } else { "saved" })
    } else {
        format!(
            "Text Editor — {}{}, {} line{}, {}",
            m.title(tab),
            if m.path(tab).is_none() { " (no file yet)" } else { "" },
            count,
            if count == 1 { "" } else { "s" },
            if modified { "unsaved" } else { "saved" }
        )
    };
    if m.tabs.len() > 1 {
        summary.push_str(&format!(" · tab {} of {}", m.active + 1, m.tabs.len()));
    }
    if hidden {
        summary.push_str(" · its text is not shown to an agent");
    }
    let unsaved = m.tabs.iter().filter(|t| m.modified(t)).count();
    if unsaved > 1 {
        summary.push_str(&format!(" ({unsaved} tabs unsaved)"));
    }
    let dialog = prompt::dialog(m);
    match dialog {
        0 => {}
        prompt::PROMPT => summary.push_str(" · asking about unsaved changes: save, discard or cancel"),
        prompt::OPEN => summary.push_str(" · Open file dialog is up"),
        prompt::SAVE_AS => summary.push_str(" · Save As dialog is up"),
        _ => summary.push_str(" · Go to line dialog is up"),
    }
    if m.busy.is_some() {
        summary.push_str(" · working");
    }
    let notice = m.notice.read(caller.agent).to_string();
    if !notice.is_empty() {
        summary.push_str(&format!(" · {notice}"));
    }
    let cut = if hidden { None } else { reading::describe_cut(text) };
    if cut.is_some() {
        summary.push_str(&format!(
            " · `content` holds its first {} characters; `read` pages the whole text",
            reading::DESCRIBE_CHARS
        ));
    }
    // "Showing the first N lines" is the file's length too.
    let view_status = if hidden { String::new() } else { viewport::status(m.viewport(), text) };
    if !view_status.is_empty() {
        summary.push_str(&format!(" · {view_status}"));
    }
    View::new(summary)
        .with("path", shown_path(m, tab, &caller))
        .with("title", shown_title(m, tab, &caller))
        .with("modified", modified)
        .with("lines", (!hidden).then_some(count as i64))
        .with("characters", (!hidden).then(|| text.chars().count() as i64))
        .with("content", if hidden { String::new() } else { reading::describe_content(text) })
        // Said out loud, with the call that carries on: a cut `content` that passed for the whole
        // file is how a mind on VM 520 (4 October) read 42% of a 9.5 KB spec and gave up.
        .with("content_cut", cut.is_some())
        .with("read_with", cut)
        .with("content_hidden", hidden.then_some("the tab holds a file an agent is not shown"))
        .with("bytes", (!hidden).then_some(text.len()))
        .with("language", language(m.path(tab)))
        // Whose each tab is, and whether it is the reader's: VM 520, 4 October, a mind closed the
        // person's saved tab to make room because nothing said it was not its own.
        .with("tabs", tabs(m, &caller))
        .with("active_tab", m.active)
        .with("busy", m.busy.is_some())
        .with("notice", notice)
        .with("dialog", dialog)
        .with("dialog_is", prompt::label(m, caller.agent))
        .with("recovery", m.recovery.clone())
        .with("cursor_line", m.cursor.0)
        .with("cursor_column", m.cursor.1)
        .with("find_query", if hidden { String::new() } else { m.find.query.clone() })
        .with("find_count", if hidden { 0 } else { m.find.matches.len() as i32 })
        .with("replacement", if hidden { String::new() } else { m.find.replacement.clone() })
}

fn tabs(m: &Model, caller: &Caller) -> serde_json::Value {
    let tabs: Vec<serde_json::Value> = m
        .tabs
        .iter()
        .map(|t| {
            json!({
                "name": shown_title(m, t, caller),
                "path": shown_path(m, t, caller),
                "modified": m.modified(t),
                "opened_by": t.opened_by.label(),
                "opened_by_you": caller.opener.owns(&t.opened_by),
            })
        })
        .collect();
    json!(tabs)
}
