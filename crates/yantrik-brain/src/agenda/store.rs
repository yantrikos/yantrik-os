//! The `agenda` table: its migration, upsert by `(source, source_id)`, state changes, snooze,
//! dismiss and the anti-nag counters. This is the only file that writes anything.

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::salience::UrgencyFn;
use super::{Kind, NextStep, State};

/// Who closed an entry. An entry the agenda closed is never reopened by a gather pass; one its
/// source closed is reopened only if the source reopens it.
pub(crate) const CLOSED_BY_AGENDA: &str = "agenda";
pub(crate) const CLOSED_BY_SOURCE: &str = "source";

/// Idempotent: safe to call on every open, and after a restart. Also brings an older `agenda`
/// table (made before `is_open`, `closed_by` and `source_hold` existed) up to this shape.
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
            is_open         INTEGER NOT NULL DEFAULT 1,  -- 0 once done or dropped
            closed_by       TEXT,                        -- 'agenda' or 'source' while closed
            source_hold     REAL NOT NULL DEFAULT 0,     -- the hold the source last asked for
            UNIQUE(source, source_id)
        );",
    )?;
    let has_is_open: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('agenda') WHERE name = 'is_open'",
        [],
        |r| r.get::<_, i64>(0).map(|n| n > 0),
    )?;
    if !has_is_open {
        conn.execute_batch(
            "ALTER TABLE agenda ADD COLUMN is_open INTEGER NOT NULL DEFAULT 1;
             ALTER TABLE agenda ADD COLUMN closed_by TEXT;
             ALTER TABLE agenda ADD COLUMN source_hold REAL NOT NULL DEFAULT 0;
             UPDATE agenda SET is_open = 0, closed_by = 'source' WHERE state IN ('done','dropped');",
        )?;
    }
    // The plain-words reason a sense gave the entry (never copied text), added after the table first shipped.
    let has_reason: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('agenda') WHERE name = 'reason'",
        [],
        |r| r.get::<_, i64>(0).map(|n| n > 0),
    )?;
    if !has_reason {
        conn.execute_batch("ALTER TABLE agenda ADD COLUMN reason TEXT;")?;
    }
    // Partial: it holds only the active agenda, so reading it costs the active agenda however
    // much closed history the table has.
    conn.execute_batch(
        "DROP INDEX IF EXISTS idx_agenda_state;
         CREATE INDEX IF NOT EXISTS idx_agenda_open ON agenda(source, source_id) WHERE is_open = 1;",
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
    /// When the source first had it, if the mirror knows. Used only when the row is first
    /// inserted, so an old promise does not start its age at the first rollout.
    pub created_at: Option<f64>,
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

struct Existing {
    state: String,
    snoozed_until: f64,
    source_hold: f64,
    is_open: bool,
    closed_by: Option<String>,
}

/// Insert, or update in place by `(source, source_id)`.
///
/// The mirror owns only *open versus closed*. While the source stays active, a state the agenda
/// set (prepared, waiting, snoozed) is kept; the mirror's own state applies when the entry is
/// first inserted or reopened. An entry the agenda closed is never reopened here; one the source
/// closed is reopened only because the source now says it is active again. The anti-nag
/// counters and `created_at` survive. `last_moved_at` changes only when the state does. A hold
/// the source set follows the source (it can be lifted early); a snooze the person set here is
/// never shortened by it.
pub fn upsert(conn: &Connection, e: &NewEntry, now: f64) -> rusqlite::Result<()> {
    let urgency = serde_json::to_string(&e.urgency)
        .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?;
    // Cached statements: a pass runs this once per row, and re-parsing the SQL each time
    // would dominate the cost of a pass over thousands of rows.
    let existing: Option<Existing> = conn
        .prepare_cached(
            "SELECT state, snoozed_until, source_hold, is_open, closed_by FROM agenda \
             WHERE source = ?1 AND source_id = ?2",
        )?
        .query_row(params![e.source, e.source_id], |r| {
            Ok(Existing {
                state: r.get(0)?,
                snoozed_until: r.get(1)?,
                source_hold: r.get(2)?,
                is_open: r.get::<_, i64>(3)? != 0,
                closed_by: r.get(4)?,
            })
        })
        .optional()?;
    let blocked = |s: &str| s == State::WaitingPerson.as_str() || s == State::WaitingModel.as_str();
    let Some(x) = existing else {
        conn.prepare_cached(
            "INSERT INTO agenda (source, source_id, kind, title, title_public, state, \
             next_step, urgency_fn, due_at, confidence, created_at, last_moved_at, \
             blocked_since, updated_at, snoozed_until, source_hold) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?12, ?14, ?14)",
        )?
        .execute(params![
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
            e.created_at.map_or(now, |c| c.min(now)),
            now,
            if blocked(e.state.as_str()) { Some(now) } else { None },
            e.hold_until,
        ])?;
        return Ok(());
    };
    if !x.is_open && x.closed_by.as_deref() != Some(CLOSED_BY_SOURCE) {
        return Ok(()); // the agenda closed it: only the agenda reopens it
    }
    let reopen = !x.is_open;
    let (mut snooze, mut hold) = (x.snoozed_until, x.source_hold);
    if reopen {
        (snooze, hold) = (e.hold_until, e.hold_until);
    } else if e.hold_until != x.source_hold {
        // Lift or move the source's own hold; leave a longer one the person set.
        snooze =
            if x.snoozed_until == x.source_hold { e.hold_until } else { x.snoozed_until.max(e.hold_until) };
        hold = e.hold_until;
    }
    let new_state: &str = if reopen {
        e.state.as_str()
    } else if x.state == State::Snoozed.as_str() {
        if snooze > now { State::Snoozed.as_str() } else { e.state.as_str() }
    } else if e.state == State::Snoozed {
        State::Snoozed.as_str()
    } else {
        x.state.as_str()
    };
    let moved = reopen || x.state != new_state;
    conn.prepare_cached(
        "UPDATE agenda SET kind = ?3, title = ?4, title_public = ?5, next_step = ?6, \
         urgency_fn = ?7, due_at = ?8, confidence = ?9, state = ?10, is_open = 1, \
         closed_by = NULL, snoozed_until = ?14, source_hold = ?15, \
         last_moved_at = CASE WHEN ?11 THEN ?12 ELSE last_moved_at END, \
         blocked_since = CASE WHEN ?11 THEN ?13 ELSE blocked_since END, \
         updated_at = ?12 \
         WHERE source = ?1 AND source_id = ?2 AND (?11 OR kind != ?3 OR title != ?4 \
         OR title_public != ?5 OR next_step != ?6 OR urgency_fn != ?7 \
         OR due_at IS NOT ?8 OR confidence != ?9 OR snoozed_until != ?14 OR source_hold != ?15)",
    )?
    .execute(params![
        e.source,
        e.source_id,
        e.kind.as_str(),
        e.title,
        e.title_public,
        e.next_step.as_str(),
        urgency,
        e.due_at,
        e.confidence,
        new_state,
        moved,
        now,
        if blocked(new_state) { Some(now) } else { None },
        snooze,
        hold,
    ])?;
    Ok(())
}

/// Who is asking for a state change.
#[derive(Clone, Copy)]
pub(crate) enum By {
    Agenda,
    Source,
}

/// Moves an entry to `state` and stamps `last_moved_at`; closing it records who did. A no-op
/// (and `false`) when it is already there or does not exist.
pub(crate) fn move_to(conn: &Connection, id: i64, state: State, by: By, now: f64) -> rusqlite::Result<bool> {
    let blocked = matches!(state, State::WaitingPerson | State::WaitingModel);
    let closed_by = match (state.is_closed(), by) {
        (false, _) => None,
        (true, By::Agenda) => Some(CLOSED_BY_AGENDA),
        (true, By::Source) => Some(CLOSED_BY_SOURCE),
    };
    let n = conn.execute(
        "UPDATE agenda SET state = ?2, last_moved_at = ?3, updated_at = ?3, \
         blocked_since = ?4, is_open = ?5, closed_by = ?6 WHERE id = ?1 AND state != ?2",
        params![id, state.as_str(), now, if blocked { Some(now) } else { None }, !state.is_closed(), closed_by],
    )?;
    Ok(n > 0)
}

/// The agenda's own state change (an executive step, the person).
pub fn set_state(conn: &Connection, id: i64, state: State, now: f64) -> rusqlite::Result<bool> {
    move_to(conn, id, state, By::Agenda, now)
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
         updated_at = ?3, blocked_since = NULL WHERE id = ?1 AND is_open = 1",
        params![id, until, now],
    )?;
    Ok(n > 0)
}

/// The person waved it away: it counts against the entry (so `DecayIfIgnored` bites) and it
/// leaves the ranking for `hold_secs`.
pub fn dismiss(conn: &Connection, id: i64, hold_secs: f64, now: f64) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "UPDATE agenda SET dismiss_count = dismiss_count + 1, snoozed_until = ?2, \
         updated_at = ?3 WHERE id = ?1 AND is_open = 1",
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

/// Sets the plain-words reason of an entry (what the calendar and notes senses say about it).
pub fn set_reason(conn: &Connection, source: &str, source_id: &str, reason: &str) -> rusqlite::Result<()> {
    conn.prepare_cached("UPDATE agenda SET reason = ?3 WHERE source = ?1 AND source_id = ?2 AND reason IS NOT ?3")?
        .execute(params![source, source_id, reason])?;
    Ok(())
}

/// The reason set for an entry, if any.
pub fn reason_of(conn: &Connection, id: i64) -> Option<String> {
    conn.query_row("SELECT reason FROM agenda WHERE id = ?1", params![id], |r| r.get::<_, Option<String>>(0))
        .ok()
        .flatten()
}
