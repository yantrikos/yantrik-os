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
    assert_eq!(rows[0].open, Some(Open::Message { account: ME.into(), folder: "INBOX".into(), id: "1".into() }));
}

#[test]
fn the_section_drops_what_the_source_returned_past_thirty_days() {
    struct Old;
    impl EmailSource for Old {
        fn exchanged(&self, _: &[String], _: i64, _: usize) -> Result<Vec<Mail>, String> {
            Ok(vec![Mail {
                from_name: "Maya".into(),
                from_addr: "maya@work.test".into(),
                subject: "Ancient".into(),
                date_ts: (NOW - 31 * DAY) as f64,
                account: ME.into(),
                folder: "INBOX".into(),
                id: "9".into(),
            }])
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
        Ok(self.0.iter().filter(|(t, _)| t.to_lowercase().contains(&q)).map(|(t, m)| NoteHit { id: format!("{t}.md"), title: (*t).into(), modified_at: (*m).into() }).collect())
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
    assert_eq!(rows[0].open, Some(Open::Note { id: "Call with Maya Chen.md".into() }));
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
    insert_mail(&conn, 1, "maya@work.test", ME, "The deck", 2.0);
    WorldModel::insert_commitment(&conn, &commitment("MAYA CHEN", "Pat", "share the deck", CommitmentSource::Email { message_id: "<1@test>".into(), subject: "The deck".into() }));
    let mut done = commitment("Pat", "Ravi Rao", "old thing", CommitmentSource::Manual);
    done.status = CommitmentStatus::Completed;
    WorldModel::insert_commitment(&conn, &done);
    WorldModel::insert_commitment(&conn, &commitment("Pat", "Someone Else", "unrelated", CommitmentSource::Manual));

    let people = [person("Ravi Rao", "ravi@work.test"), person("Maya Chen", "maya@work.test")];
    let rows = promises::section(&WorldModelPromises { conn: &conn, me: vec!["pat".into(), "user".into()] }, &people).expect("rows");
    let text: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text, ["You told Ravi: send the Q4 numbers", "Maya said: share the deck"]);
    assert_eq!(rows[0].open, None, "a promise made by hand has nothing to open");
    assert_eq!(
        rows[1].open,
        Some(Open::Message { account: ME.into(), folder: "INBOX".into(), id: "1".into() }),
        "the message the promise was found in, by where the cache holds it"
    );
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

struct NoMail;

impl EmailSource for NoMail {
    fn exchanged(&self, _: &[String], _: i64, _: usize) -> Result<Vec<Mail>, String> {
        Ok(Vec::new())
    }
}

struct NoPromises;

impl PromiseSource for NoPromises {
    fn made_by(&self, _: &str) -> Result<Vec<Promise>, String> {
        Ok(Vec::new())
    }
    fn made_to(&self, _: &str) -> Result<Vec<Promise>, String> {
        Ok(Vec::new())
    }
}

#[test]
fn gathering_one_meeting_in_a_crowded_home_folder_takes_under_200_ms() {
    use super::assemble::{gather, Sources};
    use super::window::Meeting;
    let h = home("budget");
    for i in 0..6_000 {
        std::fs::write(h.join(format!("Downloads/img-{i:05}.png")), "").expect("write");
    }
    let roots = FileRoots::in_home(&h);
    let book = Notebook(vec![]);
    let sources = Sources { emails: &NoMail, notes: &book, promises: &NoPromises, files: &roots };
    let meeting = Meeting {
        event_id: "e1".into(),
        title: "Orion launch".into(),
        start: NOW + 600,
        end: NOW + 2400,
        when: "10:00\u{2013}10:30".into(),
        place: None,
        description: String::new(),
        people: vec![person("Maya Chen", "maya@work.test"), person("Ravi Rao", "ravi@work.test")],
    };
    let started = std::time::Instant::now();
    let gathered = gather(&meeting, &sources, &own(), NOW);
    let took = started.elapsed();
    assert!(took < Duration::from_millis(200), "gathering took {took:?}");
    assert!(gathered.complete);
    let _ = std::fs::remove_dir_all(h);
}

// ── Whole addresses, exact messages, the person's own promises ──────────────────────────

fn in_folder(conn: &rusqlite::Connection, uid: u32, folder: &str, from: &str, to: &str, subject: &str, days_ago: f64) {
    let account = crate::email::db::ensure_account(conn, "Home", ME, "imap");
    crate::email::db::upsert_email(conn, account, uid, folder, from, "", to, subject, NOW as f64 - days_ago * DAY as f64, "", true, &format!("<{uid}@{folder}>"), "");
}

#[test]
fn an_address_matches_whole_and_never_as_part_of_another() {
    let conn = db();
    insert_mail(&conn, 1, "sal@x.test", ME, "From Sal", 1.0);
    insert_mail(&conn, 2, "val@x.test.au", ME, "From Val", 1.0);
    insert_mail(&conn, 3, "al@x.test.evil", ME, "From the lookalike", 1.0);
    insert_mail(&conn, 4, ME, "Sal <sal@x.test>, bob@x.test", "To Sal", 1.0);
    insert_mail(&conn, 5, "AL@X.TEST", ME, "From Al", 2.0);
    insert_mail(&conn, 6, ME, "Bob <bob@x.test>; Al Ford <Al@x.test>", "To Al among others", 3.0);
    let found = crate::email::db::emails_with_participants(&conn, &["al@x.test".to_string()], 0.0, 10);
    let subjects: Vec<&str> = found.iter().map(|m| m.subject.as_str()).collect();
    assert_eq!(subjects, ["From Al", "To Al among others"], "only Al, in any case and in any header form");
}

#[test]
fn a_row_opens_the_exact_message_not_the_one_that_shares_its_subject() {
    let conn = db();
    // The decoy is in the inbox with the same subject; the message that was meant is one the
    // person sent, which lives in Sent.
    in_folder(&conn, 7, "INBOX", "maya@work.test", ME, "Q4 numbers", 1.0);
    in_folder(&conn, 9, "Sent", ME, "maya@work.test", "Q4 numbers", 2.0);
    let rows = emails::section(&EmailCache { conn: &conn }, &[person("Maya Chen", "maya@work.test")], &own(), NOW).expect("rows");
    let opens: Vec<Option<Open>> = rows.iter().map(|r| r.open.clone()).collect();
    assert_eq!(
        opens,
        [
            Some(Open::Message { account: ME.into(), folder: "INBOX".into(), id: "7".into() }),
            Some(Open::Message { account: ME.into(), folder: "Sent".into(), id: "9".into() }),
        ],
        "each row names its own folder and id"
    );
}

#[test]
fn two_notes_with_one_title_are_two_rows_each_opened_by_its_own_id() {
    struct Twins;
    impl NoteSource for Twins {
        fn search(&self, _: &str) -> Result<Vec<NoteHit>, String> {
            Ok(vec![
                NoteHit { id: "note-a.md".into(), title: "Orion".into(), modified_at: "2026-10-01".into() },
                NoteHit { id: "note-b.md".into(), title: "Orion".into(), modified_at: "2026-10-02".into() },
            ])
        }
    }
    let rows = notes::section(&Twins, "Orion", &[]).expect("rows");
    let opens: Vec<Option<Open>> = rows.iter().map(|r| r.open.clone()).collect();
    assert_eq!(opens, [Some(Open::Note { id: "note-b.md".into() }), Some(Open::Note { id: "note-a.md".into() })]);
}

#[test]
fn a_promise_between_two_other_people_is_not_the_persons() {
    let conn = db();
    WorldModel::insert_commitment(&conn, &commitment("Maya Chen", "Ravi Rao", "review the budget", CommitmentSource::Manual));
    WorldModel::insert_commitment(&conn, &commitment("user", "Ravi Rao", "send the Q4 numbers", CommitmentSource::Manual));
    WorldModel::insert_commitment(&conn, &commitment("Maya Chen", "pat", "share the deck", CommitmentSource::Manual));
    let people = [person("Ravi Rao", "ravi@work.test"), person("Maya Chen", "maya@work.test")];
    let source = WorldModelPromises { conn: &conn, me: vec!["pat".into(), "user".into()] };
    let rows = promises::section(&source, &people).expect("rows");
    let text: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(text, ["You told Ravi: send the Q4 numbers", "Maya said: share the deck"], "Maya's promise to Ravi is neither line");
}

#[test]
fn a_walk_past_its_deadline_stops_where_it_is() {
    let h = home("deadline");
    std::fs::create_dir_all(h.join("Documents")).expect("dir");
    for i in 0..200 {
        std::fs::write(h.join("Documents").join(format!("file-{i}.txt")), "x").expect("file");
    }
    let (_, walked) = files::section_by(&FileRoots::in_home(&h), &words(&["file"]), SystemTime::now(), std::time::Instant::now());
    assert_eq!(walked.entries, 0, "a folder that stalls cannot keep the walk going");
    let (_, all) = files::section_by(&FileRoots::in_home(&h), &words(&["file"]), SystemTime::now(), std::time::Instant::now() + Duration::from_secs(30));
    assert_eq!(all.entries, 200);
    let _ = std::fs::remove_dir_all(h);
}
