//! The tick: once a minute, find the events due a brief, make each one once, replace it when the
//! event changes, and keep it gone when the person dismissed it.
//!
//! The brief lives in the anticipation table, so all of this survives a shell restart: a brief
//! already made is not made again, a dismissed one stays dismissed, and one on screen when the
//! shell restarted comes back.

use std::collections::{HashMap, HashSet};

use chrono::{Duration, NaiveDateTime};
use rusqlite::Connection;
use yantrik_ipc_contracts::calendar::{CalendarEvent, EventsParams};

use super::assemble::{gather, id_of, Sources};
use super::model::{Brief, Card, LEAD_SECS};
use super::window::{fingerprint, is_cancelled, select, to_unix, Own};
use crate::anticipation::{AnticipationEngine, PrepOutcome, PreparationTrigger, PreparedItem, PreparedKind};
use crate::calendar::backend::CalendarBackend;

/// How far ahead the calendar is read. A stored brief whose event is not in this window any more
/// was moved far off or deleted.
const LOOK_AHEAD_HOURS: i64 = 24;
/// Within this many seconds of the start, a brief with a section missing is shown as it is.
const LAST_CALL_SECS: i64 = 180;

/// What a tick found.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// Every brief that is live now and says something, soonest first.
    pub cards: Vec<Card>,
    /// The ids of the briefs made by this tick, not yet announced: the toast is for these, once.
    pub fresh: Vec<String>,
}

fn stamp(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn brief_of(item: &PreparedItem) -> Option<Brief> {
    serde_json::from_value(item.data.clone()).ok()
}

/// The live, non-empty briefs in the table at `now`.
pub fn cards(conn: &Connection, now: i64) -> Vec<Card> {
    AnticipationEngine::get_active(conn, &PreparedKind::MeetingBrief, now as f64)
        .into_iter()
        .filter(|item| item.id.starts_with("meeting:"))
        .filter_map(|item| brief_of(&item).map(|brief| Card { id: item.id, brief }))
        .filter(|c| !c.brief.is_empty())
        .collect()
}

/// Forget the briefs of events that moved, were deleted or were called off.
fn forget_stale(conn: &Connection, events: &[CalendarEvent], now: i64) {
    let mut current: HashMap<&str, HashSet<i64>> = HashMap::new();
    for e in events.iter().filter(|e| !e.is_all_day && !is_cancelled(&e.title)) {
        if let Some(start) = yantrik_ipc_contracts::calendar::parse_stamp(&e.start).and_then(to_unix) {
            current.entry(e.id.as_str()).or_default().insert(start);
        }
    }
    for item in AnticipationEngine::get_active(conn, &PreparedKind::MeetingBrief, now as f64) {
        let PreparationTrigger::CalendarEvent { event_id, .. } = &item.trigger else { continue };
        let still_there = brief_of(&item).is_some_and(|b| current.get(event_id.as_str()).is_some_and(|starts| starts.contains(&b.start)));
        if !still_there {
            AnticipationEngine::remove(conn, &item.id);
        }
    }
}

/// Run one tick at local time `now`.
///
/// `enabled` is the person's switch: off means nothing is read at all, not the calendar, not a
/// source, not the table. A calendar that cannot be read changes nothing and the briefs already
/// made are still returned.
pub fn tick(conn: &Connection, calendar: &dyn CalendarBackend, sources: &Sources<'_>, own: &Own, now: NaiveDateTime, enabled: bool) -> Outcome {
    if !enabled {
        return Outcome::default();
    }
    let Some(now_unix) = to_unix(now) else { return Outcome::default() };
    let params = EventsParams { start_date: stamp(now), end_date: stamp(now + Duration::hours(LOOK_AHEAD_HOURS)) };
    match calendar.list(&params) {
        Err(why) => tracing::debug!(error = %why, "meeting brief: the calendar did not answer"),
        Ok(events) => {
            forget_stale(conn, &events, now_unix);
            for meeting in select(&events, now_unix, own) {
                make(conn, &meeting, sources, own, now_unix);
            }
        }
    }

    // What this tick made and nobody has been told about: announced once, then marked.
    let fresh: Vec<String> = AnticipationEngine::get_relevant(conn)
        .into_iter()
        .filter(|i| i.kind == PreparedKind::MeetingBrief && i.id.starts_with("meeting:"))
        .map(|i| i.id)
        .collect();
    for id in &fresh {
        AnticipationEngine::mark_surfaced(conn, id);
    }
    AnticipationEngine::cleanup_expired(conn);
    Outcome { cards: cards(conn, now_unix), fresh }
}

/// Make one meeting's brief, unless it is already made and nothing it depends on changed.
fn make(conn: &Connection, meeting: &super::window::Meeting, sources: &Sources<'_>, own: &Own, now: i64) {
    let id = id_of(meeting);
    let existing = AnticipationEngine::get(conn, &id);
    if let Some(item) = &existing {
        // Dismissed or expired: never again for this event and start. Unchanged: already made.
        if item.outcome.is_some() || brief_of(item).is_some_and(|b| b.fingerprint == fingerprint(meeting)) {
            return;
        }
    }
    let gathered = gather(meeting, sources, own, now);
    // A source that did not answer is not "nothing to say", and a brief missing a section is not
    // final while there is time: try again next tick. In the last minutes what there is is shown.
    if !gathered.complete && (gathered.brief.is_empty() || meeting.start - now > LAST_CALL_SECS) {
        return;
    }
    let was_announced = existing.as_ref().is_some_and(|i| i.surfaced && brief_of(i).is_some_and(|b| !b.is_empty()));
    // An empty brief is recorded as already surfaced: it is kept so the sources are not asked
    // again every minute, and it is never drawn or announced.
    let surfaced = if gathered.brief.is_empty() { true } else { was_announced };
    let brief = gathered.brief;
    AnticipationEngine::store(
        conn,
        &PreparedItem {
            id,
            trigger: PreparationTrigger::CalendarEvent { event_id: meeting.event_id.clone(), title: meeting.title.clone() },
            kind: PreparedKind::MeetingBrief,
            description: format!("Brief for {}", meeting.title),
            data: serde_json::to_value(&brief).unwrap_or_default(),
            relevant_at: (meeting.start - LEAD_SECS).min(now) as f64,
            expires_at: meeting.end as f64,
            surfaced,
            outcome: None,
            created_at: now as f64,
        },
    );
}

/// The person dismissed the card: it never comes back for this event and start, and that is
/// recorded in the table, so it holds across a restart.
pub fn dismiss(conn: &Connection, id: &str) {
    AnticipationEngine::record_outcome(conn, id, PrepOutcome::Dismissed);
    AnticipationEngine::mark_surfaced(conn, id);
}
