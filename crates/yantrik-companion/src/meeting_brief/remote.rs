//! The two services the brief asks, kept off the companion's worker.
//!
//! The calendar and the notes service are programs reached over a socket, and either may be slow
//! or have to be started first. The worker that runs the think tick does everything else the
//! companion does, one thing at a time, so it must never wait on them. A [`Fetcher`] owns a
//! thread that makes those calls and leaves what it found in a [`Snapshot`]; the worker reads the
//! snapshot (a lock held for a clone, never across a call) and asks for the next refresh with a
//! send that cannot block. A brief therefore lags the data by about one tick, which is nothing
//! against a ten minute window.

use std::collections::HashMap;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Duration as Span;
use yantrik_ipc_contracts::calendar::{CalendarEvent, CreateEventParams, EventsParams, UpdateEventParams, UpsertRemoteEventParams};

use super::notes::{NoteHit, NoteSource};
use crate::calendar::backend::CalendarBackend;

/// How long an answer is trusted. Past it the calendar counts as not answering, and the briefs
/// already made are left as they are.
pub const FRESH: Duration = Duration::from_secs(5 * 60);
/// How long a notes search is reused before it is asked again.
const NOTES_REUSE: Duration = Duration::from_secs(60);
/// How far ahead the calendar is read.
const LOOK_AHEAD_HOURS: i64 = 24;

/// What the fetcher last found.
#[derive(Clone, Default)]
pub struct Snapshot {
    events: Option<(Vec<CalendarEvent>, Instant)>,
    notes: HashMap<String, (Result<Vec<NoteHit>, String>, Instant)>,
}

/// The calendar as the last snapshot has it. `list` ignores the range it is given: the fetcher
/// read the next 24 hours, which is what the engine asks for.
pub struct SnapshotCalendar<'a>(pub &'a Snapshot);

impl CalendarBackend for SnapshotCalendar<'_> {
    fn list(&self, _: &EventsParams) -> Result<Vec<CalendarEvent>, String> {
        match &self.0.events {
            Some((events, at)) if at.elapsed() < FRESH => Ok(events.clone()),
            Some(_) => Err("the calendar has not answered for a while".into()),
            None => Err("the calendar has not answered yet".into()),
        }
    }
    fn get(&self, _: &str) -> Result<CalendarEvent, String> {
        Err("not available from a snapshot".into())
    }
    fn create(&self, _: &CreateEventParams) -> Result<CalendarEvent, String> {
        Err("not available from a snapshot".into())
    }
    fn update(&self, _: &UpdateEventParams) -> Result<CalendarEvent, String> {
        Err("not available from a snapshot".into())
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        Err("not available from a snapshot".into())
    }
    fn upsert_remote(&self, _: &UpsertRemoteEventParams) -> Result<CalendarEvent, String> {
        Err("not available from a snapshot".into())
    }
}

/// Notes as the last snapshot has them. A search it does not have is an error now (so the brief
/// is "incomplete" and retried) and is remembered in `missing` for the next refresh.
pub struct CachedNotes<'a> {
    pub snapshot: &'a Snapshot,
    pub missing: std::cell::RefCell<Vec<String>>,
}

impl NoteSource for CachedNotes<'_> {
    fn search(&self, query: &str) -> Result<Vec<NoteHit>, String> {
        match self.snapshot.notes.get(query) {
            Some((answer, at)) if at.elapsed() < FRESH => answer.clone(),
            _ => {
                let mut missing = self.missing.borrow_mut();
                if !missing.iter().any(|m| m == query) {
                    missing.push(query.to_string());
                }
                Err("notes not fetched yet".into())
            }
        }
    }
}

struct Request {
    searches: Vec<String>,
}

/// The thread that asks the calendar and the notes service, and the snapshot it fills.
pub struct Fetcher {
    ask: SyncSender<Request>,
    shared: Arc<Mutex<Snapshot>>,
}

impl Fetcher {
    pub fn spawn(calendar: Arc<dyn CalendarBackend>, notes: Arc<dyn NoteSource + Send + Sync>) -> Self {
        let (ask, requests) = sync_channel::<Request>(1);
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let held = shared.clone();
        let spawned = std::thread::Builder::new().name("yos-meeting-brief-fetch".into()).spawn(move || {
            // Blocks until the worker asks: no timer, no polling.
            for request in requests {
                let now = chrono::Local::now().naive_local();
                let params = EventsParams {
                    start_date: now.format("%Y-%m-%dT%H:%M:%S").to_string(),
                    end_date: (now + Span::hours(LOOK_AHEAD_HOURS)).format("%Y-%m-%dT%H:%M:%S").to_string(),
                };
                let events = calendar.list(&params);
                let reuse: Vec<String> = {
                    let snap = held.lock().unwrap_or_else(|e| e.into_inner());
                    request
                        .searches
                        .iter()
                        .filter(|q| snap.notes.get(*q).is_some_and(|(answer, at)| answer.is_ok() && at.elapsed() < NOTES_REUSE))
                        .cloned()
                        .collect()
                };
                let found: Vec<(String, Result<Vec<NoteHit>, String>)> = request
                    .searches
                    .iter()
                    .filter(|q| !reuse.contains(q))
                    .map(|q| (q.clone(), notes.search(q)))
                    .collect();
                let mut snap = held.lock().unwrap_or_else(|e| e.into_inner());
                match events {
                    Ok(events) => snap.events = Some((events, Instant::now())),
                    Err(why) => tracing::debug!(error = %why, "meeting brief: the calendar did not answer"),
                }
                for (query, answer) in found {
                    snap.notes.insert(query, (answer, Instant::now()));
                }
                snap.notes.retain(|_, (_, at)| at.elapsed() < FRESH);
            }
        });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "meeting brief: could not start its fetch thread");
        }
        Self { ask, shared }
    }

    /// What was found so far. Never waits on a service.
    pub fn snapshot(&self) -> Snapshot {
        self.shared.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Ask for a refresh: the calendar once, and these notes searches. Never waits; a request
    /// made while one is still being answered is dropped, and the next tick asks again.
    pub fn request(&self, searches: Vec<String>) {
        let _ = self.ask.try_send(Request { searches });
    }
}
