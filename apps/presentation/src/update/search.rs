//! Searching the deck: find, count, and go to the first match. The count is what the search bar
//! draws.

use super::{say, slides};
use crate::deck::Edits;
use crate::model::{Cue, Model};

/// Find `query` in the deck and go to the first slide that holds it.
///
/// In a window drawn for a Mind nothing matches: the text a match would be in is not shown there,
/// so a count over it would answer "does it say X?" one query at a time.
pub fn run(m: &mut Model, canvas: &Edits, query: &str) {
    m.deck_mut().commit(canvas);
    m.hits = if m.for_mind { Vec::new() } else { m.deck().search(query) };
    m.hit_at = 0;
    m.query = query.to_string();
    m.cues.push(Cue::Query);
    match m.hits.first().copied() {
        Some(at) => {
            slides::go_to(m, canvas, at as i64);
            m.notice.clear();
        }
        None if query.trim().is_empty() => m.notice.clear(),
        None => say(m, format!("No slide holds \u{201c}{}\u{201d}.", query.trim())),
    }
}

/// Step to the next match, wrapping round to the first.
pub fn next(m: &mut Model, canvas: &Edits) {
    if m.hits.is_empty() {
        return say(m, "Nothing to step through; search first.");
    }
    m.hit_at = (m.hit_at + 1) % m.hits.len();
    let at = m.hits[m.hit_at];
    slides::go_to(m, canvas, at as i64);
    m.notice.clear();
}
