//! What a mind reads first: one line that says which deck is in front, which slide and whether it
//! is saved; and the state behind it, with the same keys and values yPresent has always
//! published.
//!
//! A deck the person opened from a place an agent may not read is not read back to one: not its
//! slides, its titles, its name, or a match count over it.

use yantrik_app::control::View;

use crate::agent_rule::HIDDEN;
use crate::caller::Caller;
use crate::deck::theme_spec;
use crate::model::{Model, OUTLINE_CAP};
use crate::update::clock;

/// Why a hidden deck's content is not given, in place of the path the old sentence named.
const HIDDEN_WHY: &str = "the deck is a file an agent is not shown";

pub fn view(m: &Model) -> View {
    let caller = Caller::now();
    let deck = m.deck();
    let modified = m.modified();
    if caller.agent && m.doc.hidden {
        return View::new(format!("yPresent \u{2014} {HIDDEN}"))
            .with("file", HIDDEN)
            .with("content_hidden", HIDDEN_WHY)
            .with("dirty", modified)
            .with("presenting", m.presenting)
            .with("notice", m.notice.read(true).to_string());
    }
    let at = deck.current();
    let titles = deck.outline();
    View::new(format!(
        "yPresent \u{2014} {} \u{b7} slide {} of {}{}",
        deck.title(),
        at + 1,
        deck.len(),
        if modified { " \u{b7} unsaved" } else { "" }
    ))
    .with("file", m.path().map(|p| p.display().to_string()))
    .with("content_hidden", serde_json::Value::Null)
    .with("dirty", modified)
    .with("recovered", m.doc.recovered)
    .with("slides", deck.len())
    .with("index", at)
    .with("deck_title", deck.title().to_string())
    .with(
        "current",
        serde_json::json!({
            "title": deck.current_slide().title,
            "body": deck.current_slide().body,
            "notes": deck.current_slide().notes,
            "layout": deck.current_slide().layout,
        }),
    )
    // Capped, and the cap is said out loud: an outline that silently stops at a hundred titles
    // is a caller believing it has seen the whole deck.
    .with("outline", titles.iter().take(OUTLINE_CAP).cloned().collect::<Vec<_>>())
    .with("outline_shown", titles.len().min(OUTLINE_CAP))
    .with("outline_total", titles.len())
    .with("presenting", m.presenting)
    .with("theme", theme_spec(deck.theme()).name)
    .with("timer", clock::text(m.clock.seconds))
    .with("timer_running", m.clock.running)
    .with("search_matches", m.hits.len())
    // Where Save would put a deck that has never been given a home, so a caller can find the file
    // afterwards without having to know the rule. A name already taken is counted past (`-2`), so
    // the save's own answer names the file it wrote.
    .with("saves_to", m.saves_to().display().to_string())
    .with("recovery", m.draft_path().display().to_string())
    .with("notice", m.notice.read(caller.agent).to_string())
}
