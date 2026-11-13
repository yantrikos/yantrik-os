//! Google Calendar, pulled into the machine's own calendar rather than beside it.
//!
//! The old shape was local-first into SQLite: a Google event was cached in the companion's
//! database and a local event was written there too, so the Calendar app showed neither. The sync
//! now writes through `calendar-service`, keyed on the event's id at Google, which does two
//! things at once — a synced event appears in the Calendar app like any other, and syncing the
//! same week twice edits the same file instead of storing a second copy of everything.
//!
//! With no Google account connected this is a quiet no-op, as it was before. The calendar works
//! without Google and always did; that is not the part that was broken.
//!
//! ## What this does not do
//!
//! **Events deleted at Google are not removed here.** The old code deleted a date range from its
//! cache before re-inserting, which made a remote deletion disappear and took every local event
//! in the range with it — that range delete is one of the reasons an appointment made through the
//! mind could vanish. Doing it properly needs Google's own sync tokens, so that "not in this
//! answer" can be told apart from "not in this page of this answer", and that is not built. A
//! meeting cancelled in Google's web UI stays on this machine's calendar until it is deleted here.
//!
//! **One page, one window.** `list_events` asks for at most `max_results` and does not follow
//! `nextPageToken`, which is how it has always been. A very busy week can come back short.

use crate::calendar::backend::CalendarBackend;
use crate::calendar::stamps;
use crate::calendar::GoogleAccess;
use chrono::{Local, NaiveDate, TimeZone};
use yantrik_ipc_contracts::calendar::UpsertRemoteEventParams;

/// How stale the last pull may be before another one is worth a network round trip.
///
/// Thirty minutes, which is what the SQLite cache used. A calendar tool is called in the middle
/// of a conversation and the person is waiting; asking Google on every call spends half a second
/// of that to learn nothing on all but the first.
const FRESH_FOR_SECS: f64 = 1800.0;

/// What one pull did.
#[derive(Debug, Default, Clone)]
pub struct Sync {
    /// False when there is no OAuth2 account, or the last pull is still fresh.
    pub attempted: bool,
    /// Events written through to the calendar service.
    pub stored: usize,
    /// Why it did not finish, in Google's words or the service's. Absent when it did.
    pub note: Option<String>,
}

/// Sync bookkeeping, and only that.
///
/// This is the one thing left in the companion's database that is about the calendar, and it
/// holds no events: one row, the instant of the last successful pull. The events themselves live
/// in `calendar-service` and nothing here lists them.
fn ensure_state_table(conn: &rusqlite::Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS calendar_sync_state (
            key TEXT PRIMARY KEY,
            at  REAL NOT NULL
        );",
    )
    .ok();
}

fn last_pull(conn: &rusqlite::Connection) -> f64 {
    conn.query_row(
        "SELECT at FROM calendar_sync_state WHERE key = 'google_pulled_at'",
        [],
        |row| row.get(0),
    )
    .unwrap_or(0.0)
}

fn record_pull(conn: &rusqlite::Connection, at: f64) {
    conn.execute(
        "INSERT OR REPLACE INTO calendar_sync_state (key, at) VALUES ('google_pulled_at', ?1)",
        rusqlite::params![at],
    )
    .ok();
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Pull `from..=to` from Google into the calendar service.
///
/// `force` skips the freshness gate, for a search that has to see what is there now rather than
/// what was there half an hour ago.
pub fn pull(
    google: &dyn GoogleAccess,
    conn: &rusqlite::Connection,
    backend: &dyn CalendarBackend,
    from: NaiveDate,
    to: NaiveDate,
    force: bool,
) -> Sync {
    if !google.connected() {
        return Sync::default();
    }

    ensure_state_table(conn);
    let now = now_secs();
    if !force && now - last_pull(conn) < FRESH_FOR_SECS {
        return Sync::default();
    }

    let token = match google.token() {
        Ok(t) => t,
        Err(e) => return Sync { attempted: true, stored: 0, note: Some(e) },
    };

    let (Some(time_min), Some(time_max)) = (rfc3339_start(from), rfc3339_end(to)) else {
        return Sync {
            attempted: true,
            stored: 0,
            note: Some(format!("could not express {from}..{to} as a time range")),
        };
    };

    let remote = match super::list_events(&token, None, Some(&time_min), Some(&time_max), 100, None)
    {
        Ok(events) => events,
        Err(e) => return Sync { attempted: true, stored: 0, note: Some(e) },
    };

    let mut stored = 0usize;
    let mut refused: Vec<String> = Vec::new();
    for event in &remote {
        let Some(params) = as_upsert(event) else {
            refused.push(format!("{}: no time this calendar can keep", event.summary));
            continue;
        };
        match backend.upsert_remote(&params) {
            Ok(_) => stored += 1,
            Err(e) => refused.push(format!("{}: {}", event.summary, e)),
        }
    }

    // Only a pull that reached the service counts as done. Recording the instant after a failure
    // would hold the gate shut for half an hour over an outage that lasted a second.
    if refused.is_empty() {
        record_pull(conn, now);
    }

    Sync {
        attempted: true,
        stored,
        note: if refused.is_empty() {
            None
        } else {
            Some(format!("{} of {} not stored: {}", refused.len(), remote.len(), refused.join("; ")))
        },
    }
}

/// One Google event as the service's upsert reads it, or `None` if its times are not times.
fn as_upsert(event: &super::CalEvent) -> Option<UpsertRemoteEventParams> {
    let (start, end) = if event.is_all_day {
        stamps::all_day_bounds(&event.start, &event.end)?
    } else {
        (stamps::to_store_stamp(&event.start)?, stamps::to_store_stamp(&event.end)?)
    };
    Some(UpsertRemoteEventParams {
        remote_id: event.id.clone(),
        title: if event.summary.trim().is_empty() { "(no title)".to_string() } else { event.summary.clone() },
        start,
        end,
        description: event.description.clone().unwrap_or_default(),
        location: event.location.clone().filter(|s| !s.is_empty()),
        is_all_day: event.is_all_day,
        attendees: Vec::new(),
    })
}

// `earliest` and `latest` rather than `single`: a local time on the night a zone shifts can be
// ambiguous or not exist at all, and a window that widens by an hour once a year is better than a
// sync that refuses to run that night.
fn rfc3339_start(day: NaiveDate) -> Option<String> {
    let naive = day.and_hms_opt(0, 0, 0)?;
    Some(Local.from_local_datetime(&naive).earliest()?.to_rfc3339())
}

fn rfc3339_end(day: NaiveDate) -> Option<String> {
    let naive = day.and_hms_opt(23, 59, 59)?;
    Some(Local.from_local_datetime(&naive).latest()?.to_rfc3339())
}
