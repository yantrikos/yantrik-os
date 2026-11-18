//! Find and replace: the bar's matches in the document, stepping through them, and replacing.

use super::editing::commit;
use crate::markdown;
use crate::model::{Cue, Model};
use crate::YDoc;
use yantrik_app::effects::Effects;

/// Recounts the matches for the find bar's query, and optionally goes to the first one.
pub fn search(m: &mut Model, select: bool) {
    m.find.matches = markdown::matches(m.text(), &m.find.query);
    m.find.at = 0;
    if select {
        if let Some(&(a, z)) = m.find.matches.first() {
            m.cues.push(Cue::Select(a, z));
            m.cues.push(Cue::FocusBody);
        }
    }
}

/// The selection moved to the next match, or the one before, round the document.
pub fn step(m: &mut Model, forward: bool) {
    let len = m.find.matches.len();
    if len == 0 {
        return;
    }
    m.find.at = (m.find.at + if forward { 1 } else { len - 1 }) % len;
    let (a, z) = m.find.matches[m.find.at];
    m.cues.push(Cue::Select(a, z));
    m.cues.push(Cue::FocusBody);
}

/// Replaces the match the selection is on, or every match, with `m.find.replacement`. How many
/// were replaced.
pub fn replace_matches(m: &mut Model, fx: &mut Effects<YDoc>, all: bool) -> Result<usize, String> {
    let ranges: Vec<(usize, usize)> = if all {
        m.find.matches.clone()
    } else {
        m.find.matches.get(m.find.at).copied().into_iter().collect()
    };
    let count = ranges.len();
    if count == 0 {
        return Err("Nothing matched, so nothing was replaced.".into());
    }
    let out = markdown::replace(m.text(), &ranges, &m.find.replacement)?;
    // `commit` recounts the matches against the new text.
    commit(m, fx, out, None)?;
    Ok(count)
}
