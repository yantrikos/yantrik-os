//! The find bar: matches counted over the whole document, a selection only where the window holds
//! the text (#328), and replacing one match or all of them.

use crate::model::{Cue, Model};
use crate::text::search;

/// Counts the find query's matches in the tab in front, and with `select` puts the selection on
/// the first.
pub fn search(m: &mut Model, select: bool) {
    m.find.matches = search::matches(m.active_text(), &m.find.query, m.find.match_case);
    m.find.at = 0;
    if select {
        if let Some(&(a, z)) = m.find.matches.first() {
            select_if_shown(m, a, z);
        }
    }
}

/// What `describe.find_count` and the bar's counter say: the match the selection is on, from 1.
pub fn at_match(m: &Model) -> usize {
    if m.find.matches.is_empty() {
        0
    } else {
        m.find.at + 1
    }
}

/// Moves to the next match, or the one before, wrapping round. Nothing when there are none.
pub fn step(m: &mut Model, forward: bool) {
    let len = m.find.matches.len();
    if len == 0 {
        return;
    }
    m.find.at = (m.find.at + if forward { 1 } else { len - 1 }) % len;
    let (a, z) = m.find.matches[m.find.at];
    select_if_shown(m, a, z);
}

/// A match is selected only when it lies inside the view: match offsets are the document's, and
/// one past the view would point at text the editor field does not hold (#328). The view is a
/// leading slice of the document, so its length is the boundary.
fn select_if_shown(m: &mut Model, a: usize, z: usize) {
    if z <= m.viewport().end {
        m.cues.push(Cue::Select(a, z));
    }
}

/// The tab in front's text with the current match (or with `all`, every match) replaced by
/// `with`, or why not.
pub fn replaced(m: &Model, all: bool, with: &str) -> Result<String, String> {
    let ranges: &[(usize, usize)] = if all {
        &m.find.matches
    } else {
        m.find.matches.get(m.find.at).map(std::slice::from_ref).unwrap_or(&[])
    };
    search::replace(m.active_text(), ranges, with)
}
