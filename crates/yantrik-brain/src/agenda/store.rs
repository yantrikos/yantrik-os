//! The `agenda` table: its migration, upsert by `(source, source_id)`, state changes, snooze,
//! dismiss and the anti-nag counters. This is the only file that writes anything.

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::salience::UrgencyFn;
use super::{Kind, NextStep, State};

/// Idempotent: safe to call on every open, and after a restart.
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS agenda (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            source          TEXT NOT NULL,   -- which store it mirrors, e.g. 'wm_commitments'
            source_id       TEXT NOT NULL,   -- that store's own key for the row
            kind            TEXT NOT NULL,
            title           TEXT NOT NULL,   -- generic unless title_public = 1
            title_public    INTEGER NOT NULL DEFAULT 0,
            state           TEXT NOT NULL,
            next_step       TEXT NOT NULL,
            urgency_fn      TEXT NOT NULL,   -- JSON of UrgencyFn
            due_at          REAL,
            confidence      REAL NOT NULL DEFAULT 0.5,
            created_at      REAL NOT NULL,
            last_moved_at   REAL NOT NULL,
            blocked_since   REAL,
            surface_count   INTEGER NOT NULL DEFAULT 0,
            max_surfaces    INTEGER NOT NULL DEFAULT 3,
            dismiss_count   INTEGER NOT NULL DEFAULT 0,
            snoozed_until   REAL NOT NULL DEFAULT 0,
            updated_at      REAL NOT NULL,
            UNIQUE(source, source_id)
        );
        CREATE INDEX IF NOT EXISTS idx_agenda_state ON agenda(state);",
    )
}

/// What a mirror hands to [`upsert`].
#[derive(Debug, Clone)]
pub struct NewEntry {
    pub source: &'static str,
    pub source_id: String,
    pub kind: Kind,
    pub title: String,
    pub title_public: bool,
    pub state: State,
    pub next_step: NextStep,
    pub urgency: UrgencyFn,
    pub due_at: Option<f64>,
    pub confidence: f64,
    /// The source holds it until then (a snoozed thread). 0 for none.
    pub hold_until: f64,
}

/// One row of the table.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: i64,
    pub source: String,
    pub source_id: String,
    pub kind: Kind,
    pub title: String,
    pub title_public: bool,
    pub state: State,
    pub next_step: NextStep,
    pub urgency: UrgencyFn,
    pub due_at: Option<f64>,
    pub confidence: f64,
    pub created_at: f64,
    pub last_moved_at: f64,
    pub blocked_since: Option<f64>,
    pub surface_count: u32,
    pub max_surfaces: u32,
    pub dismiss_count: u32,
    pub snoozed_until: f64,
    pub updated_at: f64,
}

pub(crate) const COLUMNS: &str = "id, source, source_id, kind, title, title_public, state, \
    next_step, urgency_fn, due_at, confidence, created_at, last_moved_at, blocked_since, \
    surface_count, max_surfaces, dismiss_count, snoozed_until, updated_at";

/// Reads one row. `None` when a column holds something this code does not know (a row from a
/// newer version, a hand edit): the caller skips and counts it.
pub(crate) fn read_entry(row: &Row) -> Option<Entry> {
    let urgency: String = row.get(8).ok()?;
    Some(Entry {
        id: row.get(0).ok()?,
        source: row.get(1).ok()?,
        source_id: row.get(2).ok()?,
        kind: Kind::parse(&row.get::<_, String>(3).ok()?)?,
        title: row.get(4).ok()?,
        title_public: row.get::<_, i64>(5).ok()? != 0,
        state: State::parse(&row.get::<_, String>(6).ok()?)?,
        next_step: NextStep::parse(&row.get::<_, String>(7).ok()?)?,
        urgency: serde_json::from_str(&urgency).ok()?,
        due_at: row.get(9).ok()?,
        confidence: row.get(10).ok()?,
        created_at: row.get(11).ok()?,
        last_moved_at: row.get(12).ok()?,
        blocked_since: row.get(13).ok()?,
        surface_count: row.get::<_, i64>(14).ok()?.max(0) as u32,
        max_surfaces: row.get::<_, i64>(15).ok()?.max(0) as u32,
        dismiss_count: row.get::<_, i64>(16).ok()?.max(0) as u32,
        snoozed_until: row.get(17).ok()?,
        updated_at: row.get(18).ok()?,
    })
}

/// Insert, or update in place by `(source, source_id)`. The anti-nag counters, the snooze and
/// `created_at` survive an update. `last_moved_at` changes only when the state does, so "time
/// since it last moved" means something. An entry the agenda itself snoozed stays snoozed while
/// its source still says it is active.
pub fn upsert(conn: &Connection, e: &NewEntry, now: f64) -> rusqlite::Result<()> {
    let urgency = serde_json::to_string(&e.urgency)
        .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
    // Cached statements: a pass runs this once per row, and re-parsing the SQL each time
    // would dominate the cost of a pass over thousands of rows.
    let existing: Option<(String, f64)> = conn
        .prepare_cached("SELECT state, snoozed_until FROM agenda WHERE source = ?1 AND source_id = ?2")?
        .query_row(params![e.source, e.source_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let blocked = matches!(e.state, State::WaitingPerson | State::WaitingModel);
    match existing {
        None => {
            conn.prepare_cached(
                "INSERT INTO agenda (source, source_id, kind, title, title_public, state, \
                 next_step, urgency_fn, due_at, confidence, created_at, last_moved_at, \
                 blocked_since, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?12, ?11)",
            )?
            .execute(
                params![
                    e.source,
                    e.source_id,
                    e.kind.as_str(),
                    e.title,
                    e.title_public,
                    e.state.as_str(),
                    e.next_step.as_str(),
                    urgency,
                    e.due_at,
                    e.confidence,
                    now,
                    if blocked { Some(now) } else { None },
                ],
            )?;
        }
        Some((old_state, snoozed_until)) => {
            let mut new_state = e.state;
            if old_state == State::Snoozed.as_str() && snoozed_until > now && !e.state.is_closed() {
                new_state = State::Snoozed;
            }
            let moved = old_state != new_state.as_str();
            conn.prepare_cached(
                "UPDATE agenda SET kind = ?3, title = ?4, title_public = ?5, next_step = ?6, \
                 urgency_fn = ?7, due_at = ?8, confidence = ?9, state = ?10, \
                 last_moved_at = CASE WHEN ?11 THEN ?12 ELSE last_moved_at END, \
                 blocked_since = CASE WHEN ?11 THEN ?13 ELSE blocked_since END, \
                 updated_at = ?12 \
                 WHERE source = ?1 AND source_id = ?2 AND (?11 OR kind != ?3 OR title != ?4 \
                 OR title_public != ?5 OR next_step != ?6 OR urgency_fn != ?7 \
                 OR due_at IS NOT ?8 OR confidence != ?9)",
            )?
            .execute(
                params![
                    e.source,
                    e.source_id,
                    e.kind.as_str(),
                    e.title,
                    e.title_public,
                    e.next_step.as_str(),
                    urgency,
                    e.due_at,
                    e.confidence,
                    new_state.as_str(),
                    moved,
                    now,
                    if blocked { Some(now) } else { None },
                ],
            )?;
        }
    }
    if e.hold_until > 0.0 {
        // Only ever raises the hold, so a snooze the person set here is never shortened, and a
        // second pass with the same hold writes nothing.
        conn.execute(
            "UPDATE agenda SET snoozed_until = ?3 \
             WHERE source = ?1 AND source_id = ?2 AND snoozed_until < ?3",
            params![e.source, e.source_id, e.hold_until],
        )?;
    }
    Ok(())
}

/// Moves an entry to `state` and stamps `last_moved_at`. A no-op (and `false`) when it is
/// already there or does not exist.
pub fn set_state(conn: &Connection, id: i64, state: State, now: f64) -> rusqlite::Result<bool> {
    let blocked = matches!(state, State::WaitingPerson | State::WaitingModel);
    let n = conn.execute(
        "UPDATE agenda SET state = ?2, last_moved_at = ?3, updated_at = ?3, \
         blocked_since = ?4 WHERE id = ?1 AND state != ?2",
        params![id, state.as_str(), now, if blocked { Some(now) } else { None }],
    )?;
    Ok(n > 0)
}

pub fn open(conn: &Connection, id: i64, now: f64) -> rusqlite::Result<bool> {
    set_state(conn, id, State::Open, now)
}
pub fn prepared(conn: &Connection, id: i64, now: f64) -> rusqlite::Result<bool> {
    set_state(conn, id, State::Prepared, now)
}
pub fn waiting_person(conn: &Connection, id: i64, now: f64) -> rusqlite::Result<bool> {
    set_state(conn, id, State::WaitingPerson, now)
}
pub fn waiting_model(conn: &Connection, id: i64, now: f64) -> rusqlite::Result<bool> {
    set_state(conn, id, State::WaitingModel, now)
}
pub fn done(conn: &Connection, id: i64, now: f64) -> rusqlite::Result<bool> {
    set_state(conn, id, State::Done, now)
}
pub fn dropped(conn: &Connection, id: i64, now: f64) -> rusqlite::Result<bool> {
    set_state(conn, id, State::Dropped, now)
}

/// Hides an entry from the ranking until `until`.
pub fn snooze(conn: &Connection, id: i64, until: f64, now: f64) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "UPDATE agenda SET snoozed_until = ?2, state = 'snoozed', last_moved_at = ?3, \
         updated_at = ?3, blocked_since = NULL WHERE id = ?1 AND state NOT IN ('done','dropped')",
        params![id, until, now],
    )?;
    Ok(n > 0)
}

/// The person waved it away: it counts against the entry (so `DecayIfIgnored` bites) and it
/// leaves the ranking for `hold_secs`.
pub fn dismiss(conn: &Connection, id: i64, hold_secs: f64, now: f64) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "UPDATE agenda SET dismiss_count = dismiss_count + 1, snoozed_until = ?2, \
         updated_at = ?3 WHERE id = ?1 AND state NOT IN ('done','dropped')",
        params![id, now + hold_secs, now],
    )?;
    Ok(n > 0)
}

/// Records that the entry was shown to the person (the anti-nag counter).
pub fn mark_surfaced(conn: &Connection, id: i64, now: f64) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "UPDATE agenda SET surface_count = surface_count + 1, updated_at = ?2 WHERE id = ?1",
        params![id, now],
    )?;
    Ok(n > 0)
}

/// True while the entry has been shown as often as it may be.
pub fn is_exhausted(e: &Entry) -> bool {
    e.surface_count >= e.max_surfaces
}

pub fn get(conn: &Connection, id: i64) -> rusqlite::Result<Option<Entry>> {
    let sql = format!("SELECT {COLUMNS} FROM agenda WHERE id = ?1");
    conn.query_row(&sql, params![id], |r| Ok(read_entry(r))).optional().map(Option::flatten)
}

pub fn find(conn: &Connection, source: &str, source_id: &str) -> rusqlite::Result<Option<Entry>> {
    let sql = format!("SELECT {COLUMNS} FROM agenda WHERE source = ?1 AND source_id = ?2");
    conn.query_row(&sql, params![source, source_id], |r| Ok(read_entry(r)))
        .optional()
        .map(Option::flatten)
}
