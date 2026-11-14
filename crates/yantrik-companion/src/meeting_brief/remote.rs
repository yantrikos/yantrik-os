//! What the brief asks of places that can be slow, kept off the companion's worker.
//!
//! The calendar and the notes service are programs reached over a socket, either may be slow or
//! have to be started first, and a home folder may sit on a network mount. The worker that runs
//! the think tick does everything else the companion does, one thing at a time, so it must never
//! wait on any of them. A [`Fetcher`] owns the threads that make those calls and the file walk, and
//! leaves what it found in a [`Snapshot`]; the worker reads the snapshot (a lock held for a clone,
//! never across a call) and asks for the next refresh with a send that cannot block. The folders
//! are walked on a thread of their own, so a mount that hangs costs the files of a brief and
//! never the calendar. A brief therefore lags the data by about one tick, which is nothing
//! against a ten minute window.
//!
//! Only answers are kept. An error is never cached: the next tick asks again, so one failed
//! search cannot hold a brief back for minutes.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
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

/// What one refresh is asked to find besides the calendar: notes searches.
#[derive(Default)]
struct Request {
    notes: Vec<String>,
}

/// The services the first thread asks. The folders have a thread of their own ([`walk`]): a
/// mount that never answers costs the files section of a brief and nothing else.
struct Backends {
    calendar: Arc<dyn CalendarBackend>,
    notes: Arc<dyn NoteSource + Send + Sync>,
}

/// The threads that ask the calendar, the notes service and the folders, and the snapshot they fill.
pub struct Fetcher {
    /// The calendar and the notes searches.
    ask: SyncSender<Request>,
    /// The file searches.
    walk: SyncSender<Vec<Vec<String>>>,
    shared: Arc<Mutex<Snapshot>>,
    /// Said once that a thread is gone, so a log is not filled every tick.
    told: AtomicBool,
}

/// Answer one request into the snapshot. Nothing is held across a call, and the calendar's answer
/// is kept before anything slower is asked: a notes search that takes a minute must not age the
/// calendar past [`FRESH`] and stop every brief.
fn refresh(shared: &Mutex<Snapshot>, backends: &Backends, request: Request) {
    let now = chrono::Local::now().naive_local();
    let params = EventsParams {
        start_date: now.format("%Y-%m-%dT%H:%M:%S").to_string(),
        end_date: (now + Span::hours(LOOK_AHEAD_HOURS)).format("%Y-%m-%dT%H:%M:%S").to_string(),
    };
    match backends.calendar.list(&params) {
        Ok(events) => shared.lock().unwrap_or_else(|e| e.into_inner()).events = Some((events, Instant::now())),
        Err(why) => tracing::debug!(error = %why, "meeting brief: the calendar did not answer"),
    }
    let reuse_notes: Vec<String> = {
        let snap = shared.lock().unwrap_or_else(|e| e.into_inner());
        request.notes.iter().filter(|q| snap.notes.get(*q).is_some_and(|(_, at)| at.elapsed() < REUSE)).cloned().collect()
    };
    let notes: Vec<(String, Result<Vec<NoteHit>, String>)> =
        request.notes.iter().filter(|q| !reuse_notes.contains(q)).map(|q| (q.clone(), backends.notes.search(q))).collect();

    let mut snap = shared.lock().unwrap_or_else(|e| e.into_inner());
    for (query, answer) in notes {
        match answer {
            Ok(hits) => {
                snap.notes.insert(query, (hits, Instant::now()));
            }
            Err(why) => tracing::debug!(error = %why, "meeting brief: a notes search did not answer"),
        }
    }
    snap.notes.retain(|_, (_, at)| at.elapsed() < FRESH);
}

/// Answer the file searches asked for, on the thread that walks the folders.
fn walk(shared: &Mutex<Snapshot>, files: &dyn FileSource, wanted: Vec<Vec<String>>) {
    let reuse: Vec<Vec<String>> = {
        let snap = shared.lock().unwrap_or_else(|e| e.into_inner());
        wanted.iter().filter(|k| snap.files.get(&files_key(k)).is_some_and(|(_, at)| at.elapsed() < REUSE)).cloned().collect()
    };
    let found: Vec<(String, Result<Vec<Row>, String>)> =
        wanted.iter().filter(|k| !reuse.contains(k)).map(|k| (files_key(k), files.find(k))).collect();
    let mut snap = shared.lock().unwrap_or_else(|e| e.into_inner());
    for (key, answer) in found {
        match answer {
            Ok(rows) => {
                snap.files.insert(key, (rows, Instant::now()));
            }
            Err(why) => tracing::debug!(error = %why, "meeting brief: a file search did not answer"),
        }
    }
    snap.files.retain(|_, (_, at)| at.elapsed() < FRESH);
}

/// Run `work` on a thread of its own for each request, until the fetcher is dropped. A fault in
/// one service's client ends that request, not the thread: the next tick asks again.
fn serve<T: Send + 'static>(name: &str, requests: Receiver<T>, work: impl Fn(T) + Send + 'static) {
    let spawned = std::thread::Builder::new().name(format!("yos-meeting-brief-{name}")).spawn(move || {
        // Blocks until the worker asks: no timer, no polling.
        for request in requests {
            if catch_unwind(AssertUnwindSafe(|| work(request))).is_err() {
                tracing::warn!("meeting brief: a service client failed while it was asked; the next refresh tries again");
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "meeting brief: could not start its {name} thread");
    }
}

impl Fetcher {
    pub fn spawn(
        calendar: Arc<dyn CalendarBackend>,
        notes: Arc<dyn NoteSource + Send + Sync>,
        files: Arc<dyn FileSource + Send + Sync>,
    ) -> Self {
        let (ask, requests) = sync_channel::<Request>(1);
        let (to_walk, walks) = sync_channel::<Vec<Vec<String>>>(1);
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let (asked, walked) = (shared.clone(), shared.clone());
        let backends = Backends { calendar, notes };
        serve("fetch", requests, move |request| refresh(&asked, &backends, request));
        serve("walk", walks, move |wanted| walk(&walked, files.as_ref(), wanted));
        Self { ask, walk: to_walk, shared, told: AtomicBool::new(false) }
    }

    /// What was found so far. Never waits on a service.
    pub fn snapshot(&self) -> Snapshot {
        self.shared.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Ask for a refresh: the calendar once, and these searches. Never waits; a request made
    /// while one is still being answered is dropped, and the next tick asks again. The file
    /// searches go to the thread that walks the folders, and wait there for the walk before them.
    pub fn request(&self, notes: Vec<String>, files: Vec<Vec<String>>) {
        let gone = matches!(self.ask.try_send(Request { notes }), Err(TrySendError::Disconnected(_)))
            || (!files.is_empty() && matches!(self.walk.try_send(files), Err(TrySendError::Disconnected(_))));
        if gone && !self.told.swap(true, Ordering::Relaxed) {
            tracing::warn!("meeting brief: a fetch thread is gone; no new briefs can be made until the shell restarts");
        }
    }
}
