//! Private things stay private.

use super::store::find;
use super::test_support::*;
use super::{describe, gather};

#[test]
fn a_conflict_touching_a_minds_own_namespace_is_never_gathered() {
    let c = mem();
    make_memory_and_conflicts(&c);
    add_memory(&c, "m1", "shared fact", "default");
    add_memory(&c, "m2", "a secret in the mind's own namespace", "mind:hermes");
    add_memory(&c, "m3", "another shared fact", "default");
    add_conflict(&c, "private", "m1", "m2");
    add_conflict(&c, "both-private", "m2", "m2");
    add_conflict(&c, "shared", "m1", "m3");
    gather(&c, T0).expect("gather");
    assert!(find(&c, "conflicts", "private").expect("q").is_none());
    assert!(find(&c, "conflicts", "both-private").expect("q").is_none());
    assert!(find(&c, "conflicts", "shared").expect("q").is_some());
}

#[test]
fn a_conflict_whose_memory_moves_to_a_private_namespace_leaves_the_agenda() {
    let c = mem();
    make_memory_and_conflicts(&c);
    add_memory(&c, "m1", "x", "default");
    add_memory(&c, "m2", "y", "default");
    add_conflict(&c, "c1", "m1", "m2");
    gather(&c, T0).expect("gather");
    set(&c, "UPDATE memories SET namespace = 'mind:hermes' WHERE rid = 'm2'", []);
    gather(&c, T0 + 1.0).expect("gather");
    assert_eq!(describe(&c, T0 + 2.0).expect("d")["count"], 0);
}

#[test]
fn conflicts_are_not_gathered_when_the_memory_table_is_missing() {
    // Without it the namespace cannot be checked, so nothing is assumed to be public.
    let c = mem();
    c.execute_batch(
        "CREATE TABLE conflicts (conflict_id TEXT PRIMARY KEY, conflict_type TEXT, priority TEXT,
         status TEXT, memory_a TEXT, memory_b TEXT);
         INSERT INTO conflicts VALUES ('c1', 'x', 'high', 'open', 'a', 'b');",
    )
    .expect("setup");
    gather(&c, T0).expect("gather");
    assert!(find(&c, "conflicts", "c1").expect("q").is_none());
}

#[test]
fn describe_never_carries_a_memorys_text_or_a_thread_label() {
    let c = mem();
    make_memory_and_conflicts(&c);
    make_threads(&c);
    make_promises(&c);
    add_memory(&c, "m1", "SECRET-MEMORY-TEXT-ONE", "default");
    add_memory(&c, "m2", "SECRET-MEMORY-TEXT-TWO", "default");
    add_conflict(&c, "c1", "m1", "m2");
    add_thread(&c, "SECRET-THREAD-LABEL", 0.0, 0.0);
    add_promise(&c, "send Sam the report", T0 + DAY);
    gather(&c, T0).expect("gather");
    let text = describe(&c, T0).expect("d").to_string();
    assert!(!text.contains("SECRET"));
    assert!(text.contains("send Sam the report"), "the person's own promise may show its title");
    // Nor is it anywhere in the table itself.
    let dump: String = c
        .query_row("SELECT group_concat(title, '|') FROM agenda", [], |r| r.get(0))
        .expect("dump");
    assert!(!dump.contains("SECRET"));
    // A conflict or thread entry carries no title key at all.
    let v = describe(&c, T0).expect("d");
    for e in v["top"].as_array().expect("top") {
        if e["kind"] != "promise" {
            assert!(e.get("title").is_none());
        }
    }
}

#[test]
fn describe_of_an_empty_agenda_is_zero_and_of_a_full_one_counts_by_kind_state_and_next_step() {
    let c = mem();
    super::store::migrate(&c).expect("m");
    let v = describe(&c, T0).expect("d");
    assert_eq!(v["count"], 0);
    assert!(v["top"].as_array().expect("top").is_empty());
    make_promises(&c);
    make_tasks(&c);
    add_promise(&c, "a", T0 + DAY);
    add_promise(&c, "b", 0.0);
    add_task(&c, "t1", "c", 2);
    gather(&c, T0).expect("gather");
    let v = describe(&c, T0).expect("d");
    assert_eq!(v["count"], 3);
    assert_eq!(v["by_kind"]["promise"], 2);
    assert_eq!(v["by_kind"]["task"], 1);
    assert_eq!(v["by_state"]["waiting_person"], 2);
    assert_eq!(v["by_next_step"]["model"], 1);
    assert_eq!(v["top"].as_array().expect("top").len(), 3);
    assert!(v["top"][0]["terms"]["deadline"].is_number());
}
