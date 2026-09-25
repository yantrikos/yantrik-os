//! Move the events the companion kept to itself into the machine's calendar, once.
//!
//! Until today the mind's `calendar_create_event` wrote a row into the SQLite `calendar_events`
//! table with `source = 'local'`. The Calendar app reads `calendar-service`, so those appointments
//! existed on a machine that would never show them. Picking one owner leaves them behind unless
//! something carries them across, and they are appointments: losing them is worse than having
//! kept them in the wrong place.
//!
//! Run once at companion start, and safe to run again because the rows that have been moved are
//! marked. Three properties, in the order they matter:
//!
//! * **Never twice.** `calendar_migrated_events` records the service id each local row was stored
//!   under. A second start skips what is already marked, so an event does not appear twice on the
//!   calendar because the shell was restarted.
//! * **Never lost.** The SQLite rows are not deleted. A marker is the record that they moved; the
//!   row staying put means a migration that went somewhere unexpected can still be read back.
//!   Nothing lists events from that table any more, so the rows sit there inert.
//! * **Safe when the calendar is down.** A service that cannot be started fails the whole pass
//!   without marking anything, and the next start tries again.

use crate::calendar::backend::CalendarBackend;
use crate::calendar::stamps;
use yantrik_ipc_contracts::calendar::CreateEventParams;

/// What one pass did. Reported into the log at start; the fields are there to be read, not
/// summed into a single "ok".
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Migration {
    /// Local rows found that had not been moved before.
    pub pending: usize,
    /// Rows stored in the calendar service by this pass.
    pub moved: usize,
    /// Rows this pass could not move, each with the reason it was given.
    pub failed: Vec<String>,
}

impl Migration {
    pub fn nothing_to_do(&self) -> bool {
        self.pending == 0
    }
}

/// The marker table. Separate from `calendar_events` so a row's own columns are left exactly as
/// they were found — the migration reads history, it does not edit it.
pub fn ensure_marker_table(conn: &rusqlite::Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS calendar_migrated_events (
            local_id   TEXT PRIMARY KEY,
            service_id TEXT NOT NULL,
            moved_at   REAL NOT NULL
        );",
    )
    .ok();
}

/// One local row waiting to be carried across.
struct LocalRow {
    id: String,
    summary: String,
    description: Option<String>,
    location: Option<String>,
    start: String,
    end: String,
    is_all_day: bool,
}

/// The local rows that have not been moved yet.
///
/// `source = 'local'` is how `create_local_event` marked them, and the `local_` id prefix is how
/// it marked them before the column existed. Both are checked because an install old enough to
/// have the second is exactly the one with events worth carrying.
fn pending_rows(conn: &rusqlite::Connection) -> Result<Vec<LocalRow>, String> {
    let sql = "SELECT e.id, e.summary, e.description, e.location, e.start, e.end, e.is_all_day
               FROM calendar_events e
               LEFT JOIN calendar_migrated_events m ON m.local_id = e.id
               WHERE m.local_id IS NULL
                 AND (e.source = 'local' OR substr(e.id, 1, 6) = 'local_')
               ORDER BY e.start ASC";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(LocalRow {
                id: row.get(0)?,
                summary: row.get(1)?,
                description: row.get(2)?,
                location: row.get(3)?,
                start: row.get(4)?,
                end: row.get(5)?,
                is_all_day: row.get::<_, i32>(6)? != 0,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

/// Carry every unmoved local event into the calendar service.
pub fn run(conn: &rusqlite::Connection, backend: &dyn CalendarBackend) -> Migration {
    ensure_marker_table(conn);

    let rows = match pending_rows(conn) {
        Ok(rows) => rows,
        // No table at all is the common case on a machine that never used the old tools.
        Err(e) => {
            return Migration { pending: 0, moved: 0, failed: vec![format!("could not read the companion's old calendar rows: {e}")] };
        }
    };

    let mut report = Migration { pending: rows.len(), ..Default::default() };
    if rows.is_empty() {
        // Nothing to move, so nothing asks the calendar service to start. A machine that never
        // used the old tools pays nothing for this at every boot.
        return report;
    }

    for row in rows {
        // A row written before the store had rules can hold a time the store will refuse. Say
        // which event and why rather than letting the service's message arrive without a name.
        let (start, end) = match bounds(&row) {
            Some(pair) => pair,
            None => {
                report.failed.push(format!(
                    "{}: {:?} does not hold a time the calendar can keep ({} to {})",
                    row.id, row.summary, row.start, row.end
                ));
                continue;
            }
        };

        let params = CreateEventParams {
            title: row.summary.clone(),
            start,
            end,
            description: row.description.clone().unwrap_or_default(),
            location: row.location.clone().filter(|s| !s.is_empty()),
            color: String::new(),
            is_all_day: row.is_all_day,
            attendees: Vec::new(),
            // Migrated out of the companion's old private cache: whoever made these is not
            // on record, and a migration is not a surface caller anybody verified (#201).
            creator: None,
        };

        match backend.create(&params) {
            Ok(stored) => {
                mark(conn, &row.id, &stored.id);
                report.moved += 1;
            }
            Err(e) => report.failed.push(format!("{}: {}", row.id, e)),
        }
    }

    report
}

fn bounds(row: &LocalRow) -> Option<(String, String)> {
    if row.is_all_day {
        // The old rows kept Google's exclusive end for all-day events, because that is what the
        // API handed them.
        if let Some(pair) = stamps::all_day_bounds(&row.start, &row.end) {
            return Some(pair);
        }
    }
    Some((stamps::to_store_stamp(&row.start)?, stamps::to_store_stamp(&row.end)?))
}

fn mark(conn: &rusqlite::Connection, local_id: &str, service_id: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    conn.execute(
        "INSERT OR REPLACE INTO calendar_migrated_events (local_id, service_id, moved_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![local_id, service_id, now],
    )
    .ok();
}
