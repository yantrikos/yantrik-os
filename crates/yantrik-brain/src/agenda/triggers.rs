//! What may be prepared for: the items "Prepare for me" starts from (#1416, slice 5). Plain code
//! over the agenda table; titles and times only, no note text.
//!
//! Two kinds of trigger, soonest first:
//! - slice 4's candidate, an `Event` within 36 h linked to a `NoteTodo` with open items
//!   (`event_with_open_note`);
//! - any other open item (not a note) due within 48 h.
//!
//! Nothing when Private mode is on or the Brain switch is off.

use rusqlite::{params, Connection};

use super::senses::{CALENDAR, NOTES};
use super::tick::Switches;

const HOUR: f64 = 3600.0;
/// Items due within this are triggers.
pub const DUE_WITHIN_HOURS: f64 = 48.0;

/// Why an item is worth preparing for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// The first anticipate rule: an event soon, and a note about it with things still to do.
    EventWithOpenNote,
    /// Something open is due within 48 h.
    DueSoon,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Trigger {
    /// Stable per item: the source and the source's own id.
    pub id: String,
    pub title: String,
    pub note_title: Option<String>,
    /// The linked note's id (its path in the library), when there is one.
    pub note_id: Option<String>,
    pub due: f64,
    pub why: Why,
}

/// Triggers due within the window, soonest first, at most `limit`.
pub fn triggers(conn: &Connection, now: f64, switches: Switches, limit: usize) -> rusqlite::Result<Vec<Trigger>> {
    if switches.private || !switches.enabled {
        return Ok(Vec::new());
    }
    super::store::migrate(conn)?;
    let mut stmt = conn.prepare(
        "SELECT ev.source, ev.source_id, ev.kind, ev.title, ev.due_at, n.title, n.source_id \
         FROM agenda ev \
         LEFT JOIN agenda n ON n.link_event = ev.source_id AND n.source = ?3 AND n.is_open = 1 AND n.open_items > 0 \
         WHERE ev.is_open = 1 AND ev.snoozed_until <= ?1 AND ev.due_at > ?1 AND ev.due_at <= ?2 \
           AND ev.source != ?3 AND ev.kind != 'note_todo' \
         ORDER BY ev.due_at, ev.source, ev.source_id",
    )?;
    let rows = stmt.query_map(params![now, now + DUE_WITHIN_HOURS * HOUR, NOTES], |r| {
        let source: String = r.get(0)?;
        let id: String = r.get(1)?;
        let kind: String = r.get(2)?;
        let due: f64 = r.get(4)?;
        let note_title: Option<String> = r.get(5)?;
        let linked = source == CALENDAR && kind == "event" && note_title.is_some() && due <= now + 36.0 * HOUR;
        Ok(Trigger {
            id: format!("{source}:{id}"),
            title: r.get(3)?,
            note_title: if linked { note_title } else { None },
            note_id: if linked { r.get(6)? } else { None },
            due,
            why: if linked { Why::EventWithOpenNote } else { Why::DueSoon },
        })
    })?;
    let mut out: Vec<Trigger> = Vec::new();
    for t in rows.flatten() {
        // One trigger per item: an event with two linked notes comes once.
        if out.iter().all(|o| o.id != t.id) {
            out.push(t);
        }
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}
