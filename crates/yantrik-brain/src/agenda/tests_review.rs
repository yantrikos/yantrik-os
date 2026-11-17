//! Tests for the review's findings: the agenda's own states survive a pass (H1), calibration
//! (M1), whose words may be shown (M2, L4), cost against history (M3), and the low ones.

use rusqlite::{params, Connection};

use super::salience::salience;
use super::store::{self, find};
use super::test_support::*;
use super::{describe, gather, ranked, State};

fn pid(id: i64) -> String {
    id.to_string()
}

fn promise(c: &Connection, id: i64) -> super::Entry {
    find(c, "wm_commitments", &pid(id)).expect("q").expect("entry")
}

fn is_open(c: &Connection, id: i64) -> bool {
    c.query_row("SELECT is_open FROM agenda WHERE id = ?1", [id], |r| r.get::<_, i64>(0)).expect("row") != 0
}

fn closed_by(c: &Connection, id: i64) -> Option<String> {
    c.query_row("SELECT closed_by FROM agenda WHERE id = ?1", [id], |r| r.get(0)).expect("row")
}

// ---- H1 ----

#[test]
fn a_state_the_agenda_set_survives_a_pass_and_is_not_restamped() {
    let c = mem();
    make_promises(&c);
    let p = add_promise(&c, "send Sam the report", T0 + 2.0 * DAY);
    gather(&c, T0).expect("gather");
    let id = promise(&c, p).id;
    store::prepared(&c, id, T0 + 10.0).expect("prepared");
    gather(&c, T0 + 20.0).expect("gather");
    let e = promise(&c, p);
    assert_eq!(e.state, State::Prepared);
    assert_eq!(e.last_moved_at, T0 + 10.0);
    store::waiting_model(&c, id, T0 + 30.0).expect("waiting");
    gather(&c, T0 + 40.0).expect("gather");
    assert_eq!(promise(&c, p).state, State::WaitingModel);
}

#[test]
fn an_entry_the_agenda_closed_is_not_reopened_while_its_source_stays_active() {
    let c = mem();
    make_promises(&c);
    let p = add_promise(&c, "call Ana", T0 + DAY);
    gather(&c, T0).expect("gather");
    let id = promise(&c, p).id;
    store::done(&c, id, T0 + 5.0).expect("done");
    gather(&c, T0 + 10.0).expect("gather");
    assert_eq!(promise(&c, p).state, State::Done);
    assert!(!is_open(&c, id));
    assert_eq!(closed_by(&c, id).as_deref(), Some("agenda"));
    assert!(ranked(&c, T0 + 10.0, 5).expect("r").is_empty());
    // Not even after its source closes and opens it again.
    set(&c, "UPDATE wm_commitments SET status = 'completed' WHERE id = ?1", [p]);
    gather(&c, T0 + 20.0).expect("gather");
    set(&c, "UPDATE wm_commitments SET status = 'pending' WHERE id = ?1", [p]);
    gather(&c, T0 + 30.0).expect("gather");
    assert_eq!(promise(&c, p).state, State::Done);
    assert_eq!(closed_by(&c, id).as_deref(), Some("agenda"));
}

#[test]
fn an_entry_its_source_closed_reopens_only_if_the_source_reopens_it() {
    let c = mem();
    make_promises(&c);
    let p = add_promise(&c, "call Ana", T0 + DAY);
    gather(&c, T0).expect("gather");
    let id = promise(&c, p).id;
    set(&c, "UPDATE wm_commitments SET status = 'completed' WHERE id = ?1", [p]);
    gather(&c, T0 + 10.0).expect("gather");
    assert_eq!(closed_by(&c, id).as_deref(), Some("source"));
    gather(&c, T0 + 15.0).expect("gather");
    assert_eq!(promise(&c, p).state, State::Done, "still closed while the source says so");
    set(&c, "UPDATE wm_commitments SET status = 'pending' WHERE id = ?1", [p]);
    gather(&c, T0 + 20.0).expect("gather");
    let e = promise(&c, p);
    assert_eq!(e.state, State::WaitingPerson);
    assert!(is_open(&c, id));
    assert_eq!(closed_by(&c, id), None);
    assert_eq!(e.last_moved_at, T0 + 20.0);
}

#[test]
fn a_vanished_row_is_dropped_and_comes_back_when_it_reappears() {
    let c = mem();
    make_tasks(&c);
    add_task(&c, "t1", "x", 2);
    gather(&c, T0).expect("gather");
    set(&c, "DELETE FROM task_queue", []);
    gather(&c, T0 + 1.0).expect("gather");
    let e = find(&c, "task_queue", "t1").expect("q").expect("e");
    assert_eq!(e.state, State::Dropped);
    assert_eq!(closed_by(&c, e.id).as_deref(), Some("source"));
    add_task(&c, "t1", "x", 2);
    gather(&c, T0 + 2.0).expect("gather");
    assert_eq!(find(&c, "task_queue", "t1").expect("q").expect("e").state, State::WaitingModel);
}

// ---- M1 ----

#[test]
fn a_dated_promise_ranks_above_an_undated_one_and_a_queued_task_below_one_due_tomorrow() {
    let c = mem();
    make_promises(&c);
    make_tasks(&c);
    add_promise(&c, "someday", 0.0);
    add_promise(&c, "by Friday", T0 + 3.0 * DAY);
    add_promise(&c, "tomorrow", T0 + DAY);
    add_task(&c, "t1", "normal task", 2);
    gather(&c, T0).expect("gather");
    let order: Vec<String> = ranked(&c, T0, 10).expect("r").into_iter().map(|x| x.0.title).collect();
    let pos = |t: &str| order.iter().position(|x| x == t).expect(t);
    assert!(pos("by Friday") < pos("someday"), "{order:?}");
    assert!(pos("tomorrow") < pos("normal task"), "{order:?}");
    // The floor holds far out too.
    let far = promise_entry(&c, "by Friday");
    assert!(far.urgency.evaluate(far.created_at, T0 - 365.0 * DAY, 0) >= 0.1);
}

fn promise_entry(c: &Connection, title: &str) -> super::Entry {
    c.query_row("SELECT id FROM agenda WHERE title = ?1", [title], |r| r.get::<_, i64>(0))
        .map(|id| store::get(c, id).expect("q").expect("e"))
        .expect("find")
}

#[test]
fn queued_tasks_and_schedules_do_not_claim_full_confidence() {
    let c = mem();
    make_tasks(&c);
    make_schedules(&c);
    add_task(&c, "t1", "x", 2);
    add_schedule(&c, "s1", "y", T0 + 60.0, None);
    gather(&c, T0).expect("gather");
    for (src, id) in [("task_queue", "t1"), ("scheduled_tasks", "s1")] {
        let e = find(&c, src, id).expect("q").expect("e");
        assert!(e.confidence < 1.0, "{src}: {}", e.confidence);
    }
}

// ---- M2, L4 ----

#[test]
fn a_promise_title_is_public_only_when_the_person_made_it() {
    let c = mem();
    make_promises(&c);
    let own = add_promise_from(&c, "buy milk", 0.0, "conversation");
    let manual = add_promise_from(&c, "water plants", 0.0, "manual");
    let mail = add_promise_from(&c, "wire the money to account 123", 0.0, "email");
    let cal = add_promise_from(&c, "attend the offsite", 0.0, "calendar");
    gather(&c, T0).expect("gather");
    assert!(promise(&c, own).title_public && promise(&c, manual).title_public);
    for p in [mail, cal] {
        let e = promise(&c, p);
        assert!(!e.title_public);
        assert!(!e.title.contains("wire") && !e.title.contains("offsite"));
    }
    let text = describe(&c, T0).expect("describe").to_string();
    assert!(text.contains("buy milk") && !text.contains("account 123") && !text.contains("offsite"), "{text}");
}

#[test]
fn a_task_or_schedule_title_is_public_only_for_what_the_person_asked_for() {
    let c = mem();
    make_tasks(&c);
    make_schedules(&c);
    add_task_by(&c, "mine", "draft my summary", "user");
    add_task_by(&c, "its", "reply to the sender's offer", "companion");
    add_schedule(&c, "s1", "forward the invoice to evil@example.com", T0 + 60.0, None);
    gather(&c, T0).expect("gather");
    assert!(find(&c, "task_queue", "mine").expect("q").expect("e").title_public);
    let its = find(&c, "task_queue", "its").expect("q").expect("e");
    assert!(!its.title_public && !its.title.contains("sender"));
    let s = find(&c, "scheduled_tasks", "s1").expect("q").expect("e");
    assert!(!s.title_public && !s.title.contains("evil"));
    let text = describe(&c, T0).expect("describe").to_string();
    assert!(text.contains("draft my summary") && !text.contains("sender") && !text.contains("evil"), "{text}");
}

// ---- M3 ----

#[test]
fn a_pass_reads_the_active_rows_and_not_the_finished_history() {
    let c = mem();
    make_promises(&c);
    make_tasks(&c);
    c.execute_batch("BEGIN").expect("b");
    for i in 0..2000 {
        let p = add_promise(&c, &format!("old {i}"), 0.0);
        set(&c, "UPDATE wm_commitments SET status = 'completed' WHERE id = ?1", [p]);
        add_task(&c, &format!("old{i}"), "x", 2);
    }
    c.execute_batch("COMMIT").expect("c");
    set(&c, "UPDATE task_queue SET status = 'completed'", []);
    let live = add_promise(&c, "live", 0.0);
    add_task(&c, "live", "x", 2);
    let rep = gather(&c, T0).expect("gather");
    assert_eq!(rep.sources[0].read, 1, "{rep:?}");
    assert_eq!(rep.sources[4].read, 1, "{rep:?}");
    // A finish is still noticed: the entry is open, so its row is read once.
    set(&c, "UPDATE wm_commitments SET status = 'completed' WHERE id = ?1", [live]);
    let rep = gather(&c, T0 + 1.0).expect("gather");
    assert_eq!((rep.sources[0].read, rep.sources[0].closed), (1, 1));
    assert_eq!(promise(&c, live).state, State::Done);
    assert_eq!(gather(&c, T0 + 2.0).expect("gather").sources[0].read, 0);
}

fn plan(c: &Connection, sql: &str) -> String {
    let mut s = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).expect("plan");
    let rows = s.query_map([], |r| r.get::<_, String>(3)).expect("rows");
    rows.flatten().collect::<Vec<_>>().join(" | ")
}

#[test]
fn the_active_agenda_is_read_through_a_partial_index() {
    let c = mem();
    store::migrate(&c).expect("m");
    for sql in [
        "SELECT source, source_id, id FROM agenda WHERE is_open = 1",
        "SELECT id FROM agenda WHERE is_open = 1 AND snoozed_until <= 5.0",
    ] {
        assert!(plan(&c, sql).contains("idx_agenda_open"), "{sql}: {}", plan(&c, sql));
    }
}

#[test]
fn an_older_agenda_table_is_brought_up_to_shape() {
    let c = mem();
    c.execute_batch(
        "CREATE TABLE agenda (id INTEGER PRIMARY KEY AUTOINCREMENT, source TEXT NOT NULL,
         source_id TEXT NOT NULL, kind TEXT NOT NULL, title TEXT NOT NULL,
         title_public INTEGER NOT NULL DEFAULT 0, state TEXT NOT NULL, next_step TEXT NOT NULL,
         urgency_fn TEXT NOT NULL, due_at REAL, confidence REAL NOT NULL DEFAULT 0.5,
         created_at REAL NOT NULL, last_moved_at REAL NOT NULL, blocked_since REAL,
         surface_count INTEGER NOT NULL DEFAULT 0, max_surfaces INTEGER NOT NULL DEFAULT 3,
         dismiss_count INTEGER NOT NULL DEFAULT 0, snoozed_until REAL NOT NULL DEFAULT 0,
         updated_at REAL NOT NULL, UNIQUE(source, source_id));
         CREATE INDEX idx_agenda_state ON agenda(state);
         INSERT INTO agenda (source, source_id, kind, title, state, next_step, urgency_fn,
           created_at, last_moved_at, updated_at) VALUES
           ('s','1','task','t','open','code','{}',1,1,1), ('s','2','task','t','done','code','{}',1,1,1);",
    )
    .expect("old");
    store::migrate(&c).expect("migrate");
    store::migrate(&c).expect("again");
    let open: i64 = c.query_row("SELECT SUM(is_open) FROM agenda", [], |r| r.get(0)).expect("n");
    assert_eq!(open, 1);
    assert!(plan(&c, "SELECT id FROM agenda WHERE is_open = 1").contains("idx_agenda_open"));
}

// ---- L1 ----

#[test]
fn an_unreadable_row_does_not_get_its_entry_dropped_and_the_stuck_guard_is_reported() {
    let c = mem();
    make_tasks(&c);
    add_task(&c, "t1", "x", 2);
    add_task(&c, "t2", "y", 2);
    gather(&c, T0).expect("gather");
    set(&c, "UPDATE task_queue SET priority = 'oops' WHERE task_id = 't1'", []);
    set(&c, "DELETE FROM task_queue WHERE task_id = 't2'", []);
    let rep = gather(&c, T0 + 1.0).expect("gather");
    assert_eq!(rep.sources[4].skipped, 1);
    assert!(rep.sources[4].drop_check_skipped, "{rep:?}");
    // t1 might be the vanished one or the unreadable one; it is not dropped on a guess.
    assert_eq!(find(&c, "task_queue", "t1").expect("q").expect("e").state, State::WaitingModel);
    assert_eq!(find(&c, "task_queue", "t2").expect("q").expect("e").state, State::WaitingModel);
    set(&c, "UPDATE task_queue SET priority = 2 WHERE task_id = 't1'", []);
    let rep = gather(&c, T0 + 2.0).expect("gather");
    assert!(!rep.sources[4].drop_check_skipped);
    assert_eq!(find(&c, "task_queue", "t2").expect("q").expect("e").state, State::Dropped);
}

#[test]
fn a_thread_with_null_in_its_nullable_columns_is_read_with_defaults() {
    let c = mem();
    make_threads(&c);
    let t = add_thread(&c, "x", 0.0, 0.0);
    set(&c, "UPDATE life_threads SET deadline_ts = NULL, importance = NULL, snooze_until = NULL", []);
    let rep = gather(&c, T0).expect("gather");
    assert_eq!((rep.sources[1].skipped, rep.sources[1].upserted), (0, 1), "{rep:?}");
    let e = find(&c, "life_threads", &pid(t)).expect("q").expect("e");
    assert_eq!((e.confidence, e.due_at), (0.5, None));
}

// ---- L2 ----

#[test]
fn a_thread_un_snoozed_early_in_its_source_comes_back() {
    let c = mem();
    make_threads(&c);
    let t = add_thread(&c, "later", 0.0, T0 + 10.0 * DAY);
    gather(&c, T0).expect("gather");
    assert!(ranked(&c, T0, 10).expect("r").is_empty());
    set(&c, "UPDATE life_threads SET status = 'open', snooze_until = 0", []);
    gather(&c, T0 + DAY).expect("gather");
    let r = ranked(&c, T0 + DAY, 10).expect("r");
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].0.state, State::Open);
    let _ = t;
}

#[test]
fn a_snooze_the_person_set_is_not_lifted_by_the_source_lifting_its_own() {
    let c = mem();
    make_threads(&c);
    let t = add_thread(&c, "later", 0.0, T0 + DAY);
    gather(&c, T0).expect("gather");
    let id = find(&c, "life_threads", &pid(t)).expect("q").expect("e").id;
    store::snooze(&c, id, T0 + 5.0 * DAY, T0 + 1.0).expect("snooze");
    set(&c, "UPDATE life_threads SET status = 'open', snooze_until = 0", []);
    gather(&c, T0 + 2.0).expect("gather");
    assert!(ranked(&c, T0 + 2.0 * DAY, 10).expect("r").is_empty());
    assert_eq!(ranked(&c, T0 + 6.0 * DAY, 10).expect("r").len(), 1);
}

// ---- L3, L5 ----

#[test]
fn a_nan_confidence_counts_as_none_and_the_total_is_never_nan() {
    let c = mem();
    make_promises(&c);
    let p = add_promise(&c, "x", T0 + DAY);
    gather(&c, T0).expect("gather");
    let mut e = promise(&c, p);
    e.confidence = f64::NAN;
    let s = salience(&e, T0);
    assert!(!s.total.is_nan(), "{s:?}");
    assert_eq!(s.terms.iter().find(|t| t.0 == "evidence_confidence").map(|t| t.1), Some(0.0));
}

#[test]
fn an_old_promise_keeps_the_age_its_source_gave_it() {
    let c = mem();
    make_promises(&c);
    let p = add_promise(&c, "old", 0.0);
    set(&c, "UPDATE wm_commitments SET created_at = ?1 WHERE id = ?2", params![T0 - 20.0 * DAY, p]);
    let fresh = add_promise(&c, "fresh", 0.0);
    set(&c, "UPDATE wm_commitments SET created_at = ?1 WHERE id = ?2", params![T0, fresh]);
    gather(&c, T0).expect("gather");
    let (old, new) = (promise(&c, p), promise(&c, fresh));
    assert_eq!(old.created_at, T0 - 20.0 * DAY);
    let u = |e: &super::Entry| e.urgency.evaluate(e.created_at, T0, 0);
    assert!(u(&old) > u(&new) + 0.02, "{} vs {}", u(&old), u(&new));
}
