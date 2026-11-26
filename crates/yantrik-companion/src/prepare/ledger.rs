//! What preparation jobs have been started and what each made: the daily limit and "once per item"
//! (#1416, slice 5). Titles, counts and paths only; never fetched text.

use rusqlite::{params, Connection};
use serde_json::{json, Value};

/// Preparations the OS starts in one local day.
pub const PER_DAY: usize = 2;
/// The token cap of one job: what it reads (fetched text, counted at 4 bytes a token) and writes.
pub const TOKEN_CAP: usize = 12_000;

const DAY: f64 = 86_400.0;

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS prepare_jobs (
            item_id    TEXT PRIMARY KEY,    -- the agenda item (an event id) it was made for
            title      TEXT NOT NULL,
            day        TEXT NOT NULL,       -- the local day it started, YYYY-MM-DD
            started_at REAL NOT NULL,
            state      TEXT NOT NULL,       -- running | done | capped | stopped
            result     TEXT NOT NULL DEFAULT '{}',
            outcome    TEXT                 -- opened | discarded | dismissed
        );",
    )
}

/// Jobs started on `day`.
pub fn started_on(conn: &Connection, day: &str) -> usize {
    let _ = migrate(conn);
    conn.query_row("SELECT COUNT(*) FROM prepare_jobs WHERE day = ?1", [day], |r| r.get::<_, i64>(0)).map_or(0, |n| n.max(0) as usize)
}

pub fn exists(conn: &Connection, item_id: &str) -> bool {
    let _ = migrate(conn);
    conn.query_row("SELECT 1 FROM prepare_jobs WHERE item_id = ?1", [item_id], |_| Ok(())).is_ok()
}

/// Starts a job. `false` when the item already has one or the day is full: nothing was written.
pub fn begin(conn: &Connection, item_id: &str, title: &str, day: &str, now: f64) -> bool {
    if migrate(conn).is_err() || exists(conn, item_id) || started_on(conn, day) >= PER_DAY {
        return false;
    }
    conn.execute(
        "INSERT INTO prepare_jobs (item_id, title, day, started_at, state) VALUES (?1, ?2, ?3, ?4, 'running')",
        params![item_id, title, day, now],
    )
    .is_ok()
}

/// Takes a running job back (paused before it wrote anything): it does not count against the day.
pub fn withdraw(conn: &Connection, item_id: &str) {
    let _ = conn.execute("DELETE FROM prepare_jobs WHERE item_id = ?1 AND state = 'running'", [item_id]);
}

pub fn finish(conn: &Connection, item_id: &str, state: &str, result: &Value) {
    let _ = conn.execute("UPDATE prepare_jobs SET state = ?2, result = ?3 WHERE item_id = ?1", params![item_id, state, result.to_string()]);
}

pub fn record_outcome(conn: &Connection, item_id: &str, outcome: &str) {
    let _ = conn.execute("UPDATE prepare_jobs SET outcome = ?2 WHERE item_id = ?1 AND outcome IS NULL", params![item_id, outcome]);
}

/// A job that was `running` when the program stopped is closed as stopped, so it is never tried a
/// second time past its cap.
pub fn close_orphans(conn: &Connection) {
    let _ = migrate(conn);
    let _ = conn.execute("UPDATE prepare_jobs SET state = 'stopped' WHERE state = 'running'", []);
}

/// For `describe shell`: the newest jobs, counts and titles.
pub fn describe(conn: &Connection, now: f64) -> Value {
    let Ok(mut stmt) = conn.prepare("SELECT item_id, title, state, result, outcome FROM prepare_jobs WHERE started_at > ?1 ORDER BY started_at DESC LIMIT 5") else {
        return json!([]);
    };
    let rows = stmt.query_map([now - 7.0 * DAY], |r| {
        let result: String = r.get(3)?;
        let result: Value = serde_json::from_str(&result).unwrap_or(Value::Null);
        Ok(json!({
            "item": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)?, "state": r.get::<_, String>(2)?,
            "line": result.get("line").cloned().unwrap_or(Value::Null),
            "sources": result.get("sources").cloned().unwrap_or(Value::Null),
            "refused": result.get("refused").cloned().unwrap_or(Value::Null),
            "outcome": r.get::<_, Option<String>>(4)?.unwrap_or_else(|| "waiting".into()),
        }))
    });
    rows.map(|r| Value::Array(r.flatten().collect())).unwrap_or_else(|_| json!([]))
}

/// A job's result as it was stored, for the moment line.
pub fn result(conn: &Connection, item_id: &str) -> Option<Value> {
    let s: String = conn.query_row("SELECT result FROM prepare_jobs WHERE item_id = ?1", [item_id], |r| r.get(0)).ok()?;
    serde_json::from_str(&s).ok()
}

/// The item a prepared folder belongs to, if the ledger made that folder. The press of Open
/// folder or Discard is only believed for a folder the ledger knows.
pub fn item_of_folder(conn: &Connection, folder: &str) -> Option<String> {
    let mut stmt = conn.prepare("SELECT item_id, result FROM prepare_jobs").ok()?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).ok()?;
    let found = rows.flatten().find(|(_, result)| serde_json::from_str::<Value>(result).ok().and_then(|v| v["folder"].as_str().map(|f| f == folder)).unwrap_or(false)).map(|(id, _)| id);
    found
}
