//! Which message a command means: a row the person pressed, the message open in the reading
//! pane, or one a caller named.

use crate::command::{Pick, Target};
use crate::model::{Model, Row, OPEN_MESSAGE};
use crate::state;

fn of_row(row: &Row, shown: Option<usize>) -> Target {
    Target {
        id: row.id.clone(),
        folder: row.folder.clone(),
        subject: row.item.subject.clone(),
        was_read: row.item.is_read,
        flagged: row.item.is_flagged,
        shown,
    }
}

/// The message `pick` names, or why there is none.
///
/// A row id is the row's place in the listing, never its place among the rows a triage tab
/// keeps, so Unread and Flagged act on the row that was pressed. The reading pane's buttons act
/// on the message that is open, wherever it is in the list, or when it is no longer in it.
pub fn target(m: &Model, pick: &Pick) -> Result<Target, String> {
    match pick {
        Pick::Key(OPEN_MESSAGE) => open_message(m),
        Pick::Key(key) => usize::try_from(*key)
            .ok()
            .and_then(|at| m.rows.get(at))
            .map(|row| of_row(row, None))
            .ok_or_else(|| format!("there is no message at row {}", i64::from(*key) + 1)),
        Pick::Named(which) => {
            let rows = m.message_rows();
            // Accepts a number as well as a string: `which=1` arrives as the text "1".
            let at = if !which.trim().is_empty() {
                state::resolve_which(&rows, which)?
            } else {
                let open = m.reading().ok_or_else(|| "no message is open, and none was named".to_string())?;
                state::resolve_which(&rows, &open.detail.subject)?
            };
            let shown = m.shown();
            let (_, row) = shown.get(at).ok_or_else(|| format!("there is no message at row {}", at + 1))?;
            Ok(of_row(row, Some(at)))
        }
    }
}

/// The message in the reading pane: its row when it is listed, its own flags when it is not.
fn open_message(m: &Model) -> Result<Target, String> {
    let open = m.open.as_ref().ok_or_else(|| "no message is open, and none was named".to_string())?;
    if let Some(row) = m.rows.iter().find(|r| r.id == open.id && r.folder == open.folder) {
        return Ok(of_row(row, None));
    }
    Ok(Target {
        id: open.id.clone(),
        folder: open.folder.clone(),
        subject: open.detail.subject.clone(),
        was_read: open.detail.is_read,
        flagged: open.detail.is_flagged,
        shown: None,
    })
}

/// The subject at `row` among the rows on screen, as the list shows it now.
pub fn subject_at(m: &Model, row: usize) -> String {
    m.shown().get(row).map(|(_, r)| r.item.subject.clone()).unwrap_or_default()
}
