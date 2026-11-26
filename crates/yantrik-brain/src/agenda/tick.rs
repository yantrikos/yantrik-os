//! One pass of the agenda for the shell's Think arm: the switches, then `gather`, then the two
//! senses. The caller owns the clock and the sources; this owns the order and the refusals.

use std::path::Path;

use rusqlite::Connection;

use super::gather::{gather, GatherReport, SourceReport};
use super::notes_dir::read_notes;
use super::senses::{gather_senses, CalEvent, NOTE_RECENT_DAYS};

const DAY: f64 = 86_400.0;

/// Whether the agenda may look at anything this tick.
#[derive(Debug, Clone, Copy)]
pub struct Switches {
    /// Private mode: nothing is gathered while it is on.
    pub private: bool,
    /// The person's `brain` switch (Settings → Initiative). On unless turned off.
    pub enabled: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TickReport {
    pub mirrors: GatherReport,
    pub senses: Vec<SourceReport>,
}

/// Runs one pass. `Ok(None)` when a switch says no: then nothing was read (`events` is not even
/// called, the notes directory is not opened) and nothing was written. `events` answers `None`
/// when the calendar could not be read; the notes are skipped likewise when `notes` is `None`
/// or unreadable, and in both cases the agenda keeps what it had.
pub fn tick(
    conn: &Connection,
    now: f64,
    switches: Switches,
    events: impl FnOnce() -> Option<Vec<CalEvent>>,
    notes: Option<&Path>,
) -> rusqlite::Result<Option<TickReport>> {
    if switches.private || !switches.enabled {
        return Ok(None);
    }
    let mirrors = gather(conn, now)?;
    let events = events();
    let notes = notes.and_then(|d| read_notes(d, now, NOTE_RECENT_DAYS * DAY));
    let senses = gather_senses(conn, now, events.as_deref(), notes.as_deref())?;
    Ok(Some(TickReport { mirrors, senses }))
}
