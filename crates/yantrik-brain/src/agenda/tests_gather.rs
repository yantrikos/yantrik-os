//! Each mirror: appears, updates in place, closes, and leaves no duplicates after a second
//! pass or a restart (a fresh `gather` call on the same database file).

use super::store::{self, find};
use super::test_support::*;
use super::{gather, State};

fn count(c: &rusqlite::Connection) -> i64 {
    c.query_row("SELECT COUNT(*) FROM agenda", [], |r| r.get(0)).expect("count")
}

#[test]
fn promise_appears_as_waiting_on_the_person_with_its_deadline() {
    let c = mem();
    make_promises(&c);
    let id = add_promise(&c, "send Sam the report", T0 + 2.0 * DAY);
    gather(&c, T0).expect("gather");
    let e = find(&c, "wm_commitments", &id.to_string()).expect("q").expect("entry");
    assert_eq!(e.state, State::WaitingPerson);
    assert_eq!(e.next_step.as_str(), "person");
    assert_eq!(e.kind.as_str(), "promise");
    assert_eq!(e.due_at, Some(T0 + 2.0 * DAY));
    assert_eq!(e.title, "send Sam the report");
}

#[test]
fn promise_updates_in_place_and_closes_when_its_source_says_so() {
    let c = mem();
    make_promises(&c);
    let id = add_promise(&c, "call Ana", T0 + DAY);
    gather(&c, T0).expect("gather");
    set(&c, "UPDATE wm_commitments SET deadline = ?1 WHERE id = ?2", rusqlite::params![T0 + 5.0 * DAY, id]);
    gather(&c, T0 + 10.0).expect("gather");
    let e = find(&c, "wm_commitments", &id.to_string()).expect("q").expect("entry");
    assert_eq!(e.due_at, Some(T0 + 5.0 * DAY));
    assert_eq!(count(&c), 1);
    set(&c, "UPDATE wm_commitments SET status = 'completed' WHERE id = ?1", [id]);
    let rep = gather(&c, T0 + 20.0).expect("gather");
    let e = find(&c, "wm_commitments", &id.to_string()).expect("q").expect("entry");
    assert_eq!(e.state, State::Done);
    assert_eq!(e.last_moved_at, T0 + 20.0);
    assert_eq!(rep.sources[0].closed, 1);
    set(&c, "UPDATE wm_commitments SET status = 'cancelled' WHERE id = ?1", [id]);
    gather(&c, T0 + 30.0).expect("gather");
    // Already closed: a later pass does not re-close or re-stamp it.
    let e2 = find(&c, "wm_commitments", &id.to_string()).expect("q").expect("entry");
    assert_eq!(e2.state, State::Done);
}

#[test]
fn promise_with_no_deadline_still_appears() {
    let c = mem();
    make_promises(&c);
    let id = add_promise(&c, "think about it", 0.0);
    gather(&c, T0).expect("gather");
    let e = find(&c, "wm_commitments", &id.to_string()).expect("q").expect("entry");
    assert_eq!(e.due_at, None);
}

#[test]
fn thread_appears_updates_closes_and_never_copies_its_label() {
    let c = mem();
    make_threads(&c);
    let id = add_thread(&c, "reply to the landlord about the deposit", 0.0, 0.0);
    gather(&c, T0).expect("gather");
    let e = find(&c, "life_threads", &id.to_string()).expect("q").expect("entry");
    assert_eq!(e.state, State::Open);
    assert!(!e.title.contains("landlord"));
    assert!(!e.title_public);
    set(&c, "UPDATE life_threads SET deadline_ts = ?1 WHERE id = ?2", rusqlite::params![T0 + DAY, id]);
    gather(&c, T0 + 1.0).expect("gather");
    assert_eq!(find(&c, "life_threads", &id.to_string()).expect("q").expect("e").due_at, Some(T0 + DAY));
    set(&c, "UPDATE life_threads SET status = 'resolved' WHERE id = ?1", [id]);
    gather(&c, T0 + 2.0).expect("gather");
    assert_eq!(find(&c, "life_threads", &id.to_string()).expect("q").expect("e").state, State::Done);
    assert_eq!(count(&c), 1);
}

#[test]
fn snoozed_thread_is_held_out_of_the_ranking_until_its_snooze_ends() {
    let c = mem();
    make_threads(&c);
    add_thread(&c, "later", 0.0, T0 + DAY);
    gather(&c, T0).expect("gather");
    assert!(super::ranked(&c, T0, 10).expect("ranked").is_empty());
    assert_eq!(super::ranked(&c, T0 + 2.0 * DAY, 10).expect("ranked").len(), 1);
}

#[test]
fn conflict_appears_as_waiting_on_the_person_updates_and_closes() {
    let c = mem();
    make_memory_and_conflicts(&c);
    add_memory(&c, "m1", "likes tea", "default");
    add_memory(&c, "m2", "hates tea", "default");
    add_conflict(&c, "c1", "m1", "m2");
    gather(&c, T0).expect("gather");
    let e = find(&c, "conflicts", "c1").expect("q").expect("entry");
    assert_eq!((e.state, e.next_step.as_str()), (State::WaitingPerson, "person"));
    set(&c, "UPDATE conflicts SET priority = 'critical' WHERE conflict_id = 'c1'", []);
    gather(&c, T0 + 1.0).expect("gather");
    let e = find(&c, "conflicts", "c1").expect("q").expect("entry");
    assert_eq!(e.urgency, super::UrgencyFn::Constant { value: 0.9 });
    set(&c, "UPDATE conflicts SET status = 'resolved' WHERE conflict_id = 'c1'", []);
    gather(&c, T0 + 2.0).expect("gather");
    assert_eq!(find(&c, "conflicts", "c1").expect("q").expect("e").state, State::Done);
    assert_eq!(count(&c), 1);
}

#[test]
fn schedule_appears_updates_and_closes_with_code_or_model_next_step() {
    let c = mem();
    make_schedules(&c);
    add_schedule(&c, "s1", "water the plants", T0 + 120.0, None);
    add_schedule(&c, "s2", "summarise mail", T0 + 120.0, Some("{\"tool\":\"x\"}"));
    gather(&c, T0).expect("gather");
    assert_eq!(find(&c, "scheduled_tasks", "s1").expect("q").expect("e").next_step.as_str(), "code");
    assert_eq!(find(&c, "scheduled_tasks", "s2").expect("q").expect("e").next_step.as_str(), "model");
    set(&c, "UPDATE scheduled_tasks SET next_invoke = ?1 WHERE task_id = 's1'", [T0 + 600.0]);
    gather(&c, T0 + 1.0).expect("gather");
    assert_eq!(find(&c, "scheduled_tasks", "s1").expect("q").expect("e").due_at, Some(T0 + 600.0));
    set(&c, "UPDATE scheduled_tasks SET status = 'completed' WHERE task_id = 's1'", []);
    gather(&c, T0 + 700.0).expect("gather");
    assert_eq!(find(&c, "scheduled_tasks", "s1").expect("q").expect("e").state, State::Done);
    assert_eq!(count(&c), 2);
}

#[test]
fn queued_task_is_a_model_step_and_closes_when_finished() {
    let c = mem();
    make_tasks(&c);
    add_task(&c, "t1", "draft the summary", 3);
    gather(&c, T0).expect("gather");
    let e = find(&c, "task_queue", "t1").expect("q").expect("e");
    assert_eq!((e.state, e.next_step.as_str()), (State::WaitingModel, "model"));
    set(&c, "UPDATE task_queue SET status = 'in_progress' WHERE task_id = 't1'", []);
    gather(&c, T0 + 1.0).expect("gather");
    assert_eq!(find(&c, "task_queue", "t1").expect("q").expect("e").state, State::WaitingModel);
    set(&c, "UPDATE task_queue SET status = 'failed' WHERE task_id = 't1'", []);
    gather(&c, T0 + 2.0).expect("gather");
    assert_eq!(find(&c, "task_queue", "t1").expect("q").expect("e").state, State::Dropped);
}

#[test]
fn a_row_deleted_from_its_source_is_dropped() {
    let c = mem();
    make_tasks(&c);
    add_task(&c, "t1", "x", 2);
    gather(&c, T0).expect("gather");
    set(&c, "DELETE FROM task_queue", []);
    gather(&c, T0 + 1.0).expect("gather");
    assert_eq!(find(&c, "task_queue", "t1").expect("q").expect("e").state, State::Dropped);
}

#[test]
fn second_pass_adds_no_duplicates_and_changes_nothing() {
    let c = mem();
    make_promises(&c);
    make_threads(&c);
    make_memory_and_conflicts(&c);
    make_schedules(&c);
    make_tasks(&c);
    add_promise(&c, "a", T0 + DAY);
    add_thread(&c, "b", 0.0, 0.0);
    add_memory(&c, "m1", "x", "default");
    add_memory(&c, "m2", "y", "default");
    add_conflict(&c, "c1", "m1", "m2");
    add_schedule(&c, "s1", "d", T0 + 60.0, None);
    add_task(&c, "t1", "e", 1);
    gather(&c, T0).expect("gather");
    assert_eq!(count(&c), 5);
    let before: String = c
        .query_row("SELECT group_concat(updated_at || last_moved_at, ',') FROM agenda", [], |r| r.get(0))
        .expect("snap");
    // "Restart": nothing is held in memory between passes, so a later pass is the same as a
    // fresh process opening the same file.
    store::migrate(&c).expect("migrate again");
    let rep = gather(&c, T0 + 90.0).expect("gather");
    assert_eq!(count(&c), 5);
    assert_eq!(rep.skipped(), 0);
    let after: String = c
        .query_row("SELECT group_concat(updated_at || last_moved_at, ',') FROM agenda", [], |r| r.get(0))
        .expect("snap");
    assert_eq!(before, after, "an unchanged source must not touch the agenda");
}

#[test]
fn restart_on_a_file_keeps_entries_without_duplicates() {
    let dir = std::env::temp_dir().join(format!("agenda-restart-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("m.db");
    {
        let c = rusqlite::Connection::open(&path).expect("open");
        make_promises(&c);
        add_promise(&c, "send Sam the report", T0 + DAY);
        gather(&c, T0).expect("gather");
    }
    let c = rusqlite::Connection::open(&path).expect("reopen");
    gather(&c, T0 + 60.0).expect("gather");
    assert_eq!(count(&c), 1);
    drop(c);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_source_tables_are_tolerated_and_reported() {
    let c = mem();
    let rep = gather(&c, T0).expect("gather on an empty database");
    assert!(rep.sources.iter().all(|s| !s.present && !s.failed));
    assert_eq!(count(&c), 0);
    // Only some exist.
    make_tasks(&c);
    add_task(&c, "t1", "x", 2);
    let rep = gather(&c, T0).expect("gather");
    assert_eq!(rep.sources.iter().filter(|s| s.present).count(), 1);
    assert_eq!(count(&c), 1);
}

#[test]
fn unreadable_rows_are_skipped_and_counted_and_never_panic() {
    let c = mem();
    // SQLite is loosely typed: a text where a number belongs is a row we cannot read.
    c.execute_batch(
        "CREATE TABLE task_queue (task_id TEXT, title TEXT, priority, status TEXT, created_by TEXT);
         INSERT INTO task_queue VALUES ('ok', 'fine', 2, 'pending', 'user');
         INSERT INTO task_queue VALUES ('bad', 'broken', 'not a number', 'pending', 'user');
         INSERT INTO task_queue VALUES (NULL, 'no id', 2, 'pending', 'user');",
    )
    .expect("setup");
    let rep = gather(&c, T0).expect("gather");
    let t = rep.sources.iter().find(|s| s.source == "task_queue").expect("task report");
    assert_eq!(t.skipped, 2);
    assert_eq!(t.upserted, 1);
    assert_eq!(count(&c), 1);
}

#[test]
fn a_source_with_a_different_shape_is_reported_failed_not_fatal() {
    let c = mem();
    c.execute_batch("CREATE TABLE task_queue (task_id TEXT);").expect("setup");
    let rep = gather(&c, T0).expect("gather");
    assert!(rep.sources.iter().find(|s| s.source == "task_queue").expect("r").failed);
}

#[test]
fn nothing_outside_the_agenda_table_is_written() {
    let c = mem();
    make_promises(&c);
    make_threads(&c);
    make_memory_and_conflicts(&c);
    make_schedules(&c);
    make_tasks(&c);
    add_promise(&c, "a", T0 + DAY);
    add_thread(&c, "b", T0 + 2.0 * DAY, 0.0);
    add_memory(&c, "m1", "x", "default");
    add_memory(&c, "m2", "y", "default");
    add_conflict(&c, "c1", "m1", "m2");
    add_schedule(&c, "s1", "d", T0 + 60.0, None);
    add_task(&c, "t1", "e", 1);
    let before = dump_all_but_agenda(&c);
    gather(&c, T0).expect("gather");
    let id = find(&c, "wm_commitments", "1").expect("q").expect("e").id;
    store::snooze(&c, id, T0 + 99.0, T0 + 6.0).expect("snooze");
    store::dismiss(&c, id, 60.0, T0 + 7.0).expect("dismiss");
    store::done(&c, id, T0 + 8.0).expect("done");
    gather(&c, T0 + 9.0).expect("gather again");
    let _ = super::describe(&c, T0 + 10.0).expect("describe");
    assert!(!before.is_empty());
    assert_eq!(before, dump_all_but_agenda(&c), "every other table must be byte-for-byte unchanged");
}
