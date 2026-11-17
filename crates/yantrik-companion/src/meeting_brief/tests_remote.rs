//! The worker never waits on the calendar or the notes service: they are asked on the fetcher's
//! thread, and the worker's tick reads what was found.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Duration as Span;
use yantrik_ipc_contracts::calendar::{CalendarEvent, CreateEventParams, EventsParams, UpdateEventParams, UpsertRemoteEventParams};

use super::files::{FileRoots, FileSource};
use super::model::Row;
use super::local::{tick_fresh_with, tick_with};
use super::notes::{NoteHit, NoteSource};
use super::remote::Fetcher;
use super::tests::{attendee, db, event, maya_email_row, now, ME};
use crate::calendar::backend::CalendarBackend;

const SLOW: Duration = Duration::from_secs(1);

/// A calendar that takes a second to answer, and counts the calls.
struct SlowCalendar {
    events: Vec<CalendarEvent>,
    calls: AtomicUsize,
}

impl CalendarBackend for SlowCalendar {
    fn list(&self, _: &EventsParams) -> Result<Vec<CalendarEvent>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(SLOW);
        Ok(self.events.clone())
    }
    fn get(&self, _: &str) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
    fn create(&self, _: &CreateEventParams) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
    fn update(&self, _: &UpdateEventParams) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        Err("not used".into())
    }
    fn upsert_remote(&self, _: &UpsertRemoteEventParams) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
}

/// A notes service that takes a second per search, and counts them.
struct SlowNotes(AtomicUsize);

impl NoteSource for SlowNotes {
    fn search(&self, query: &str) -> Result<Vec<NoteHit>, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(SLOW);
        Ok(if query == "orion" { vec![NoteHit { id: "orion.md".into(), title: "Orion notes".into(), modified_at: "2026-11-01".into() }] } else { Vec::new() })
    }
}

#[test]
fn a_slow_calendar_or_notes_service_never_delays_the_worker() {
    let conn = db();
    let n = now();
    maya_email_row(&conn, n);
    let calendar = Arc::new(SlowCalendar {
        events: vec![event("e1", "Orion launch", n + Span::minutes(10), vec![attendee("Pat", ME), attendee("Maya", "maya@work.test")])],
        calls: AtomicUsize::new(0),
    });
    let notes = Arc::new(SlowNotes(AtomicUsize::new(0)));
    let fetcher = Fetcher::spawn(calendar.clone(), notes.clone(), Arc::new(FileRoots::default()));

    // Every tick returns at once, whatever the services are doing: nothing to wait for.
    let mut longest = Duration::ZERO;
    let mut cards = 0;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(30) && cards == 0 {
        let t = Instant::now();
        let out = tick_with(&conn, "Pat", &fetcher, n, Some(true));
        longest = longest.max(t.elapsed());
        cards = out.cards.len();
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(longest < Duration::from_millis(250), "a tick took {longest:?} while the services each take {SLOW:?}");
    assert_eq!(cards, 1, "once the services had answered, the brief was made");
    assert!(calendar.calls.load(Ordering::SeqCst) >= 1);
    assert!(notes.0.load(Ordering::SeqCst) >= 1, "the notes were asked, on the fetcher's thread");
}

#[test]
fn with_the_switch_off_the_services_are_never_asked() {
    let conn = db();
    let n = now();
    let calendar = Arc::new(SlowCalendar { events: vec![], calls: AtomicUsize::new(0) });
    let notes = Arc::new(SlowNotes(AtomicUsize::new(0)));
    let fetcher = Fetcher::spawn(calendar.clone(), notes.clone(), Arc::new(FileRoots::default()));
    for _ in 0..3 {
        tick_with(&conn, "Pat", &fetcher, n, Some(false));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(calendar.calls.load(Ordering::SeqCst), 0);
    assert_eq!(notes.0.load(Ordering::SeqCst), 0);
}

/// A calendar that answers at once and counts the calls.
struct Counting {
    events: Vec<CalendarEvent>,
    calls: AtomicUsize,
}

impl CalendarBackend for Counting {
    fn list(&self, _: &EventsParams) -> Result<Vec<CalendarEvent>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.events.clone())
    }
    fn get(&self, _: &str) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
    fn create(&self, _: &CreateEventParams) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
    fn update(&self, _: &UpdateEventParams) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        Err("not used".into())
    }
    fn upsert_remote(&self, _: &UpsertRemoteEventParams) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
}

/// Waits, up to two seconds, until `done` holds.
fn until(done: impl Fn() -> bool) {
    let started = Instant::now();
    while !done() && started.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn an_idle_tick_asks_the_calendar_once_and_the_notes_never() {
    let conn = db();
    let n = now();
    // The only event is hours away: nothing is in the window, so no brief is being made.
    let calendar = Arc::new(Counting {
        events: vec![event("e1", "Orion launch", n + Span::hours(3), vec![attendee("Maya Chen", "maya@work.test")])],
        calls: AtomicUsize::new(0),
    });
    let notes = Arc::new(SlowNotes(AtomicUsize::new(0)));
    let fetcher = Fetcher::spawn(calendar.clone(), notes.clone(), Arc::new(FileRoots::default()));
    tick_with(&conn, "Pat", &fetcher, n, Some(true));
    until(|| calendar.calls.load(Ordering::SeqCst) >= 1);
    tick_with(&conn, "Pat", &fetcher, n, Some(true));
    until(|| calendar.calls.load(Ordering::SeqCst) >= 2);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(calendar.calls.load(Ordering::SeqCst), 2, "one calendar call per tick");
    assert_eq!(notes.0.load(Ordering::SeqCst), 0, "no event near the window, so no notes call");
}

/// Notes that fail the first time they are asked, and answer after.
struct FlakyNotes {
    asked: AtomicUsize,
}

impl NoteSource for FlakyNotes {
    fn search(&self, query: &str) -> Result<Vec<NoteHit>, String> {
        if self.asked.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err("the notes service is starting".into());
        }
        Ok(if query == "orion" { vec![NoteHit { id: "orion.md".into(), title: "Orion notes".into(), modified_at: "2026-11-01".into() }] } else { Vec::new() })
    }
}

fn orion(n: chrono::NaiveDateTime, minutes: i64) -> Vec<CalendarEvent> {
    vec![event("e1", "Orion launch", n + Span::minutes(minutes), vec![attendee("Pat", ME), attendee("Maya", "maya@work.test")])]
}

#[test]
fn a_failed_notes_search_is_asked_again_next_tick_and_does_not_hold_the_brief_back() {
    let conn = db();
    let n = now();
    maya_email_row(&conn, n);
    let calendar = Arc::new(Counting { events: orion(n, 10), calls: AtomicUsize::new(0) });
    let notes = Arc::new(FlakyNotes { asked: AtomicUsize::new(0) });
    let fetcher = Fetcher::spawn(calendar, notes.clone(), Arc::new(FileRoots::default()));
    let started = Instant::now();
    let mut card = None;
    while started.elapsed() < Duration::from_secs(10) && card.is_none() {
        let out = tick_with(&conn, "Pat", &fetcher, n, Some(true));
        card = out.cards.into_iter().next();
        std::thread::sleep(Duration::from_millis(50));
    }
    let card = card.expect("the brief was made once the notes answered, not after a failure aged out");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(card.brief.notes.len(), 1, "with its note: the error was not kept");
    assert!(card.brief.complete);
}

#[test]
fn the_searches_of_a_meeting_about_to_be_due_are_asked_before_its_brief_needs_them() {
    let conn = db();
    let n = now();
    // Twelve minutes out: not yet due a brief, but inside the lead plus two minutes.
    let calendar = Arc::new(Counting { events: orion(n, 12), calls: AtomicUsize::new(0) });
    let notes = Arc::new(SlowNotes(AtomicUsize::new(0)));
    let fetcher = Fetcher::spawn(calendar.clone(), notes.clone(), Arc::new(FileRoots::default()));
    tick_with(&conn, "Pat", &fetcher, n, Some(true));
    until(|| calendar.calls.load(Ordering::SeqCst) >= 1);
    tick_with(&conn, "Pat", &fetcher, n, Some(true));
    until(|| notes.0.load(Ordering::SeqCst) >= 1);
    assert!(notes.0.load(Ordering::SeqCst) >= 1, "the notes were asked ahead");
    assert_eq!(conn.query_row("SELECT COUNT(*) FROM prepared_items", [], |r| r.get::<_, i64>(0)).expect("count"), 0, "and no brief was made yet");
}

/// A calendar whose client fails on its first call, as a transport or parsing fault would.
struct PanickingCalendar {
    events: Vec<CalendarEvent>,
    calls: AtomicUsize,
}

impl CalendarBackend for PanickingCalendar {
    fn list(&self, _: &EventsParams) -> Result<Vec<CalendarEvent>, String> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("a fault in the calendar client");
        }
        Ok(self.events.clone())
    }
    fn get(&self, _: &str) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
    fn create(&self, _: &CreateEventParams) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
    fn update(&self, _: &UpdateEventParams) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        Err("not used".into())
    }
    fn upsert_remote(&self, _: &UpsertRemoteEventParams) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
}

#[test]
fn a_client_that_panics_ends_that_refresh_and_not_the_fetcher() {
    let conn = db();
    let n = now();
    maya_email_row(&conn, n);
    let calendar = Arc::new(PanickingCalendar { events: orion(n, 10), calls: AtomicUsize::new(0) });
    let fetcher = Fetcher::spawn(calendar.clone(), Arc::new(SlowNotes(AtomicUsize::new(0))), Arc::new(FileRoots::default()));
    tick_with(&conn, "Pat", &fetcher, n, Some(true));
    until(|| calendar.calls.load(Ordering::SeqCst) >= 1);
    let started = Instant::now();
    let mut cards = 0;
    while started.elapsed() < Duration::from_secs(20) && cards == 0 {
        cards = tick_with(&conn, "Pat", &fetcher, n, Some(true)).cards.len();
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(calendar.calls.load(Ordering::SeqCst) >= 2, "the fetcher asked again after the fault");
    assert_eq!(cards, 1, "and the brief was made: the thread was still there");
}

/// A folder search that does not return until the test lets it, as a hung network mount does.
struct HungFiles {
    hold: Mutex<Receiver<()>>,
    asked: AtomicUsize,
}

impl FileSource for HungFiles {
    fn find(&self, _: &[String]) -> Result<Vec<Row>, String> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let _ = self.hold.lock().map(|hold| hold.recv());
        Ok(Vec::new())
    }
}

#[test]
fn a_folder_that_never_answers_costs_the_files_of_a_brief_and_nothing_else() {
    let conn = db();
    let n = now();
    maya_email_row(&conn, n);
    // Two minutes out: inside the last call, so the brief is shown with what it has.
    let calendar = Arc::new(Counting { events: orion(n, 2), calls: AtomicUsize::new(0) });
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let files = Arc::new(HungFiles { hold: Mutex::new(hold), asked: AtomicUsize::new(0) });
    let fetcher = Fetcher::spawn(calendar.clone(), Arc::new(SlowNotes(AtomicUsize::new(0))), files.clone());
    let started = Instant::now();
    let mut cards = 0;
    while started.elapsed() < Duration::from_secs(20) && cards == 0 {
        cards = tick_with(&conn, "Pat", &fetcher, n, Some(true)).cards.len();
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(cards, 1, "the brief was made without its files");
    assert!(files.asked.load(Ordering::SeqCst) >= 1, "the walk is hung, as the test means it to be");
    // The calendar goes on being read while the walk is stuck, so no brief is stopped by it.
    let calls = calendar.calls.load(Ordering::SeqCst);
    while started.elapsed() < Duration::from_secs(20) && calendar.calls.load(Ordering::SeqCst) < calls + 2 {
        tick_with(&conn, "Pat", &fetcher, n, Some(true));
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(calendar.calls.load(Ordering::SeqCst) >= calls + 2, "the calendar is still asked every tick");
    assert!(fetcher.snapshot().events().is_some(), "and its answer is fresh");
    drop(release);
}

/// A calendar the test edits between ticks.
struct Editable(Mutex<Vec<CalendarEvent>>);

impl CalendarBackend for Editable {
    fn list(&self, _: &EventsParams) -> Result<Vec<CalendarEvent>, String> {
        Ok(self.0.lock().expect("events").clone())
    }
    fn get(&self, _: &str) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
    fn create(&self, _: &CreateEventParams) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
    fn update(&self, _: &UpdateEventParams) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        Err("not used".into())
    }
    fn upsert_remote(&self, _: &UpsertRemoteEventParams) -> Result<CalendarEvent, String> {
        Err("not used".into())
    }
}

#[test]
fn a_change_to_the_calendar_reaches_the_card_on_the_very_next_tick() {
    let conn = db();
    let n = now();
    maya_email_row(&conn, n);
    let calendar = Arc::new(Editable(Mutex::new(orion(n, 10))));
    let fetcher = Fetcher::spawn(calendar.clone(), Arc::new(SlowNotes(AtomicUsize::new(0))), Arc::new(FileRoots::default()));
    let wait = Duration::from_secs(5);
    // Notes are asked on the first tick and answer a moment later (the real ticks are a minute
    // apart; here they are 200 ms), so the brief is made within a few.
    let mut cards = Vec::new();
    for _ in 0..40 {
        cards = tick_fresh_with(&conn, "Pat", &fetcher, n, Some(true), wait).cards;
        if !cards.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(cards.len(), 1, "the brief was made");
    // The event is called off. The very next tick, with no tick between to carry the news, ends the card.
    let mut cancelled = orion(n, 10);
    cancelled[0].title = "Cancelled: Orion launch".into();
    *calendar.0.lock().expect("events") = cancelled;
    let after = tick_fresh_with(&conn, "Pat", &fetcher, n, Some(true), wait);
    assert!(after.cards.is_empty(), "the card outlived its event by a tick");
}

#[test]
fn every_thread_name_fits_what_linux_keeps_and_they_differ() {
    let names: Vec<String> = ["cal", "fetch", "walk"].iter().map(|n| super::remote::thread_name(n)).collect();
    assert!(names.iter().all(|n| n.len() <= 15), "{names:?}");
    assert_ne!(names[0], names[1]);
    assert_ne!(names[1], names[2]);
    assert_ne!(names[0], names[2]);
}

/// A notes service that is stuck in one search until the test lets go: dropping the sender frees
/// it at once, also when an assertion fails first; the timeout only bounds a test that leaks it.
struct StuckNotes(Mutex<Receiver<()>>);

impl NoteSource for StuckNotes {
    fn search(&self, _: &str) -> Result<Vec<NoteHit>, String> {
        let _ = self.0.lock().expect("gate").recv_timeout(Duration::from_secs(5));
        Ok(Vec::new())
    }
}

/// Gate 13, step 15: a retitle to "Cancelled" took two ticks (122 s) when it came while a notes
/// search was in flight, because the calendar read waited behind it. It has a thread of its own.
#[test]
fn a_calendar_change_is_read_while_a_notes_search_is_stuck() {
    let conn = db();
    let n = now();
    maya_email_row(&conn, n);
    let calendar = Arc::new(Editable(Mutex::new(orion(n, 10))));
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let fetcher = Fetcher::spawn(calendar.clone(), Arc::new(StuckNotes(Mutex::new(gate))), Arc::new(FileRoots::default()));
    // The first tick asks for notes; the fetch thread is then stuck in the search.
    tick_fresh_with(&conn, "Pat", &fetcher, n, Some(true), Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(200));
    let mut cancelled = orion(n, 10);
    cancelled[0].title = "Cancelled: Orion launch".into();
    *calendar.0.lock().expect("events") = cancelled;
    let started = Instant::now();
    tick_fresh_with(&conn, "Pat", &fetcher, n, Some(true), Duration::from_secs(5));
    assert!(started.elapsed() < Duration::from_secs(3), "the calendar read waited behind the notes search");
    let events = fetcher.snapshot().events().map(|e| e.to_vec()).unwrap_or_default();
    assert!(events.iter().all(|e| e.title.starts_with("Cancelled")), "the snapshot still holds the old title");
    drop(release);
}
