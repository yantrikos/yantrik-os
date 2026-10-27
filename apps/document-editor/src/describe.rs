//! What a mind reads first: one line that says which document is in front, how long it is and
//! whether it is saved; and the state behind it, with the same keys and values yDoc has always
//! published.
//!
//! A glance, not a transcript (docs/app-control.md): the first 4000 characters of the document,
//! and the state says how much of it that was, so a caller reading an excerpt knows it is one.
//!
//! A document the person opened from a place an agent may not read is not read back to one: not
//! its text, its outline, its size, its name, or a match count over it. A count would answer
//! "does it contain X?" one query at a time.

use yantrik_app::control::View;

use crate::agent_rule::HIDDEN;
use crate::caller::Caller;
use crate::markdown;
use crate::model::Model;
use crate::prompt;

/// How much of the document `describe` carries.
pub const EXCERPT_CHARS: usize = 4000;

pub fn view(m: &Model) -> View {
    let caller = Caller::now();
    let hidden = caller.agent && m.doc.hidden;
    let text = if hidden { "" } else { m.text() };
    let words = markdown::word_count(text);
    let total = markdown::char_count(text);
    let excerpt: String = text.chars().take(EXCERPT_CHARS).collect();
    let (title, path) = if hidden {
        (HIDDEN.to_string(), serde_json::json!(HIDDEN))
    } else {
        (m.title(), serde_json::json!(m.path()))
    };
    let where_it_lives = match (hidden, m.path()) {
        (true, _) => HIDDEN.to_string(),
        (false, Some(p)) => p.display().to_string(),
        (false, None) => "no file yet".into(),
    };
    let modified = m.modified();
    let mut summary = format!(
        "yDoc \u{2014} \u{201c}{title}\u{201d}, {words} words, {} \u{b7} {where_it_lives}",
        if modified { "unsaved" } else { "saved" },
    );
    if m.busy.is_some() {
        summary.push_str(" \u{b7} working");
    }
    View::new(summary)
        .with("format", "markdown")
        .with("path", path)
        .with("title", title)
        .with("content_hidden", hidden.then_some("the document is a file an agent is not shown"))
        .with("dirty", modified)
        .with("recovered_draft", m.doc.recovered)
        .with("words", words)
        .with("chars", total)
        .with(
            "outline",
            markdown::outline(text)
                .into_iter()
                .map(|h| serde_json::json!({"title": h.title, "level": h.level, "line": h.line, "offset": h.offset}))
                .collect::<Vec<_>>(),
        )
        .with("content", excerpt.clone())
        .with("content_chars_shown", markdown::char_count(&excerpt))
        .with("content_chars_total", total)
        .with("find_query", if hidden { String::new() } else { m.find.query.clone() })
        .with("find_matches", if hidden { 0 } else { m.find.matches.len() })
        // A prompt is open in the window, so a person is part-way through something and an
        // action that changed the document under them would be a surprise.
        .with("prompt_open", prompt::dialog(m) != prompt::NONE)
        .with("notice", m.notice.read(caller.agent).to_string())
}
