//! The engine over time: a calendar that blinks, an event that is edited, the outcome every brief
//! ends with, and that nothing a brief held outlives the meeting.

use chrono::{Duration, NaiveDateTime};
use rusqlite::Connection;

use super::files::FileRoots;
use super::tests::{db, event, id_for, maya, now, now_ts, over, own, prepare, scratch, tick_with_mail, Calendar, Forbidden, Mailbox, Notebook, Nothing};
use super::*;
use crate::anticipation::{AnticipationEngine, PrepOutcome};

fn stamp(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn tick_at(conn: &Connection, cal: &Calendar, n: NaiveDateTime, minutes: i64) -> Outcome {
    tick_with_mail(conn, cal, n + Duration::minutes(minutes))
}

// ── A calendar that blinks ──────────────────────────────────────────────────────────────

#[test]
fn a_calendar_that_answers_empty_for_a_moment_neither_ends_the_brief_nor_toasts_it_again() {
    let (conn, n) = (db(), now());
    let e = event("e1", "Q4 planning", n + Duration::minutes(10), vec![maya()]);
    let cal = Calendar::of(vec![e.clone()]);
    let mut fresh = tick_at(&conn, &cal, n, 0).fresh.len();
    // The calendar rewrites its directory: for one tick the event is not listed.
    cal.set(vec![]);
    let blink = tick_at(&conn, &cal, n, 1);
    fresh += blink.fresh.len();
    assert_eq!(blink.cards.len(), 1, "the card stays while the calendar may only be blinking");
    cal.set(vec![e]);
    let back = tick_at(&conn, &cal, n, 2);
    fresh += back.fresh.len();
    assert_eq!(back.cards.len(), 1, "and it is the same card");
    assert_eq!(fresh, 1, "one announcement in all");
}

#[test]
fn an_event_the_calendar_keeps_out_is_ended_and_what_it_held_is_emptied() {
    let (conn, n) = (db(), now());
    let e = event("e1", "Q4 planning", n + Duration::minutes(10), vec![maya()]);
    let cal = Calendar::of(vec![e.clone()]);
    tick_at(&conn, &cal, n, 0);
    cal.set(vec![]);
    tick_at(&conn, &cal, n, 1);
    let gone = tick_at(&conn, &cal, n, 3);
    assert!(gone.cards.is_empty());
    assert_eq!(over(&conn, &id_for(&e)), Some(PrepOutcome::Ignored), "a card that was shown and never pressed ends as ignored, however it ended");
}

// ── An edited event ─────────────────────────────────────────────────────────────────────

#[test]
fn an_extended_meeting_keeps_its_card_and_says_the_new_end_without_a_second_toast() {
    let (conn, n) = (db(), now());
    let at = n + Duration::minutes(10);
    let mut e = event("e1", "Q4 planning", at, vec![maya()]);
    let cal = Calendar::of(vec![e.clone()]);
    let first = tick_at(&conn, &cal, n, 0);
    let before = first.cards[0].brief.when.clone();
    e.end = stamp(at + Duration::minutes(60));
    e.location = Some("Room 9".into());
    cal.set(vec![e.clone()]);
    let out = tick_at(&conn, &cal, n, 1);
    assert!(out.fresh.is_empty());
    assert_ne!(out.cards[0].brief.when, before, "the time range follows the edit");
    assert_eq!(out.cards[0].brief.place.as_deref(), Some("Room 9"));
    // The card stays until the new end: still there 45 minutes after the old one.
    assert_eq!(cards(&conn, super::window::to_unix(at + Duration::minutes(45)).expect("local")).len(), 1);
    // And an edit made after the meeting began still reaches the card.
    e.location = Some("Room 12".into());
    cal.set(vec![e]);
    let during = tick_at(&conn, &cal, n, 15);
    assert_eq!(during.cards[0].brief.place.as_deref(), Some("Room 12"));
}

// ── A brief shown at the last call ──────────────────────────────────────────────────────

#[test]
fn a_brief_shown_without_a_source_is_filled_in_when_it_answers_and_not_announced_twice() {
    let (conn, n) = (db(), now());
    let cal = Calendar::of(vec![event("e1", "Q4 planning", n + Duration::minutes(3), vec![maya()])]);
    let mail = Mailbox(vec![super::tests::slides_mail(n)]);
    let notes = Notebook::with(vec!["Q4 planning notes"]);
    notes.broken.set(true);
    let roots = FileRoots::default();
    let sources = Sources { emails: &mail, notes: &notes, promises: &Nothing, files: &roots };
    let first = tick(&conn, &cal, &sources, &own(), n, true);
    assert_eq!((first.cards.len(), first.fresh.len()), (1, 1), "shown as it is, in the last minutes");
    assert_eq!(first.cards[0].brief.notes.len(), 0);
    notes.broken.set(false);
    let second = tick(&conn, &cal, &sources, &own(), n + Duration::seconds(30), true);
    assert!(second.fresh.is_empty(), "the card is filled in, not announced again");
    assert_eq!(second.cards[0].brief.notes.len(), 1, "the source answered later and the brief has it");
}

// ── What a finished brief keeps ─────────────────────────────────────────────────────────

#[test]
fn nothing_a_brief_held_is_in_the_database_after_the_meeting() {
    let (conn, n) = (db(), now());
    let mut e = event("e1", "Q4 planning", n + Duration::minutes(10), vec![maya()]);
    e.description = "SEED-DESCRIPTION line".into();
    e.location = Some("SEED-ROOM".into());
    let cal = Calendar::of(vec![e.clone()]);
    let out = tick_at(&conn, &cal, n, 0);
    assert_eq!(out.cards.len(), 1);
    let held = |conn: &Connection| -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM prepared_items WHERE data LIKE '%SEED%' OR data LIKE '%Slides%' OR description LIKE '%Q4%' OR trigger_data LIKE '%Q4%'",
            [],
            |r| r.get(0),
        )
        .expect("count")
    };
    assert_eq!(held(&conn), 1, "while the meeting is on, the card is in the table");
    let after = tick_at(&conn, &cal, n, 41);
    assert!(after.cards.is_empty());
    assert_eq!(held(&conn), 0, "after the end only the record is left");
    let item = AnticipationEngine::get(&conn, &id_for(&e)).expect("the record");
    assert_eq!(item.outcome, Some(PrepOutcome::Ignored), "shown and never pressed");
    assert!(item.surfaced);
}

// ── Outcomes ────────────────────────────────────────────────────────────────────────────

#[test]
fn every_brief_ends_with_one_outcome_opened_dismissed_or_ignored() {
    let (conn, n) = (db(), now());
    let at = n + Duration::minutes(10);
    let events: Vec<_> = ["opened", "dismissed", "ignored", "both"].iter().map(|id| event(id, "Q4 planning", at, vec![maya()])).collect();
    let cal = Calendar::of(events.clone());
    let out = tick_at(&conn, &cal, n, 0);
    assert_eq!(out.cards.len(), 4);
    let (now_f, id) = (now_ts(n), |i: usize| id_for(&events[i]));
    press(&conn, &id(0), Press::Opened, now_f);
    press(&conn, &id(1), Press::Dismissed, now_f);
    press(&conn, &id(3), Press::Opened, now_f);
    press(&conn, &id(3), Press::Dismissed, now_f);
    // An opened card stays on the desktop; a dismissed one goes at once.
    let mid = tick_at(&conn, &cal, n, 1);
    let mut live: Vec<&str> = mid.cards.iter().map(|c| c.brief.event_id.as_str()).collect();
    live.sort_unstable();
    assert_eq!(live, ["ignored", "opened"]);
    assert!(mid.fresh.is_empty());
    let end = tick_at(&conn, &cal, n, 41);
    assert_eq!(end.outcomes, Outcomes { opened: 2, dismissed: 1, ignored: 1 }, "one outcome each; opened then dismissed is opened");
    assert!(end.cards.is_empty());
}

#[test]
fn a_brief_nobody_saw_is_not_counted_as_ignored() {
    let (conn, n) = (db(), now());
    let cal = Calendar::of(vec![event("e1", "Q4 planning", n + Duration::minutes(10), vec![maya()])]);
    // Nothing to say: recorded so the sources are not asked again, never shown.
    let mail = Mailbox(vec![]);
    let notes = Notebook::with(vec![]);
    let roots = FileRoots::default();
    let sources = Sources { emails: &mail, notes: &notes, promises: &Nothing, files: &roots };
    tick(&conn, &cal, &sources, &own(), n, true);
    let end = tick(&conn, &cal, &sources, &own(), n + Duration::minutes(41), true);
    assert_eq!(end.outcomes, Outcomes::default());
}

// ── The switch is off, or not known ─────────────────────────────────────────────────────

/// What the brief held, counted in the whole table.
fn held(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM prepared_items WHERE data LIKE '%SEED%' OR data LIKE '%Slides%' OR description LIKE '%Q4%' OR trigger_data LIKE '%Q4%'",
        [],
        |r| r.get(0),
    )
    .expect("count")
}

/// A tick with the switch off, and sources that panic when asked.
fn off_at(conn: &Connection, n: NaiveDateTime, minutes: i64) -> Outcome {
    let roots = FileRoots::default();
    let sources = Sources { emails: &Forbidden, notes: &Forbidden, promises: &Forbidden, files: &roots };
    tick(conn, &Forbidden, &sources, &own(), n + Duration::minutes(minutes), false)
}

fn brief_up(conn: &Connection, n: NaiveDateTime) -> CalendarEvent {
    let mut e = event("e1", "Q4 planning", n + Duration::minutes(10), vec![maya()]);
    e.description = "SEED-DESCRIPTION line".into();
    let out = tick_at(conn, &Calendar::of(vec![e.clone()]), n, 0);
    assert_eq!(out.cards.len(), 1);
    assert_eq!(held(conn), 1, "the card is in the table");
    e
}

#[test]
fn turning_briefs_off_takes_back_what_a_card_held_without_asking_any_source() {
    let (conn, n) = (db(), now());
    let e = brief_up(&conn, n);
    let out = off_at(&conn, n, 5);
    assert!(out.cards.is_empty() && out.fresh.is_empty());
    assert_eq!(held(&conn), 0, "nothing it said is left, with the meeting still ahead");
    assert_eq!(over(&conn, &id_for(&e)), Some(PrepOutcome::Ignored), "it was shown and not pressed");
    // And it stays over for this meeting when briefs are turned on again.
    let back = tick_at(&conn, &Calendar::of(vec![e]), n, 6);
    assert!(back.cards.is_empty() && back.fresh.is_empty());
}

#[test]
fn a_meeting_that_ends_while_briefs_are_off_still_leaves_only_its_record() {
    let (conn, n) = (db(), now());
    let e = brief_up(&conn, n);
    off_at(&conn, n, 41);
    assert_eq!(held(&conn), 0);
    assert_eq!(over(&conn, &id_for(&e)), Some(PrepOutcome::Ignored));
}

#[test]
fn an_off_tick_does_not_zero_the_outcomes_it_does_not_gather() {
    let (conn, n) = (db(), now());
    let e = brief_up(&conn, n);
    press(&conn, &id_for(&e), Press::Dismissed, now_ts(n));
    let out = off_at(&conn, n, 5);
    assert_eq!(out.outcomes, Outcomes { opened: 0, dismissed: 1, ignored: 0 }, "the table's own count, not a default");
}

// ── What the file keeps ─────────────────────────────────────────────────────────────────

/// A brief is made and its meeting ends, in a database file in WAL mode, and the file and its log
/// are read as bytes after a checkpoint: whether what the card said is still in them.
fn is_the_text_left_in_the_file(name: &str, secure_delete: bool) -> bool {
    const SEED: &str = "SEEDED-STRING-IN-THE-DESCRIPTION";
    let dir = scratch(name);
    let conn = Connection::open(dir.join("companion.db")).expect("database file");
    conn.execute_batch("PRAGMA journal_mode = WAL;").expect("wal");
    prepare(&conn);
    if !secure_delete {
        conn.execute_batch("PRAGMA secure_delete = OFF;").expect("pragma");
    }
    let (n, mut e) = (now(), event("e1", "Q4 planning", now() + Duration::minutes(10), vec![maya()]));
    e.description = format!("{SEED} and a line");
    let cal = Calendar::of(vec![e]);
    assert_eq!(tick_at(&conn, &cal, n, 0).cards.len(), 1);
    assert_eq!(tick_at(&conn, &cal, n, 41).cards.len(), 0, "the meeting is over");
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).expect("checkpoint");
    let mut bytes = Vec::new();
    for file in std::fs::read_dir(&dir).expect("dir").flatten() {
        bytes.extend(std::fs::read(file.path()).unwrap_or_default());
    }
    bytes.windows(SEED.len()).any(|w| w == SEED.as_bytes())
}

#[test]
fn what_a_brief_said_is_not_in_the_database_file_once_its_meeting_is_over() {
    assert!(!is_the_text_left_in_the_file("secure", true), "the text left the file with the row");
}

/// The check above can see what it looks for: with `secure_delete` off the same run leaves it.
#[test]
fn without_secure_delete_the_same_text_stays_in_the_file() {
    assert!(is_the_text_left_in_the_file("insecure", false));
}
