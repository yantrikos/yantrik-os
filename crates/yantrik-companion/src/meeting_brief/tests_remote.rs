//! The worker never waits on the calendar or the notes service: they are asked on the fetcher's
//! thread, and the worker's tick reads what was found.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Duration as Span;
use yantrik_ipc_contracts::calendar::{CalendarEvent, CreateEventParams, EventsParams, UpdateEventParams, UpsertRemoteEventParams};

use super::local::tick_with;
use super::notes::{NoteHit, NoteSource};
use super::remote::Fetcher;
use super::tests::{attendee, db, event, maya_email_row, now, ME};
use crate::calendar::backend::CalendarBackend;

const SLOW: Duration = Duration::from_secs(3);

/// A calendar that takes three seconds to answer, and counts the calls.
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

/// A notes service that takes three seconds per search, and counts them.
struct SlowNotes(AtomicUsize);

impl NoteSource for SlowNotes {
    fn search(&self, query: &str) -> Result<Vec<NoteHit>, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(SLOW);
        Ok(if query == "orion" { vec![NoteHit { title: "Orion notes".into(), modified_at: "2026-11-01".into() }] } else { Vec::new() })
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
    let fetcher = Fetcher::spawn(calendar.clone(), notes.clone());

    // Every tick returns at once, whatever the services are doing: nothing to wait for.
    let mut longest = Duration::ZERO;
    let mut cards = 0;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(30) && cards == 0 {
        let t = Instant::now();
        let out = tick_with(&conn, "Pat", &fetcher, n, true);
        longest = longest.max(t.elapsed());
        cards = out.cards.len();
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(longest < Duration::from_millis(1500), "a tick took {longest:?} while the services each take {SLOW:?}");
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
    let fetcher = Fetcher::spawn(calendar.clone(), notes.clone());
    for _ in 0..3 {
        tick_with(&conn, "Pat", &fetcher, n, false);
    }
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(calendar.calls.load(Ordering::SeqCst), 0);
    assert_eq!(notes.0.load(Ordering::SeqCst), 0);
}

#[test]
fn an_idle_tick_asks_the_calendar_once_and_the_notes_never() {
    let conn = db();
    let n = now();
    // The only event is hours away: nothing is in the window, so no brief is being made.
    let calendar = Arc::new(SlowCalendar {
        events: vec![event("e1", "Orion launch", n + Span::hours(3), vec![attendee("Maya Chen", "maya@work.test")])],
        calls: AtomicUsize::new(0),
    });
    let notes = Arc::new(SlowNotes(AtomicUsize::new(0)));
    let fetcher = Fetcher::spawn(calendar.clone(), notes.clone());
    tick_with(&conn, "Pat", &fetcher, n, true);
    std::thread::sleep(Duration::from_millis(3600));
    tick_with(&conn, "Pat", &fetcher, n, true);
    std::thread::sleep(Duration::from_millis(3600));
    assert!(calendar.calls.load(Ordering::SeqCst) >= 2, "one calendar call per tick");
    assert!(calendar.calls.load(Ordering::SeqCst) <= 2);
    assert_eq!(notes.0.load(Ordering::SeqCst), 0, "no event in the window, so no notes call");
}
