//! The tick: once a minute, find the events due a brief, make each one once, replace it when the
//! event changes, and keep it gone when the person dismissed it.
//!
//! The brief lives in the anticipation table, so all of this survives a shell restart: a brief
//! already made is not made again, a dismissed one stays dismissed, and one on screen when the
//! shell restarted comes back. Every brief ends with exactly one outcome (opened, dismissed or
//! ignored), and once it is over only that record is kept, never what the card said.

use chrono::{Duration, NaiveDateTime};
use rusqlite::Connection;
use yantrik_ipc_contracts::calendar::{CalendarEvent, EventsParams};

use super::assemble::{gather, id_of, Sources};
use super::model::{Brief, Card, Outcomes, Press, LEAD_SECS};
use super::window::{fingerprint, meeting_of, select, to_unix, Meeting, Own};
use crate::anticipation::{AnticipationEngine, PrepOutcome, PreparationTrigger, PreparedItem, PreparedKind};
use crate::calendar::backend::CalendarBackend;

/// How far ahead the calendar is read.
const LOOK_AHEAD_HOURS: i64 = 24;
/// Within this many seconds of the start, a brief with a section missing is shown as it is.
const LAST_CALL_SECS: i64 = 180;
/// How long the calendar may leave an event out before its brief is ended. A calendar that
/// answers an empty list while its files are rewritten must not end, or re-toast, a brief.
const GONE_AFTER_SECS: i64 = 90;
/// How far back the outcomes `describe` counts reach.
const OUTCOME_DAYS: f64 = 30.0;

/// What a tick found.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    /// Every brief that is live now and says something, soonest first.
    pub cards: Vec<Card>,
    /// The ids of the briefs made by this tick, not yet announced: the notice is for these, once.
    pub fresh: Vec<String>,
    /// How the briefs of the last 30 days ended.
    pub outcomes: Outcomes,
}

fn stamp(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn brief_of(item: &PreparedItem) -> Option<Brief> {
    serde_json::from_value(item.data.clone()).ok()
}

fn is_brief(item: &PreparedItem) -> bool {
    item.id.starts_with("meeting:")
}

/// The live briefs in the table at `now`, with what they hold.
fn live(conn: &Connection, now: i64) -> Vec<PreparedItem> {
    AnticipationEngine::get_active(conn, &PreparedKind::MeetingBrief, now as f64).into_iter().filter(is_brief).collect()
}

/// The live, non-empty briefs in the table at `now`.
pub fn cards(conn: &Connection, now: i64) -> Vec<Card> {
    live(conn, now)
        .into_iter()
        .filter_map(|item| brief_of(&item).map(|brief| Card { id: item.id, brief }))
        .filter(|c| !c.brief.is_empty())
        .collect()
}

fn save(conn: &Connection, item: &PreparedItem, brief: &Brief) {
    let mut item = item.clone();
    item.data = serde_json::to_value(brief).unwrap_or_default();
    AnticipationEngine::store(conn, &item);
}

/// End the briefs of events that moved, were called off or are gone, from what the calendar says.
///
/// An event the calendar lists with another start, or as called off, is over at once. One it does
/// not list at all is only maybe gone: it is marked, and ended once it has stayed out for
/// [`GONE_AFTER_SECS`]; if it comes back first, the mark is cleared and nothing else happened.
fn settle(conn: &Connection, events: &[CalendarEvent], own: &Own, now: i64) {
    for item in live(conn, now) {
        let (Some(mut brief), PreparationTrigger::CalendarEvent { event_id, .. }) = (brief_of(&item), item.trigger.clone()) else { continue };
        let listed: Vec<&CalendarEvent> = events.iter().filter(|e| e.id == event_id).collect();
        if listed.is_empty() {
            match brief.missing_since {
                None => {
                    brief.missing_since = Some(now);
                    save(conn, &item, &brief);
                }
                Some(since) if now - since >= GONE_AFTER_SECS => end(conn, &item, now as f64),
                Some(_) => {}
            }
            continue;
        }
        let still = listed.iter().filter_map(|e| meeting_of(e, own)).any(|m| m.start == brief.start);
        if !still {
            end(conn, &item, now as f64);
        } else if brief.missing_since.take().is_some() {
            save(conn, &item, &brief);
        }
    }
}

/// Run one tick at local time `now`.
///
/// `enabled` is the person's switch: off gathers nothing, not from the calendar and not from a
/// source, and takes back what is up ([`off`]). A calendar that cannot be read changes nothing
/// and the briefs already made are still returned.
pub fn tick(conn: &Connection, calendar: &dyn CalendarBackend, sources: &Sources<'_>, own: &Own, now: NaiveDateTime, enabled: bool) -> Outcome {
    let Some(now_unix) = to_unix(now) else { return Outcome::default() };
    if !enabled {
        return off(conn, now_unix);
    }
    let now_ts = now_unix as f64;
    let params = EventsParams { start_date: stamp(now), end_date: stamp(now + Duration::hours(LOOK_AHEAD_HOURS)) };
    match calendar.list(&params) {
        Err(why) => tracing::debug!(error = %why, "meeting brief: the calendar did not answer"),
        Ok(events) => {
            settle(conn, &events, own, now_unix);
            for meeting in due_and_live(conn, &events, own, now_unix) {
                make(conn, &meeting, sources, own, now_unix);
            }
        }
    }

    // What this tick made and nobody has been told about: announced once, then marked.
    let fresh: Vec<String> = AnticipationEngine::get_relevant(conn, now_ts)
        .into_iter()
        .filter(|i| i.kind == PreparedKind::MeetingBrief && is_brief(i))
        .map(|i| i.id)
        .collect();
    for id in &fresh {
        AnticipationEngine::mark_surfaced(conn, id);
    }
    finish(conn, now_ts);
    AnticipationEngine::cleanup_expired(conn, now_ts);
    Outcome { cards: cards(conn, now_unix), fresh, outcomes: outcomes(conn, now_ts) }
}

/// The meetings to look at: those due a brief, and those that already have one that is on, even
/// after they have started, so a change to the end or the place reaches the card.
fn due_and_live(conn: &Connection, events: &[CalendarEvent], own: &Own, now: i64) -> Vec<Meeting> {
    let mut meetings = select(events, now, own);
    for item in live(conn, now) {
        if meetings.iter().any(|m| id_of(m) == item.id) {
            continue;
        }
        if let Some(m) = events.iter().filter_map(|e| meeting_of(e, own)).find(|m| id_of(m) == item.id) {
            meetings.push(m);
        }
    }
    meetings
}

/// Make one meeting's brief, unless it is already made and nothing it says changed.
fn make(conn: &Connection, meeting: &Meeting, sources: &Sources<'_>, own: &Own, now: i64) {
    let id = id_of(meeting);
    let existing = AnticipationEngine::get(conn, &id);
    if let Some(item) = &existing {
        // Over (dismissed, ignored, gone, ended): never again for this event and start.
        let over = item.expires_at <= now as f64 || item.outcome.as_ref().is_some_and(|o| *o != PrepOutcome::Useful);
        // Unchanged and final: already made.
        let final_ = brief_of(item).is_some_and(|b| b.fingerprint == fingerprint(meeting) && b.complete);
        if over || final_ {
            return;
        }
    }
    let gathered = gather(meeting, sources, own, now);
    // A source that did not answer is not "nothing to say", and a brief missing a section is not
    // final while there is time: try again next tick. In the last minutes what there is is shown,
    // and filled in later if the source answers.
    if !gathered.complete && (gathered.brief.is_empty() || meeting.start - now > LAST_CALL_SECS) {
        return;
    }
    let was_announced = existing.as_ref().is_some_and(|i| i.surfaced && brief_of(i).is_some_and(|b| !b.is_empty()));
    // An empty brief is recorded as already surfaced: it is kept so the sources are not asked
    // again every minute, and it is never drawn or announced.
    let surfaced = gathered.brief.is_empty() || was_announced;
    AnticipationEngine::store(
        conn,
        &PreparedItem {
            id,
            trigger: PreparationTrigger::CalendarEvent { event_id: meeting.event_id.clone(), title: meeting.title.clone() },
            kind: PreparedKind::MeetingBrief,
            description: format!("Brief for {}", meeting.title),
            data: serde_json::to_value(&gathered.brief).unwrap_or_default(),
            relevant_at: existing.as_ref().map_or((meeting.start - LEAD_SECS).min(now) as f64, |i| i.relevant_at),
            expires_at: meeting.end as f64,
            surfaced,
            outcome: existing.as_ref().and_then(|i| i.outcome.clone()),
            created_at: existing.as_ref().map_or(now as f64, |i| i.created_at),
        },
    );
}

/// The switch is off, or its state is not known: nothing is gathered and no card is drawn, and
/// what the person has not kept is taken back. Every brief that is up ends now, and the briefs
/// whose time is up are emptied, as they would have been with the switch on. Only the table is
/// read and written; no source is asked, and what a card said does not wait in it until the
/// person turns briefs back on. The outcomes are the table's, so `describe` still counts them.
pub fn off(conn: &Connection, now: i64) -> Outcome {
    let now_ts = now as f64;
    for item in live(conn, now) {
        end(conn, &item, now_ts);
    }
    finish(conn, now_ts);
    AnticipationEngine::cleanup_expired(conn, now_ts);
    Outcome { cards: Vec::new(), fresh: Vec::new(), outcomes: outcomes(conn, now_ts) }
}

/// End one brief at `now`, whatever the reason, and empty what it held. Shown and never pressed
/// is "ignored"; one nobody saw ends as "expired" and is not counted. One the person pressed
/// already keeps that outcome.
fn end(conn: &Connection, item: &PreparedItem, now: f64) {
    let said = brief_of(item).is_some_and(|b| !b.is_empty());
    let outcome = if said && item.surfaced { PrepOutcome::Ignored } else { PrepOutcome::Expired };
    AnticipationEngine::retire(conn, &item.id, outcome, now);
}

/// End the briefs whose time is up.
fn finish(conn: &Connection, now: f64) {
    for item in AnticipationEngine::ended(conn, &PreparedKind::MeetingBrief, now).into_iter().filter(is_brief) {
        end(conn, &item, now);
    }
}

/// How the briefs of the last 30 days ended.
fn outcomes(conn: &Connection, now: f64) -> Outcomes {
    let counts = AnticipationEngine::outcome_counts(conn, &PreparedKind::MeetingBrief, now - OUTCOME_DAYS * 86_400.0);
    let of = |o: PrepOutcome| counts.get(o.as_str()).copied().unwrap_or(0);
    Outcomes { opened: of(PrepOutcome::Useful), dismissed: of(PrepOutcome::Dismissed), ignored: of(PrepOutcome::Ignored) }
}

/// The person pressed something on a card at `now` (unix seconds). Opened is the card's outcome
/// unless it already has one, and the card stays. Dismissed ends the card for good, whatever the
/// outcome is: it never comes back for this event and start, and that holds across a restart.
pub fn press(conn: &Connection, id: &str, press: Press, now: f64) {
    match press {
        Press::Opened => {
            if AnticipationEngine::get(conn, id).is_some_and(|i| i.outcome.is_none()) {
                AnticipationEngine::record_outcome(conn, id, PrepOutcome::Useful);
            }
        }
        Press::Dismissed => AnticipationEngine::retire(conn, id, PrepOutcome::Dismissed, now),
    }
}
