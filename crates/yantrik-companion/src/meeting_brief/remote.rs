//! What the brief asks of places that can be slow, kept off the companion's worker.
//!
//! The calendar and the notes service are programs reached over a socket, either may be slow or
//! have to be started first, and a home folder may sit on a network mount. The worker that runs
//! the think tick does everything else the companion does, one thing at a time, so it must never
//! wait on any of them. A [`Fetcher`] owns a thread that makes those calls and the file walk, and
//! leaves what it found in a [`Snapshot`]; the worker reads the snapshot (a lock held for a clone,
//! never across a call) and asks for the next refresh with a send that cannot block. A brief
//! therefore lags the data by about one tick, which is nothing against a ten minute window.
//!
//! Only answers are kept. An error is never cached: the next tick asks again, so one failed
//! search cannot hold a brief back for minutes.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Duration as Span;
use yantrik_ipc_contracts::calendar::{CalendarEvent, CreateEventParams, EventsParams, UpdateEventParams, UpsertRemoteEventParams};

use super::files::FileSource;
use super::model::Row;
use super::notes::{NoteHit, NoteSource};
use crate::calendar::backend::CalendarBackend;

/// How long an answer is trusted. Past it the calendar counts as not answering, and the briefs
/// already made are left as they are.
pub const FRESH: Duration = Duration::from_secs(5 * 60);
/// How long an answer is reused before it is asked again.
const REUSE: Duration = Duration::from_secs(60);
/// How far ahead the calendar is read.
const LOOK_AHEAD_HOURS: i64 = 24;

/// What the fetcher last found.
#[derive(Clone, Default)]
pub struct Snapshot {
    events: Option<(Vec<CalendarEvent>, Instant)>,
    notes: HashMap<String, (Vec<NoteHit>, Instant)>,
    files: HashMap<String, (Vec<Row>, Instant)>,
}

fn files_key(keywords: &[String]) -> String {
    keywords.join("\u{1f}")
}

impl Snapshot {
    /// The events as last read, while they are fresh.
    pub fn events(&self) -> Option<&[CalendarEvent]> {
        self.events.as_ref().filter(|(_, at)| at.elapsed() < FRESH).map(|(events, _)| events.as_slice())
    }

    /// Whether a fresh answer to this notes search is held.
    pub fn has_notes(&self, query: &str) -> bool {
        self.notes.get(query).is_some_and(|(_, at)| at.elapsed() < FRESH)
    }

    /// Whether a fresh answer to this file search is held.
    pub fn has_files(&self, keywords: &[String]) -> bool {
        self.files.get(&files_key(keywords)).is_some_and(|(_, at)| at.elapsed() < FRESH)
    }
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

/// What a gather asked for that the snapshot did not have: the next refresh fetches it.
#[derive(Default)]
pub struct Missing {
    pub notes: std::cell::RefCell<Vec<String>>,
    pub files: std::cell::RefCell<Vec<Vec<String>>>,
}

impl Missing {
    /// Remember that this notes search is wanted.
    pub fn want_notes(&self, query: &str) {
        let mut missing = self.notes.borrow_mut();
        if !missing.iter().any(|m| m == query) {
            missing.push(query.to_string());
        }
    }

    /// Remember that this file search is wanted.
    pub fn want_files(&self, keywords: &[String]) {
        let mut missing = self.files.borrow_mut();
        if !missing.iter().any(|m| m == keywords) {
            missing.push(keywords.to_vec());
        }
    }
}

/// Notes as the last snapshot has them. A search it does not have is an error now (so the brief
/// is "incomplete" and retried) and is remembered in `missing` for the next refresh.
pub struct CachedNotes<'a> {
    pub snapshot: &'a Snapshot,
    pub missing: &'a Missing,
}

impl NoteSource for CachedNotes<'_> {
    fn search(&self, query: &str) -> Result<Vec<NoteHit>, String> {
        match self.snapshot.notes.get(query) {
            Some((hits, at)) if at.elapsed() < FRESH => Ok(hits.clone()),
            _ => {
                self.missing.want_notes(query);
                Err("notes not fetched yet".into())
            }
        }
    }
}

/// File names as the last snapshot has them, on the same terms as [`CachedNotes`].
pub struct CachedFiles<'a> {
    pub snapshot: &'a Snapshot,
    pub missing: &'a Missing,
}

impl FileSource for CachedFiles<'_> {
    fn find(&self, keywords: &[String]) -> Result<Vec<Row>, String> {
        match self.snapshot.files.get(&files_key(keywords)) {
            Some((rows, at)) if at.elapsed() < FRESH => Ok(rows.clone()),
            _ => {
                self.missing.want_files(keywords);
                Err("files not searched yet".into())
            }
        }
    }
}

/// What one refresh is asked to find besides the calendar.
#[derive(Default)]
struct Request {
    notes: Vec<String>,
    files: Vec<Vec<String>>,
}

/// The services and folders the thread asks.
struct Backends {
    calendar: Arc<dyn CalendarBackend>,
    notes: Arc<dyn NoteSource + Send + Sync>,
    files: Arc<dyn FileSource + Send + Sync>,
}

/// The thread that asks the calendar, the notes service and the folders, and the snapshot it fills.
pub struct Fetcher {
    ask: SyncSender<Request>,
    shared: Arc<Mutex<Snapshot>>,
    /// Said once that the thread is gone, so a log is not filled every tick.
    told: AtomicBool,
}

/// Answer one request into the snapshot. Nothing is held across a call.
fn refresh(shared: &Mutex<Snapshot>, backends: &Backends, request: Request) {
    let now = chrono::Local::now().naive_local();
    let params = EventsParams {
        start_date: now.format("%Y-%m-%dT%H:%M:%S").to_string(),
        end_date: (now + Span::hours(LOOK_AHEAD_HOURS)).format("%Y-%m-%dT%H:%M:%S").to_string(),
    };
    let events = backends.calendar.list(&params);
    let (reuse_notes, reuse_files) = {
        let snap = shared.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = |at: &Instant| at.elapsed() < REUSE;
        (
            request.notes.iter().filter(|q| snap.notes.get(*q).is_some_and(|(_, at)| fresh(at))).cloned().collect::<Vec<_>>(),
            request.files.iter().filter(|k| snap.files.get(&files_key(k)).is_some_and(|(_, at)| fresh(at))).cloned().collect::<Vec<_>>(),
        )
    };
    let notes: Vec<(String, Result<Vec<NoteHit>, String>)> =
        request.notes.iter().filter(|q| !reuse_notes.contains(q)).map(|q| (q.clone(), backends.notes.search(q))).collect();
    let files: Vec<(String, Result<Vec<Row>, String>)> =
        request.files.iter().filter(|k| !reuse_files.contains(k)).map(|k| (files_key(k), backends.files.find(k))).collect();

    let mut snap = shared.lock().unwrap_or_else(|e| e.into_inner());
    match events {
        Ok(events) => snap.events = Some((events, Instant::now())),
        Err(why) => tracing::debug!(error = %why, "meeting brief: the calendar did not answer"),
    }
    for (query, answer) in notes {
        match answer {
            Ok(hits) => {
                snap.notes.insert(query, (hits, Instant::now()));
            }
            Err(why) => tracing::debug!(error = %why, "meeting brief: a notes search did not answer"),
        }
    }
    for (key, answer) in files {
        match answer {
            Ok(rows) => {
                snap.files.insert(key, (rows, Instant::now()));
            }
            Err(why) => tracing::debug!(error = %why, "meeting brief: a file search did not answer"),
        }
    }
    snap.notes.retain(|_, (_, at)| at.elapsed() < FRESH);
    snap.files.retain(|_, (_, at)| at.elapsed() < FRESH);
}

impl Fetcher {
    pub fn spawn(
        calendar: Arc<dyn CalendarBackend>,
        notes: Arc<dyn NoteSource + Send + Sync>,
        files: Arc<dyn FileSource + Send + Sync>,
    ) -> Self {
        let (ask, requests) = sync_channel::<Request>(1);
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let held = shared.clone();
        let backends = Backends { calendar, notes, files };
        let spawned = std::thread::Builder::new().name("yos-meeting-brief-fetch".into()).spawn(move || {
            // Blocks until the worker asks: no timer, no polling.
            for request in requests {
                // A fault in one service's client ends that refresh, not the thread: the next
                // tick asks again.
                if catch_unwind(AssertUnwindSafe(|| refresh(&held, &backends, request))).is_err() {
                    tracing::warn!("meeting brief: a service client failed while it was asked; the next refresh tries again");
                }
            }
        });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "meeting brief: could not start its fetch thread");
        }
        Self { ask, shared, told: AtomicBool::new(false) }
    }

    /// What was found so far. Never waits on a service.
    pub fn snapshot(&self) -> Snapshot {
        self.shared.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Ask for a refresh: the calendar once, and these searches. Never waits; a request made
    /// while one is still being answered is dropped, and the next tick asks again.
    pub fn request(&self, notes: Vec<String>, files: Vec<Vec<String>>) {
        if let Err(TrySendError::Disconnected(_)) = self.ask.try_send(Request { notes, files }) {
            if !self.told.swap(true, Ordering::Relaxed) {
                tracing::warn!("meeting brief: its fetch thread is gone; no new briefs can be made until the shell restarts");
            }
        }
    }
}
