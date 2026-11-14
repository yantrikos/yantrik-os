//! The four sources and the words they search with.

use std::time::{Duration, SystemTime};

use super::emails::{self, EmailSource, Mail};
use super::files::{self, FileRoots, MAX_ENTRIES};
use super::keywords::{person_terms, title_keywords};
use super::local::{EmailCache, WorldModelPromises};
use super::model::{age, Open, LOOKBACK_DAYS};
use super::notes::{self, NoteHit, NoteSource};
use super::promises::{self, Promise, PromiseSource};
use super::tests::{db, own, ME};
use super::window::Person;
use crate::world_model::{Commitment, CommitmentSource, CommitmentStatus, WorldModel};

const NOW: i64 = 1_800_000_000;
const DAY: i64 = 86_400;

fn person(name: &str, email: &str) -> Person {
    Person { name: name.into(), email: email.into() }
}

fn insert_mail(conn: &rusqlite::Connection, uid: u32, from: &str, to: &str, subject: &str, days_ago: f64) {
    let account = crate::email::db::ensure_account(conn, "Home", ME, "imap");
    crate::email::db::upsert_email(
        conn,
        account,
        uid,
        "INBOX",
        from,
        "",
        to,
        subject,
        NOW as f64 - days_ago * DAY as f64,
        "",
        true,
        &format!("<{uid}@test>"),
        "",
    );
}

// ── Email ───────────────────────────────────────────────────────────────────────────────

#[test]
fn email_matches_the_sender_and_the_recipient_newest_first() {
    let conn = db();
    insert_mail(&conn, 1, "maya@work.test", ME, "From Maya", 5.0);
    insert_mail(&conn, 2, ME, "Maya Chen <MAYA@work.test>, ravi@work.test", "To Maya", 2.0);
    insert_mail(&conn, 3, "stranger@else.test", ME, "Not about them", 1.0);
    let found = crate::email::db::emails_with_participants(&conn, &["maya@work.test".to_string()], 0.0, 10);
    let subjects: Vec<&str> = found.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, ["To Maya", "From Maya"]);
}

#[test]
fn an_address_is_a_value_never_sql() {
    let conn = db();
    insert_mail(&conn, 1, "maya@work.test", ME, "From Maya", 1.0);
    let hostile = ["x' OR '1'='1".to_string(), "%".to_string(), "' ; DROP TABLE emails; --".to_string()];
    assert!(crate::email::db::emails_with_participants(&conn, &hostile, 0.0, 10).is_empty());
    assert_eq!(crate::email::db::emails_with_participants(&conn, &["maya@work.test".to_string()], 0.0, 10).len(), 1, "the table is still there");
}

#[test]
fn the_section_takes_three_messages_from_the_last_thirty_days_and_names_the_sender() {
    let conn = db();
    insert_mail(&conn, 1, "maya@work.test", ME, "Slides for Friday", 2.0);
    insert_mail(&conn, 2, ME, "maya@work.test", "Re: Slides for Friday", 3.0);
    insert_mail(&conn, 3, "maya@work.test", ME, "Budget", 10.0);
    insert_mail(&conn, 4, "maya@work.test", ME, "Lunch", 20.0);
    insert_mail(&conn, 5, "maya@work.test", ME, "Last quarter", (LOOKBACK_DAYS + 1) as f64);
    let rows = emails::section(&EmailCache { conn: &conn }, &[person("Maya Chen", "maya@work.test")], &own(), NOW).expect("rows");
    let text: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text, ["Maya \u{b7} Slides for Friday \u{b7} 2 d", "You \u{b7} Re: Slides for Friday \u{b7} 3 d", "Maya \u{b7} Budget \u{b7} 10 d"]);
    assert_eq!(rows[0].open, Some(Open::Message { subject: "Slides for Friday".into() }));
}

#[test]
fn the_section_drops_what_the_source_returned_past_thirty_days() {
    struct Old;
    impl EmailSource for Old {
        fn exchanged(&self, _: &[String], _: i64, _: usize) -> Result<Vec<Mail>, String> {
            Ok(vec![Mail { from_name: "Maya".into(), from_addr: "maya@work.test".into(), subject: "Ancient".into(), date_ts: (NOW - 31 * DAY) as f64 }])
        }
    }
    assert!(emails::section(&Old, &[person("Maya", "maya@work.test")], &own(), NOW).expect("rows").is_empty());
}

#[test]
fn ages_read_in_minutes_hours_and_days() {
    assert_eq!(age(NOW, NOW - 30), "1 min");
    assert_eq!(age(NOW, NOW - 600), "10 min");
    assert_eq!(age(NOW, NOW - 3 * 3600), "3 h");
    assert_eq!(age(NOW, NOW - 2 * DAY - 5), "2 d");
}

// ── Words ───────────────────────────────────────────────────────────────────────────────

#[test]
fn stop_words_and_short_words_are_not_keywords() {
    assert!(title_keywords("Meeting sync call with the team").contains(&"team".to_string()));
    for stop in ["meeting", "sync", "call", "the", "with"] {
        assert!(!title_keywords(&format!("{stop} Orion")).contains(&stop.to_string()), "{stop}");
    }
    assert_eq!(title_keywords("Q4 to do: a Orion launch"), ["orion", "launch"], "one- and two-letter words are not used");
    assert_eq!(title_keywords("Orion Orion ORION"), ["orion"]);
    assert!(title_keywords("Sync").is_empty());
}

#[test]
fn a_person_is_searched_by_the_whole_name_and_the_first() {
    assert_eq!(person_terms("Maya Chen"), ["Maya Chen", "Maya"]);
    assert_eq!(person_terms("Ravi"), ["Ravi"]);
    assert!(person_terms("Al").is_empty());
    assert_eq!(super::keywords::display_name("", "maya.chen@work.test"), "Maya Chen");
}

// ── Notes ───────────────────────────────────────────────────────────────────────────────

struct Notebook(Vec<(&'static str, &'static str)>);

impl NoteSource for Notebook {
    fn search(&self, query: &str) -> Result<Vec<NoteHit>, String> {
        let q = query.to_lowercase();
        Ok(self.0.iter().filter(|(t, _)| t.to_lowercase().contains(&q)).map(|(t, m)| NoteHit { title: (*t).into(), modified_at: (*m).into() }).collect())
    }
}

#[test]
fn notes_match_the_titles_words_and_an_attendees_name() {
    let book = Notebook(vec![
        ("Orion launch checklist", "2026-11-01"),
        ("Call with Maya Chen", "2026-11-02"),
        ("Groceries", "2026-11-03"),
        ("Meeting notes", "2026-11-04"),
    ]);
    let rows = notes::section(&book, "Orion sync meeting", &[person("Maya Chen", "maya@work.test")]).expect("rows");
    let titles: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(titles, ["Call with Maya Chen", "Orion launch checklist"], "\"meeting\" and \"sync\" are not searched; \"Meeting notes\" is not found by them");
    assert_eq!(rows[0].open, Some(Open::Note { title: "Call with Maya Chen".into() }));
}

#[test]
fn at_most_three_notes_the_best_matches_first() {
    let book = Notebook(vec![
        ("Orion a", "2026-11-01"),
        ("Orion b", "2026-11-02"),
        ("Orion c", "2026-11-03"),
        ("Orion d", "2026-11-04"),
        ("Orion and Maya Chen", "2026-10-01"),
    ]);
    let rows = notes::section(&book, "Orion", &[person("Maya Chen", "maya@work.test")]).expect("rows");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].text, "Orion and Maya Chen", "matched by the title word and by the name");
    assert_eq!(rows[1].text, "Orion d", "then the newest");
}

#[test]
fn the_notes_service_failing_is_an_error_not_an_empty_section() {
    struct Down;
    impl NoteSource for Down {
        fn search(&self, _: &str) -> Result<Vec<NoteHit>, String> {
            Err("down".into())
        }
    }
    assert!(notes::section(&Down, "Orion", &[]).is_err());
}

// ── Promises ────────────────────────────────────────────────────────────────────────────

fn commitment(promisor: &str, promisee: &str, action: &str, source: CommitmentSource) -> Commitment {
    Commitment {
        id: 0,
        promisor: promisor.into(),
        promisee: promisee.into(),
        action: action.into(),
        deadline: 0.0,
        status: CommitmentStatus::Pending,
        confidence: 0.9,
        source,
        evidence_text: String::new(),
        related_entities: Vec::new(),
        created_at: NOW as f64,
        updated_at: NOW as f64,
        completion_evidence: None,
    }
}

#[test]
fn promises_match_in_both_directions_without_regard_to_case_and_skip_done_ones() {
    let conn = db();
    WorldModel::insert_commitment(&conn, &commitment("Pat", "ravi rao", "send the Q4 numbers", CommitmentSource::Manual));
    WorldModel::insert_commitment(&conn, &commitment("MAYA CHEN", "Pat", "share the deck", CommitmentSource::Email { message_id: "<1@x>".into(), subject: "The deck".into() }));
    let mut done = commitment("Pat", "Ravi Rao", "old thing", CommitmentSource::Manual);
    done.status = CommitmentStatus::Completed;
    WorldModel::insert_commitment(&conn, &done);
    WorldModel::insert_commitment(&conn, &commitment("Pat", "Someone Else", "unrelated", CommitmentSource::Manual));

    let people = [person("Ravi Rao", "ravi@work.test"), person("Maya Chen", "maya@work.test")];
    let rows = promises::section(&WorldModelPromises { conn: &conn }, &people).expect("rows");
    let text: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text, ["You told Ravi: send the Q4 numbers", "Maya said: share the deck"]);
    assert_eq!(rows[0].open, None, "a promise made by hand has nothing to open");
    assert_eq!(rows[1].open, Some(Open::Message { subject: "The deck".into() }));
}

#[test]
fn at_most_three_promises() {
    struct Many;
    impl PromiseSource for Many {
        fn made_by(&self, name: &str) -> Result<Vec<Promise>, String> {
            Ok((0..5).map(|i| Promise { text: format!("{name} item {i}"), open: None }).collect())
        }
        fn made_to(&self, _: &str) -> Result<Vec<Promise>, String> {
            Ok(Vec::new())
        }
    }
    assert_eq!(promises::section(&Many, &[person("Maya Chen", "m@x.test")]).expect("rows").len(), 3);
}

// ── Files ───────────────────────────────────────────────────────────────────────────────

fn home(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("meeting-brief-files-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in ["Documents", "Desktop", "Downloads"] {
        std::fs::create_dir_all(dir.join(d)).expect("dir");
    }
    dir
}

fn words(w: &[&str]) -> Vec<String> {
    w.iter().map(|s| s.to_string()).collect()
}

#[test]
fn files_match_on_the_name_only_and_never_on_contents() {
    let h = home("names");
    std::fs::write(h.join("Documents/orion-plan.md"), "plain").expect("write");
    std::fs::write(h.join("Downloads/notes.txt"), "all about orion in here").expect("write");
    std::fs::write(h.join("Desktop/ORION budget.xlsx"), "x").expect("write");
    let (rows, _) = files::section(&FileRoots::in_home(&h), &words(&["orion"]), SystemTime::now());
    let names: Vec<String> = rows.iter().map(|r| r.text.split(" \u{b7} ").next().unwrap_or("").to_string()).collect();
    assert_eq!(rows.len(), 2, "at most two");
    assert!(names.iter().all(|n| n.to_lowercase().contains("orion")), "{names:?}");
    assert!(rows.iter().all(|r| matches!(&r.open, Some(Open::File { path }) if path.starts_with(h.to_str().unwrap_or("")))));
    let _ = std::fs::remove_dir_all(h);
}

#[test]
fn files_older_than_thirty_days_and_hidden_ones_are_skipped() {
    let h = home("old");
    let old = h.join("Documents/orion-old.md");
    std::fs::write(&old, "x").expect("write");
    let file = std::fs::File::options().write(true).open(&old).expect("open");
    file.set_modified(SystemTime::now() - Duration::from_secs(40 * 86_400)).expect("mtime");
    std::fs::create_dir_all(h.join("Documents/.cache")).expect("dir");
    std::fs::write(h.join("Documents/.cache/orion-hidden.md"), "x").expect("write");
    std::fs::write(h.join("Documents/.orion-dot.md"), "x").expect("write");
    let (rows, _) = files::section(&FileRoots::in_home(&h), &words(&["orion"]), SystemTime::now());
    assert!(rows.is_empty(), "{rows:?}");
    let _ = std::fs::remove_dir_all(h);
}

#[test]
fn the_walk_is_three_levels_deep() {
    let h = home("depth");
    std::fs::create_dir_all(h.join("Documents/a/b/c")).expect("dirs");
    std::fs::write(h.join("Documents/a/b/orion-three.md"), "x").expect("write");
    std::fs::write(h.join("Documents/a/b/c/orion-four.md"), "x").expect("write");
    let (rows, _) = files::section(&FileRoots::in_home(&h), &words(&["orion"]), SystemTime::now());
    assert_eq!(rows.len(), 1);
    assert!(rows[0].text.starts_with("orion-three.md"));
    let _ = std::fs::remove_dir_all(h);
}

#[test]
fn the_walk_stops_at_five_thousand_entries() {
    let h = home("cap");
    for i in 0..6_000 {
        std::fs::write(h.join(format!("Documents/file-{i:05}.txt")), "").expect("write");
    }
    let (_, walked) = files::section(&FileRoots::in_home(&h), &words(&["file"]), SystemTime::now());
    assert_eq!(walked.entries, MAX_ENTRIES, "6,000 files, and the walk looked at {}", walked.entries);
    let _ = std::fs::remove_dir_all(h);
}

#[cfg(unix)]
#[test]
fn a_link_is_not_followed_out_of_the_folder() {
    let h = home("link");
    let outside = std::env::temp_dir().join(format!("meeting-brief-outside-{}", std::process::id()));
    std::fs::create_dir_all(&outside).expect("dir");
    std::fs::write(outside.join("orion-secret.md"), "x").expect("write");
    std::os::unix::fs::symlink(&outside, h.join("Documents/linked")).expect("link");
    let (rows, _) = files::section(&FileRoots::in_home(&h), &words(&["orion"]), SystemTime::now());
    assert!(rows.is_empty());
    let _ = std::fs::remove_dir_all(h);
    let _ = std::fs::remove_dir_all(outside);
}

#[test]
fn a_title_with_no_meaningful_word_looks_at_no_files() {
    let h = home("none");
    let (rows, walked) = files::section(&FileRoots::in_home(&h), &[], SystemTime::now());
    assert!(rows.is_empty() && walked.entries == 0);
    let _ = std::fs::remove_dir_all(h);
}
