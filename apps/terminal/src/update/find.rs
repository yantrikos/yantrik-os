//! Find in the session's history. The scrollback is bounded (`HISTORY_LINES`), and the search is
//! a scan of it in memory, so it runs here and not on a worker.

use yantrik_app::effects::Effects;

use super::frame;
use crate::model::{Model, PART_FIND};
use crate::Terminal;

type Fx = Effects<Terminal>;

fn matching(history: &[String], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    history
        .iter()
        .enumerate()
        .filter_map(|(i, line)| line.to_lowercase().contains(&query).then_some(i))
        .collect()
}

/// The query changed. An empty one puts the view back at the live screen.
pub fn search(m: &mut Model, fx: &mut Fx, query: &str) {
    m.query = query.to_string();
    m.matches.clear();
    m.match_index = 0;
    if let Some(session) = m.session().cloned() {
        if query.is_empty() {
            session.set_scrollback(0);
        } else {
            m.matches = matching(&session.history(), query);
        }
    }
    jump(m, fx);
}

/// Scroll to the current match and say where it sits on the screen.
pub fn jump(m: &mut Model, fx: &mut Fx) {
    m.match_row = -1;
    if let (Some(&line), Some(session)) = (m.matches.get(m.match_index), m.session()) {
        let history = session.history();
        let rows = session.geometry().1 .0 as usize;
        let available = history.len().saturating_sub(rows);
        let offset = available.saturating_sub(line);
        session.set_scrollback(offset);
        m.match_row = line.saturating_sub(available - offset).min(rows - 1) as i32;
    }
    fx.changed(PART_FIND);
    frame::catch_up(m, fx);
}

/// The next or previous match, wrapping at the ends.
pub fn step(m: &mut Model, fx: &mut Fx, forward: bool) {
    if m.matches.is_empty() {
        return;
    }
    let n = m.matches.len();
    m.match_index = (m.match_index + n + if forward { 1 } else { n - 1 }) % n;
    jump(m, fx);
}

/// A frame with a query open: the matches are found again in the history as it now is.
pub fn refresh(m: &mut Model, fx: &mut Fx) {
    if m.query.is_empty() {
        return;
    }
    let Some(session) = m.session() else {
        return;
    };
    let history = session.history();
    let (scrollback, size) = session.geometry();
    m.matches = matching(&history, &m.query);
    m.match_index = m.match_index.min(m.matches.len().saturating_sub(1));
    let top = history.len().saturating_sub(size.0 as usize + scrollback);
    m.match_row = m
        .matches
        .get(m.match_index)
        .filter(|&&line| line >= top && line < top + size.0 as usize)
        .map(|&line| (line - top) as i32)
        .unwrap_or(-1);
    fx.changed(PART_FIND);
}
