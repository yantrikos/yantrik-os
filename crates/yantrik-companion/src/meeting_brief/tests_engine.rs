//! The engine over time: a calendar that blinks, an event that is edited, the outcome every brief
//! ends with, and that nothing a brief held outlives the meeting.

use chrono::{Duration, NaiveDateTime};
use rusqlite::Connection;
use yantrik_ipc_contracts::calendar::CalendarEvent;

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
    let first = tick(&conn, &cal, &sources, &own(), n, Some(true));
    assert_eq!((first.cards.len(), first.fresh.len()), (1, 1), "shown as it is, in the last minutes");
    assert_eq!(first.cards[0].brief.notes.len(), 0);
    notes.broken.set(false);
    let second = tick(&conn, &cal, &sources, &own(), n + Duration::seconds(30), Some(true));
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
    tick(&conn, &cal, &sources, &own(), n, Some(true));
    let end = tick(&conn, &cal, &sources, &own(), n + Duration::minutes(41), Some(true));
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
    tick(conn, &Forbidden, &sources, &own(), n + Duration::minutes(minutes), Some(false))
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

/// A tick whose switch is not known, with sources that panic when asked.
fn unknown_at(conn: &Connection, n: NaiveDateTime, minutes: i64) -> Outcome {
    let roots = FileRoots::default();
    let sources = Sources { emails: &Forbidden, notes: &Forbidden, promises: &Forbidden, files: &roots };
    tick(conn, &Forbidden, &sources, &own(), n + Duration::minutes(minutes), None)
}

/// The service that holds the switch is down when the shell starts: the first tick does not know
/// it. That must not decide anything lasting (it used to end every brief as ignored, for good).
#[test]
fn a_switch_that_is_not_known_ends_nothing_and_the_card_returns_when_it_is() {
    let (conn, n) = (db(), now());
    let e = brief_up(&conn, n);
    let unknown = unknown_at(&conn, n, 5);
    assert!(unknown.cards.is_empty() && unknown.fresh.is_empty(), "nothing drawn, nothing announced");
    assert_eq!(held(&conn), 1, "the row is as it was");
    assert_eq!(over(&conn, &id_for(&e)), None, "and nothing ended it");
    assert_eq!(unknown.outcomes, Outcomes::default());
    // The service answers: the switch is on. The card is back, and is not announced again.
    let back = tick_at(&conn, &Calendar::of(vec![e.clone()]), n, 6);
    assert_eq!(back.cards.len(), 1, "the card is back");
    assert_eq!(over(&conn, &id_for(&e)), None, "and still open to an outcome");
    assert!(back.fresh.is_empty(), "with no second announcement");
    assert_eq!(back.outcomes, Outcomes::default(), "no ignored the person never chose");
}

/// What is purely a matter of time still happens when the switch is not known.
#[test]
fn a_meeting_that_ends_while_the_switch_is_not_known_is_still_emptied() {
    let (conn, n) = (db(), now());
    let e = brief_up(&conn, n);
    unknown_at(&conn, n, 41);
    assert_eq!(held(&conn), 0, "what the card said is gone with the meeting");
    assert_eq!(over(&conn, &id_for(&e)), Some(PrepOutcome::Ignored));
}

// ── What the file keeps ─────────────────────────────────────────────────────────────────

const SEED: &str = "SEEDED-STRING-IN-THE-DESCRIPTION";

/// A brief for an event whose description carries [`SEED`], made in a database file in WAL mode.
fn brief_in_a_file(name: &str) -> (std::path::PathBuf, Connection, CalendarEvent, NaiveDateTime) {
    let dir = scratch(name);
    let conn = Connection::open(dir.join("companion.db")).expect("database file");
    conn.execute_batch("PRAGMA journal_mode = WAL;").expect("wal");
    prepare(&conn);
    let (n, mut e) = (now(), event("e1", "Q4 planning", now() + Duration::minutes(10), vec![maya()]));
    e.description = format!("{SEED} and a line");
    assert_eq!(tick_at(&conn, &Calendar::of(vec![e.clone()]), n, 0).cards.len(), 1);
    (dir, conn, e, n)
}

/// Whether [`SEED`] is anywhere in the database's files, read as bytes after a TRUNCATE checkpoint.
fn is_the_text_left_in_the_file(dir: &std::path::Path, conn: &Connection) -> bool {
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).expect("checkpoint");
    let mut bytes = Vec::new();
    for file in std::fs::read_dir(dir).expect("dir").flatten() {
        bytes.extend(std::fs::read(file.path()).unwrap_or_default());
    }
    bytes.windows(SEED.len()).any(|w| w == SEED.as_bytes())
}

/// The meeting ends on the companion's own connection: the text is not left in the file.
#[test]
fn what_a_brief_said_is_not_in_the_database_file_once_its_meeting_is_over() {
    let (dir, conn, e, n) = brief_in_a_file("secure");
    assert_eq!(tick_at(&conn, &Calendar::of(vec![e]), n, 41).cards.len(), 0, "the meeting is over");
    assert!(!is_the_text_left_in_the_file(&dir, &conn), "the text left the file with the row");
}

/// The commonest end is Dismiss, written first on a connection of the shell's own that never ran
/// `ensure_table`. A setting of the companion's connection does not reach it, so the statement
/// itself must clear the text.
#[test]
fn a_dismissal_written_on_another_connection_leaves_no_text_in_the_file() {
    let (dir, conn, e, n) = brief_in_a_file("dismissed");
    let other = Connection::open(dir.join("companion.db")).expect("a second connection");
    press(&other, &id_for(&e), Press::Dismissed, now_ts(n + Duration::minutes(2)));
    assert_eq!(over(&conn, &id_for(&e)), Some(PrepOutcome::Dismissed));
    assert!(!is_the_text_left_in_the_file(&dir, &conn), "the dismissed brief's text stayed in the page");
}

/// The check above can see what it looks for: the same text, emptied by a plain statement with
/// nothing turned on, is still in the file.
#[test]
fn a_plain_update_of_the_same_row_leaves_the_text_in_the_file() {
    let (dir, conn, _, _) = brief_in_a_file("plain");
    conn.execute("UPDATE prepared_items SET data = '{}', description = '', trigger_data = '{}'", []).unwrap();
    assert!(is_the_text_left_in_the_file(&dir, &conn));
}

/// The pragma is the statement's own: nothing leaves it on, and nothing global is set for the
/// rest of the database.
#[test]
fn secure_delete_is_not_left_on_for_the_rest_of_the_database() {
    let (_, conn, e, n) = brief_in_a_file("pragma");
    let on = |c: &Connection| c.query_row("PRAGMA secure_delete", [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(on(&conn), 0, "not set by making a brief");
    press(&conn, &id_for(&e), Press::Dismissed, now_ts(n + Duration::minutes(2)));
    assert_eq!(on(&conn), 0, "and put back after the one that needed it");
}
