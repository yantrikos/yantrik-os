//! The engine: who gets a brief and when, once per event and start, change, dismissal, the
//! switch, and that no model or network client is anywhere in the gatherer.

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{Duration, Local, NaiveDateTime, Timelike};
use rusqlite::Connection;
use yantrik_ipc_contracts::calendar::{
    Attendee, AttendeeStatus, CalendarEvent, CreateEventParams, EventsParams, UpdateEventParams, UpsertRemoteEventParams,
};

use super::emails::{EmailSource, Mail};
use super::files::FileRoots;
use super::notes::{NoteHit, NoteSource};
use super::promises::{Promise, PromiseSource};
use super::*;
use crate::anticipation::{AnticipationEngine, PrepOutcome, PreparedKind};
use crate::calendar::backend::CalendarBackend;

pub(super) const ME: &str = "me@home.test";

pub(super) fn db() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory database");
    prepare(&conn);
    conn
}

pub(super) fn prepare(conn: &Connection) {
    AnticipationEngine::ensure_table(conn).expect("prepared_items table");
    crate::email::db::init_tables(conn);
    crate::world_model::WorldModel::ensure_tables(conn);
}

pub(super) fn own() -> Own {
    Own::new([ME.to_string()], "Pat")
}

pub(super) fn now() -> NaiveDateTime {
    Local::now().naive_local().with_nanosecond(0).expect("whole seconds")
}

fn stamp(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%S").to_string()
}

pub(super) fn attendee(name: &str, email: &str) -> Attendee {
    Attendee { name: name.into(), email: email.into(), status: AttendeeStatus::Pending }
}

pub(super) fn event(id: &str, title: &str, start: NaiveDateTime, people: Vec<Attendee>) -> CalendarEvent {
    CalendarEvent {
        id: id.into(),
        title: title.into(),
        description: String::new(),
        start: stamp(start),
        end: stamp(start + Duration::minutes(30)),
        is_all_day: false,
        location: None,
        attendees: people,
        recurrence: None,
        calendar_id: "local".into(),
        remote_id: None,
        creator: None,
        reminder_minutes: 10,
    }
}

pub(super) fn maya() -> Attendee {
    attendee("Maya Chen", "maya@work.test")
}

pub(super) struct Calendar(Mutex<Vec<CalendarEvent>>);

impl Calendar {
    pub(super) fn of(events: Vec<CalendarEvent>) -> Self {
        Self(Mutex::new(events))
    }

    pub(super) fn set(&self, events: Vec<CalendarEvent>) {
        *self.0.lock().expect("lock") = events;
    }
}

impl CalendarBackend for Calendar {
    fn list(&self, _: &EventsParams) -> Result<Vec<CalendarEvent>, String> {
        Ok(self.0.lock().expect("lock").clone())
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

/// One message from Maya, an hour old.
pub(super) struct Mailbox(pub(super) Vec<Mail>);

impl EmailSource for Mailbox {
    fn exchanged(&self, _: &[String], _: i64, limit: usize) -> Result<Vec<Mail>, String> {
        Ok(self.0.iter().take(limit).cloned().collect())
    }
}

pub(super) struct Notebook {
    pub(super) titles: Vec<&'static str>,
    pub(super) broken: Cell<bool>,
    pub(super) asked: Cell<usize>,
}

impl Notebook {
    pub(super) fn with(titles: Vec<&'static str>) -> Self {
        Self { titles, broken: Cell::new(false), asked: Cell::new(0) }
    }
}

impl NoteSource for Notebook {
    fn search(&self, query: &str) -> Result<Vec<NoteHit>, String> {
        self.asked.set(self.asked.get() + 1);
        if self.broken.get() {
            return Err("the notes service is down".into());
        }
        let q = query.to_lowercase();
        Ok(self.titles.iter().filter(|t| t.to_lowercase().contains(&q)).map(|t| NoteHit { id: format!("{t}.md"), title: (*t).into(), modified_at: "2026-11-01T10:00:00".into() }).collect())
    }
}

pub(super) struct Nothing;

impl PromiseSource for Nothing {
    fn made_by(&self, _: &str) -> Result<Vec<Promise>, String> {
        Ok(Vec::new())
    }
    fn made_to(&self, _: &str) -> Result<Vec<Promise>, String> {
        Ok(Vec::new())
    }
}

/// A source that must never be asked: the person's switch is off.
pub(super) struct Forbidden;

impl EmailSource for Forbidden {
    fn exchanged(&self, _: &[String], _: i64, _: usize) -> Result<Vec<Mail>, String> {
        panic!("a source was asked while the switch was off");
    }
}
impl NoteSource for Forbidden {
    fn search(&self, _: &str) -> Result<Vec<NoteHit>, String> {
        panic!("a source was asked while the switch was off");
    }
}
impl PromiseSource for Forbidden {
    fn made_by(&self, _: &str) -> Result<Vec<Promise>, String> {
        panic!("a source was asked while the switch was off");
    }
    fn made_to(&self, _: &str) -> Result<Vec<Promise>, String> {
        panic!("a source was asked while the switch was off");
    }
}
impl CalendarBackend for Forbidden {
    fn list(&self, _: &EventsParams) -> Result<Vec<CalendarEvent>, String> {
        panic!("the calendar was read while the switch was off");
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

pub(super) fn slides_mail(now: NaiveDateTime) -> Mail {
    let at = (super::window::to_unix(now).expect("local time") - 3600) as f64;
    Mail {
        from_name: "Maya Chen".into(),
        from_addr: "maya@work.test".into(),
        subject: "Slides for Friday".into(),
        date_ts: at,
        account: ME.into(),
        folder: "INBOX".into(),
        id: "1".into(),
    }
}

/// One cached message from Maya, so a brief for a meeting with her is never empty.
pub(super) fn maya_email_row(conn: &Connection, now: NaiveDateTime) {
    let account = crate::email::db::ensure_account(conn, "Home", ME, "imap");
    let at = (super::window::to_unix(now).expect("local time") - 3600) as f64;
    crate::email::db::upsert_email(conn, account, 1, "INBOX", "maya@work.test", "Maya", ME, "Slides for Friday", at, "", true, "<1@t>", "");
}

pub(super) fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("meeting-brief-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// One tick with a mailbox that always has Maya's message, so a due meeting is never empty.
pub(super) fn tick_with_mail(conn: &Connection, calendar: &Calendar, now: NaiveDateTime) -> Outcome {
    let mail = Mailbox(vec![slides_mail(now)]);
    let notes = Notebook::with(vec![]);
    let roots = FileRoots::default();
    let sources = Sources { emails: &mail, notes: &notes, promises: &Nothing, files: &roots };
    tick(conn, calendar, &sources, &own(), now, true)
}

/// `now` as the table keeps it.
pub(super) fn now_ts(now: NaiveDateTime) -> f64 {
    super::window::to_unix(now).expect("local time") as f64
}

/// The outcome of an item that is over and holds nothing any more, or `None` when it is not so.
pub(super) fn over(conn: &Connection, id: &str) -> Option<PrepOutcome> {
    AnticipationEngine::get(conn, id).filter(|i| i.data == serde_json::json!({})).and_then(|i| i.outcome)
}

pub(super) fn id_for(e: &CalendarEvent) -> String {
    item_id(&e.id, super::window::to_unix(chrono::NaiveDateTime::parse_from_str(&e.start, "%Y-%m-%dT%H:%M:%S").expect("stamp")).expect("local"))
}

// ── Window ──────────────────────────────────────────────────────────────────────────────

#[test]
fn an_event_ten_minutes_away_gets_a_brief() {
    let (conn, n) = (db(), now());
    let e = event("e1", "Q4 planning", n + Duration::minutes(10), vec![attendee("Pat", ME), maya()]);
    let out = tick_with_mail(&conn, &Calendar::of(vec![e.clone()]), n);
    assert_eq!(out.cards.len(), 1);
    assert_eq!(out.cards[0].id, id_for(&e));
    assert_eq!(out.fresh, vec![id_for(&e)]);
    assert_eq!(out.cards[0].brief.people, vec!["Maya Chen".to_string()], "the person's own address is left out");
    assert_eq!(out.cards[0].brief.when.chars().count(), 11, "10:00\u{2013}10:30 shaped");
}

#[test]
fn an_event_eleven_minutes_away_does_not_yet() {
    let (conn, n) = (db(), now());
    let e = event("e1", "Q4 planning", n + Duration::minutes(11), vec![maya()]);
    let cal = Calendar::of(vec![e.clone()]);
    let out = tick_with_mail(&conn, &cal, n);
    assert!(out.cards.is_empty() && out.fresh.is_empty());
    assert!(AnticipationEngine::get(&conn, &id_for(&e)).is_none(), "nothing was even gathered");
    // A minute later it is ten minutes away.
    let out = tick_with_mail(&conn, &cal, n + Duration::minutes(1));
    assert_eq!(out.cards.len(), 1);
}

#[test]
fn an_event_that_has_started_does_not() {
    let (conn, n) = (db(), now());
    let started = event("e1", "Q4 planning", n - Duration::minutes(1), vec![maya()]);
    let at_the_second = event("e2", "Q4 planning", n, vec![maya()]);
    let out = tick_with_mail(&conn, &Calendar::of(vec![started, at_the_second]), n);
    assert!(out.cards.is_empty());
}

#[test]
fn an_all_day_event_never_does() {
    let (conn, n) = (db(), now());
    let mut e = event("e1", "Offsite", n + Duration::minutes(5), vec![maya()]);
    e.is_all_day = true;
    assert!(tick_with_mail(&conn, &Calendar::of(vec![e]), n).cards.is_empty());
}

#[test]
fn an_event_with_nobody_but_the_person_never_does() {
    let (conn, n) = (db(), now());
    let alone = event("e1", "Focus time", n + Duration::minutes(5), vec![]);
    let only_me = event("e2", "Focus time", n + Duration::minutes(5), vec![attendee("Pat", "ME@home.test")]);
    assert!(tick_with_mail(&conn, &Calendar::of(vec![alone, only_me]), n).cards.is_empty());
}

#[test]
fn a_cancelled_event_never_does() {
    let (conn, n) = (db(), now());
    let a = event("e1", "Cancelled: Q4 planning", n + Duration::minutes(5), vec![maya()]);
    let b = event("e2", "Canceled - Q4 planning", n + Duration::minutes(5), vec![maya()]);
    assert!(tick_with_mail(&conn, &Calendar::of(vec![a, b]), n).cards.is_empty());
}

// ── Once, change, dismissal ─────────────────────────────────────────────────────────────

#[test]
fn a_brief_is_stored_and_announced_once_per_event_and_start() {
    let (conn, n) = (db(), now());
    let cal = Calendar::of(vec![event("e1", "Q4 planning", n + Duration::minutes(10), vec![maya()])]);
    assert_eq!(tick_with_mail(&conn, &cal, n).fresh.len(), 1);
    for minute in 1..=5 {
        let out = tick_with_mail(&conn, &cal, n + Duration::minutes(minute));
        assert!(out.fresh.is_empty(), "minute {minute}: announced again");
        assert_eq!(out.cards.len(), 1, "minute {minute}: still on screen");
    }
    let rows: i64 = conn.query_row("SELECT COUNT(*) FROM prepared_items", [], |r| r.get(0)).expect("count");
    assert_eq!(rows, 1);
}

#[test]
fn a_dismissed_brief_stays_gone_after_a_restart() {
    let dir = scratch("dismiss");
    let file = dir.join("companion.db");
    let n = now();
    let cal = Calendar::of(vec![event("e1", "Q4 planning", n + Duration::minutes(10), vec![maya()])]);
    let id = {
        let conn = Connection::open(&file).expect("open");
        prepare(&conn);
        let out = tick_with_mail(&conn, &cal, n);
        press(&conn, &out.cards[0].id, Press::Dismissed, now_ts(n));
        out.cards[0].id.clone()
    };
    // A new process: a new connection to the same file.
    let conn = Connection::open(&file).expect("reopen");
    prepare(&conn);
    let out = tick_with_mail(&conn, &cal, n + Duration::minutes(2));
    assert!(out.cards.is_empty() && out.fresh.is_empty(), "the dismissed brief came back");
    assert!(AnticipationEngine::get(&conn, &id).is_some_and(|i| i.outcome.is_some()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_brief_that_was_not_dismissed_comes_back_after_a_restart_without_a_second_toast() {
    let dir = scratch("restart");
    let file = dir.join("companion.db");
    let n = now();
    let cal = Calendar::of(vec![event("e1", "Q4 planning", n + Duration::minutes(10), vec![maya()])]);
    {
        let conn = Connection::open(&file).expect("open");
        prepare(&conn);
        assert_eq!(tick_with_mail(&conn, &cal, n).fresh.len(), 1);
    }
    let conn = Connection::open(&file).expect("reopen");
    prepare(&conn);
    let out = tick_with_mail(&conn, &cal, n + Duration::minutes(3));
    assert_eq!(out.cards.len(), 1);
    assert!(out.fresh.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn moving_an_events_start_replaces_its_brief() {
    let (conn, n) = (db(), now());
    let first = event("e1", "Q4 planning", n + Duration::minutes(10), vec![maya()]);
    let cal = Calendar::of(vec![first.clone()]);
    tick_with_mail(&conn, &cal, n);

    let moved = event("e1", "Q4 planning", n + Duration::minutes(9) + Duration::seconds(1), vec![maya()]);
    cal.set(vec![moved.clone()]);
    let out = tick_with_mail(&conn, &cal, n);
    assert_eq!(over(&conn, &id_for(&first)), Some(PrepOutcome::Expired), "the old item is over, and holds nothing");
    assert_eq!(out.cards.len(), 1);
    assert_eq!(out.cards[0].id, id_for(&moved));
    assert_eq!(out.fresh, vec![id_for(&moved)], "the new one is shown");

    // Moved out of the window altogether: nothing is left.
    cal.set(vec![event("e1", "Q4 planning", n + Duration::hours(3), vec![maya()])]);
    let out = tick_with_mail(&conn, &cal, n);
    assert!(out.cards.is_empty());
    assert_eq!(over(&conn, &id_for(&moved)), Some(PrepOutcome::Expired));
}

#[test]
fn a_changed_title_or_guest_list_replaces_the_brief_without_announcing_it_again() {
    let (conn, n) = (db(), now());
    let at = n + Duration::minutes(10);
    let cal = Calendar::of(vec![event("e1", "Q4 planning", at, vec![maya()])]);
    tick_with_mail(&conn, &cal, n);
    cal.set(vec![event("e1", "Q4 budget", at, vec![maya(), attendee("Ravi Rao", "ravi@work.test")])]);
    let out = tick_with_mail(&conn, &cal, n + Duration::minutes(1));
    assert!(out.fresh.is_empty());
    assert_eq!(out.cards.len(), 1);
    assert_eq!(out.cards[0].brief.title, "Q4 budget");
    assert_eq!(out.cards[0].brief.people.len(), 2);
}

#[test]
fn an_event_that_is_deleted_or_called_off_takes_its_card_away() {
    let (conn, n) = (db(), now());
    let at = n + Duration::minutes(10);
    let cal = Calendar::of(vec![event("e1", "Q4 planning", at, vec![maya()])]);
    assert_eq!(tick_with_mail(&conn, &cal, n).cards.len(), 1);
    cal.set(vec![event("e1", "Cancelled: Q4 planning", at, vec![maya()])]);
    assert!(tick_with_mail(&conn, &cal, n + Duration::minutes(1)).cards.is_empty());
}

#[test]
fn the_card_goes_when_the_meeting_ends() {
    let (conn, n) = (db(), now());
    let cal = Calendar::of(vec![event("e1", "Q4 planning", n + Duration::minutes(10), vec![maya()])]);
    assert_eq!(tick_with_mail(&conn, &cal, n).cards.len(), 1);
    let after = n + Duration::minutes(41);
    cal.set(vec![]);
    assert!(cards(&conn, super::window::to_unix(after).expect("local")).is_empty());
}

// ── Empty ───────────────────────────────────────────────────────────────────────────────

#[test]
fn a_brief_that_says_nothing_gets_no_card_and_no_toast_and_is_not_gathered_again() {
    let (conn, n) = (db(), now());
    let cal = Calendar::of(vec![event("e1", "Q4 planning", n + Duration::minutes(10), vec![maya()])]);
    let mail = Mailbox(vec![]);
    let notes = Notebook::with(vec![]);
    let roots = FileRoots::default();
    let sources = Sources { emails: &mail, notes: &notes, promises: &Nothing, files: &roots };
    let out = tick(&conn, &cal, &sources, &own(), n, true);
    assert!(out.cards.is_empty() && out.fresh.is_empty());
    let asked = notes.asked.get();
    assert!(asked > 0, "the sources were asked");
    let out = tick(&conn, &cal, &sources, &own(), n + Duration::minutes(1), true);
    assert!(out.cards.is_empty() && out.fresh.is_empty());
    assert_eq!(notes.asked.get(), asked, "nothing to say is remembered, not asked again every minute");
}

#[test]
fn a_source_that_did_not_answer_is_not_recorded_as_nothing_to_say() {
    let (conn, n) = (db(), now());
    let cal = Calendar::of(vec![event("e1", "Q4 planning", n + Duration::minutes(10), vec![maya()])]);
    let mail = Mailbox(vec![]);
    let notes = Notebook::with(vec!["Q4 planning notes"]);
    notes.broken.set(true);
    let roots = FileRoots::default();
    let sources = Sources { emails: &mail, notes: &notes, promises: &Nothing, files: &roots };
    assert!(tick(&conn, &cal, &sources, &own(), n, true).cards.is_empty());
    notes.broken.set(false);
    let out = tick(&conn, &cal, &sources, &own(), n + Duration::minutes(1), true);
    assert_eq!(out.cards.len(), 1, "the next tick tried again");
    assert_eq!(out.fresh.len(), 1);
}

// ── The switch ──────────────────────────────────────────────────────────────────────────

#[test]
fn with_the_switch_off_nothing_is_read_or_gathered() {
    let (conn, n) = (db(), now());
    let roots = FileRoots::default();
    let sources = Sources { emails: &Forbidden, notes: &Forbidden, promises: &Forbidden, files: &roots };
    let out = tick(&conn, &Forbidden, &sources, &own(), n, false);
    assert!(out.cards.is_empty() && out.fresh.is_empty());
    let rows: i64 = conn.query_row("SELECT COUNT(*) FROM prepared_items", [], |r| r.get(0)).expect("count");
    assert_eq!(rows, 0);
}

// ── No model, no network ────────────────────────────────────────────────────────────────

/// The gatherer takes four local sources as trait objects and nothing else, so a build without
/// any client compiles and runs (every test above is one). This one only guards the naming: it
/// reads the source for the names of a model or a network client, and proves nothing beyond it.
#[test]
fn no_file_of_the_gatherer_names_a_model_or_a_network_client() {
    let files: [(&str, &str); 13] = [
        ("assemble.rs", include_str!("assemble.rs")),
        ("emails.rs", include_str!("emails.rs")),
        ("engine.rs", include_str!("engine.rs")),
        ("files.rs", include_str!("files.rs")),
        ("keywords.rs", include_str!("keywords.rs")),
        ("local.rs", include_str!("local.rs")),
        ("model.rs", include_str!("model.rs")),
        ("mod.rs", include_str!("mod.rs")),
        ("notes.rs", include_str!("notes.rs")),
        ("prefetch.rs", include_str!("prefetch.rs")),
        ("promises.rs", include_str!("promises.rs")),
        ("remote.rs", include_str!("remote.rs")),
        ("window.rs", include_str!("window.rs")),
    ];
    let forbidden = [
        ["ur", "eq"].concat(),
        ["req", "west"].concat(),
        ["hy", "per"].concat(),
        ["Tcp", "Stream"].concat(),
        ["Udp", "Socket"].concat(),
        ["ht", "tp"].concat(),
        ["LLM", "Backend"].concat(),
        ["yantrik", "_ml"].concat(),
        ["Her", "mes"].concat(),
        ["Mind", "Client"].concat(),
        ["send", "_message"].concat(),
        ["run", "_tool"].concat(),
    ];
    for (name, text) in files {
        for word in &forbidden {
            assert!(!text.contains(word.as_str()), "{name} mentions `{word}`");
        }
    }
    for (name, text) in files {
        assert!(text.lines().count() <= 600, "{name} is over 600 lines");
    }
}

#[test]
fn the_stored_item_is_the_anticipation_tables_own_kind() {
    let (conn, n) = (db(), now());
    let e = event("e1", "Q4 planning", n + Duration::minutes(10), vec![maya()]);
    tick_with_mail(&conn, &Calendar::of(vec![e.clone()]), n);
    let item = AnticipationEngine::get(&conn, &id_for(&e)).expect("stored");
    assert_eq!(item.kind, PreparedKind::MeetingBrief);
    assert!(item.surfaced, "announced, so marked");
    assert_eq!(item.id, format!("meeting:e1:{}", super::window::to_unix(chrono::NaiveDateTime::parse_from_str(&e.start, "%Y-%m-%dT%H:%M:%S").expect("stamp")).expect("local")));
}
