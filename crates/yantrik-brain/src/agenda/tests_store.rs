//! The table: migration, upsert, state changes, snooze, dismiss, anti-nag.

use super::salience::UrgencyFn;
use super::store::{self, NewEntry};
use super::test_support::*;
use super::{Kind, NextStep, State};

type Step = fn(&rusqlite::Connection, i64, f64) -> rusqlite::Result<bool>;

fn entry(source_id: &str) -> NewEntry {
    NewEntry {
        source: "test",
        source_id: source_id.to_string(),
        kind: Kind::Task,
        title: "t".into(),
        title_public: true,
        state: State::Open,
        next_step: NextStep::Code,
        urgency: UrgencyFn::Constant { value: 0.5 },
        due_at: None,
        confidence: 0.5,
        hold_until: 0.0,
        created_at: None,
    }
}

#[test]
fn migration_is_idempotent_and_keeps_rows() {
    let c = mem();
    store::migrate(&c).expect("1");
    store::upsert(&c, &entry("a"), T0).expect("up");
    store::migrate(&c).expect("2");
    store::migrate(&c).expect("3");
    let n: i64 = c.query_row("SELECT COUNT(*) FROM agenda", [], |r| r.get(0)).expect("n");
    assert_eq!(n, 1);
}

#[test]
fn upsert_by_source_and_id_updates_in_place_and_keeps_counters() {
    let c = mem();
    store::migrate(&c).expect("m");
    store::upsert(&c, &entry("a"), T0).expect("up");
    let id = store::find(&c, "test", "a").expect("q").expect("e").id;
    store::mark_surfaced(&c, id, T0 + 1.0).expect("s");
    store::dismiss(&c, id, 10.0, T0 + 2.0).expect("d");
    let mut e = entry("a");
    e.title = "renamed".into();
    store::upsert(&c, &e, T0 + 100.0).expect("up2");
    let got = store::find(&c, "test", "a").expect("q").expect("e");
    assert_eq!(got.id, id);
    assert_eq!(got.title, "renamed");
    assert_eq!((got.surface_count, got.dismiss_count), (1, 1));
    assert_eq!(got.created_at, T0);
    // The state did not change, so "last moved" did not either.
    assert_eq!(got.last_moved_at, T0);
}

#[test]
fn state_changes_stamp_last_moved_and_a_same_state_change_is_a_no_op() {
    let c = mem();
    store::migrate(&c).expect("m");
    store::upsert(&c, &entry("a"), T0).expect("up");
    let id = store::find(&c, "test", "a").expect("q").expect("e").id;
    let steps: [(Step, State); 6] = [
        (store::prepared, State::Prepared),
        (store::waiting_person, State::WaitingPerson),
        (store::waiting_model, State::WaitingModel),
        (store::open, State::Open),
        (store::done, State::Done),
        (store::dropped, State::Dropped),
    ];
    for (i, (f, want)) in steps.iter().enumerate() {
        let now = T0 + 10.0 * (i as f64 + 1.0);
        assert!(f(&c, id, now).expect("change"));
        let e = store::get(&c, id).expect("q").expect("e");
        assert_eq!((e.state, e.last_moved_at), (*want, now));
        assert!(!f(&c, id, now + 1.0).expect("again"), "same state is a no-op");
        assert_eq!(store::get(&c, id).expect("q").expect("e").last_moved_at, now);
    }
}

#[test]
fn blocked_since_is_set_on_waiting_and_cleared_on_leaving() {
    let c = mem();
    store::migrate(&c).expect("m");
    store::upsert(&c, &entry("a"), T0).expect("up");
    let id = store::find(&c, "test", "a").expect("q").expect("e").id;
    store::waiting_person(&c, id, T0 + 5.0).expect("w");
    assert_eq!(store::get(&c, id).expect("q").expect("e").blocked_since, Some(T0 + 5.0));
    store::open(&c, id, T0 + 6.0).expect("o");
    assert_eq!(store::get(&c, id).expect("q").expect("e").blocked_since, None);
}

#[test]
fn snooze_hides_until_its_time_then_the_entry_returns() {
    let c = mem();
    store::migrate(&c).expect("m");
    store::upsert(&c, &entry("a"), T0).expect("up");
    let id = store::find(&c, "test", "a").expect("q").expect("e").id;
    assert!(store::snooze(&c, id, T0 + HOUR, T0).expect("s"));
    assert!(super::ranked(&c, T0 + 10.0, 5).expect("r").is_empty());
    assert_eq!(super::ranked(&c, T0 + HOUR, 5).expect("r").len(), 1);
    // The source still says active: the entry stays snoozed, then comes back open.
    store::upsert(&c, &entry("a"), T0 + 20.0).expect("up");
    assert_eq!(store::get(&c, id).expect("q").expect("e").state, State::Snoozed);
    store::upsert(&c, &entry("a"), T0 + 2.0 * HOUR).expect("up");
    assert_eq!(store::get(&c, id).expect("q").expect("e").state, State::Open);
}

#[test]
fn snoozing_a_closed_entry_does_nothing() {
    let c = mem();
    store::migrate(&c).expect("m");
    store::upsert(&c, &entry("a"), T0).expect("up");
    let id = store::find(&c, "test", "a").expect("q").expect("e").id;
    store::done(&c, id, T0 + 1.0).expect("d");
    assert!(!store::snooze(&c, id, T0 + HOUR, T0 + 2.0).expect("s"));
    assert_eq!(store::get(&c, id).expect("q").expect("e").state, State::Done);
}

#[test]
fn dismiss_counts_holds_and_makes_a_decaying_entry_less_urgent() {
    let c = mem();
    store::migrate(&c).expect("m");
    let mut e = entry("a");
    e.urgency = UrgencyFn::DecayIfIgnored { initial: 0.8, decay_factor: 0.5 };
    store::upsert(&c, &e, T0).expect("up");
    let id = store::find(&c, "test", "a").expect("q").expect("e").id;
    let before = super::ranked(&c, T0, 5).expect("r")[0].1.terms[0].1;
    store::dismiss(&c, id, 60.0, T0).expect("d");
    assert!(super::ranked(&c, T0 + 30.0, 5).expect("r").is_empty());
    let after = super::ranked(&c, T0 + 61.0, 5).expect("r")[0].1.terms[0].1;
    assert!((before - 2.4).abs() < 1e-9 && (after - 1.2).abs() < 1e-9, "{before} {after}");
    assert_eq!(store::get(&c, id).expect("q").expect("e").dismiss_count, 1);
}

#[test]
fn the_anti_nag_counter_runs_out() {
    let c = mem();
    store::migrate(&c).expect("m");
    store::upsert(&c, &entry("a"), T0).expect("up");
    let id = store::find(&c, "test", "a").expect("q").expect("e").id;
    for _ in 0..3 {
        assert!(!store::is_exhausted(&store::get(&c, id).expect("q").expect("e")));
        store::mark_surfaced(&c, id, T0).expect("s");
    }
    assert!(store::is_exhausted(&store::get(&c, id).expect("q").expect("e")));
}

#[test]
fn a_row_with_an_unknown_kind_is_left_out_of_the_ranking_not_a_panic() {
    let c = mem();
    store::migrate(&c).expect("m");
    store::upsert(&c, &entry("a"), T0).expect("up");
    set(&c, "UPDATE agenda SET kind = 'from-the-future'", []);
    assert!(super::ranked(&c, T0, 5).expect("r").is_empty());
}
