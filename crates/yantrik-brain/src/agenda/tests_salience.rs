//! The score: each urgency function at, before and after its deadline; the total is the sum
//! of its terms; two promises order by deadline; 5,000 entries stay fast.

use rusqlite::params;

use super::salience::{salience, UrgencyFn};
use super::store::{self, NewEntry};
use super::test_support::*;
use super::{gather, ranked, Kind, NextStep, State};

const D: f64 = T0 + DAY;

fn at(f: &UrgencyFn, now: f64, dismissed: u32) -> f64 {
    f.evaluate(T0, now, dismissed)
}

#[test]
fn step_at_deadline_is_zero_before_the_buffer_and_one_from_it() {
    let f = UrgencyFn::StepAtDeadline { deadline: D, buffer_secs: HOUR };
    assert_eq!(at(&f, D - 2.0 * HOUR, 0), 0.0);
    assert_eq!(at(&f, D - HOUR, 0), 1.0);
    assert_eq!(at(&f, D, 0), 1.0);
    assert_eq!(at(&f, D + DAY, 0), 1.0);
}

#[test]
fn sigmoid_is_low_before_half_at_the_deadline_and_high_after() {
    let f = UrgencyFn::Sigmoid { deadline: D, steepness: 1.0 / 43_200.0, offset: 0.0 };
    let (before, on, after) = (at(&f, D - 3.0 * DAY, 0), at(&f, D, 0), at(&f, D + DAY, 0));
    assert!(before < 0.01, "{before}");
    assert!((on - 0.5).abs() < 1e-9);
    assert!(after > 0.85, "{after}");
    assert!(before < on && on < after);
}

#[test]
fn linear_grows_with_age_and_is_clamped_to_one() {
    let f = UrgencyFn::Linear { base: 0.1, slope: 0.1 / DAY };
    assert!((at(&f, T0, 0) - 0.1).abs() < 1e-9);
    assert!((at(&f, T0 + DAY, 0) - 0.2).abs() < 1e-9);
    assert_eq!(at(&f, T0 + 100.0 * DAY, 0), 1.0);
    // A clock that went backwards does not make it negative.
    assert!((at(&f, T0 - DAY, 0) - 0.1).abs() < 1e-9);
}

#[test]
fn decay_if_ignored_falls_with_each_dismissal() {
    let f = UrgencyFn::DecayIfIgnored { initial: 0.8, decay_factor: 0.5 };
    assert!((at(&f, T0, 0) - 0.8).abs() < 1e-9);
    assert!((at(&f, T0, 1) - 0.4).abs() < 1e-9);
    assert!((at(&f, T0, 2) - 0.2).abs() < 1e-9);
}

#[test]
fn constant_never_changes_and_out_of_range_values_are_clamped() {
    let f = UrgencyFn::Constant { value: 0.3 };
    assert_eq!(at(&f, T0, 0), at(&f, T0 + 1e6, 9));
    assert_eq!(at(&UrgencyFn::Constant { value: 7.0 }, T0, 0), 1.0);
    assert_eq!(at(&UrgencyFn::Constant { value: f64::NAN }, T0, 0), 0.0);
}

fn new(source_id: &str, urgency: UrgencyFn, confidence: f64) -> NewEntry {
    NewEntry {
        source: "test",
        source_id: source_id.into(),
        kind: Kind::Promise,
        title: source_id.into(),
        title_public: true,
        state: State::WaitingPerson,
        next_step: NextStep::Person,
        urgency,
        due_at: None,
        confidence,
        hold_until: 0.0,
        created_at: None,
    }
}

#[test]
fn the_total_is_the_sum_of_its_named_terms_and_each_follows_its_formula() {
    let c = mem();
    store::migrate(&c).expect("m");
    let mut e = new("a", UrgencyFn::Constant { value: 0.4 }, 0.7);
    e.due_at = Some(T0 + 10.0 * DAY);
    store::upsert(&c, &e, T0).expect("up");
    let now = T0 + 3.0 * DAY;
    let r = ranked(&c, now, 5).expect("r");
    let s = &r[0].1;
    let names: Vec<&str> = s.terms.iter().map(|t| t.0).collect();
    assert_eq!(names, ["deadline", "overdue", "stale", "blocked", "drives", "fit"]);
    let sum: f64 = s.terms.iter().map(|t| t.1).sum();
    assert_eq!(s.total, sum);
    assert!((s.terms[0].1 - 3.0 * 0.4 * 0.7).abs() < 1e-9, "confidence multiplies the deadline");
    assert_eq!(s.terms[1].1, 0.0, "not yet due");
    assert!((s.terms[2].1 - 0.3 * 3.0 / 14.0).abs() < 1e-9);
    assert!((s.terms[3].1 - 0.3 * 3.0 / 7.0).abs() < 1e-9, "blocked since it was added waiting");
    assert_eq!((s.terms[4].1, s.terms[5].1), (0.0, 0.0));
}

#[test]
fn confidence_is_clamped_between_the_floor_and_one() {
    let c = mem();
    store::migrate(&c).expect("m");
    store::upsert(&c, &new("low", UrgencyFn::Constant { value: 1.0 }, 0.0), T0).expect("up");
    store::upsert(&c, &new("high", UrgencyFn::Constant { value: 1.0 }, 9.0), T0).expect("up");
    let r = ranked(&c, T0, 5).expect("r");
    let d = |t: &str| r.iter().find(|x| x.0.title == t).map(|x| x.1.terms[0].1).expect("t");
    assert!((d("low") - 0.9).abs() < 1e-9 && (d("high") - 3.0).abs() < 1e-9);
}

#[test]
fn stale_blocked_and_overdue_terms_saturate() {
    let c = mem();
    store::migrate(&c).expect("m");
    let mut e = new("a", UrgencyFn::Constant { value: 0.0 }, 0.0);
    e.due_at = Some(T0);
    store::upsert(&c, &e, T0).expect("up");
    let now = T0 + 1000.0 * DAY;
    let s = salience(&ranked(&c, now, 5).expect("r")[0].0, now);
    assert!((s.terms[1].1 - 1.0).abs() < 1e-9);
    assert!((s.terms[2].1 - 0.3).abs() < 1e-9 && (s.terms[3].1 - 0.3).abs() < 1e-9);
}

#[test]
fn the_dated_urgency_is_one_when_due_and_matches_the_decided_values() {
    let f = UrgencyFn::dated(D);
    let u = |left_days: f64| f.evaluate(T0, D - left_days * DAY, 0);
    assert_eq!(f.evaluate(T0, D, 0), 1.0);
    assert_eq!(f.evaluate(T0, D + 5.0 * DAY, 0), 1.0, "once due it stays 1");
    let near = |a: f64, b: f64| (a - b).abs() < 0.005;
    assert!(near(u(1.0), 0.67) && near(u(2.0), 0.5) && near(u(5.0), 0.29) && near(u(14.0), 0.125));
}

#[test]
fn the_dated_urgency_strictly_decreases_with_time_left_from_14_days_to_0() {
    let f = UrgencyFn::dated(T0 + 20.0 * DAY);
    let mut last = f64::MAX;
    for step in 0..=(14 * 24 * 4) {
        // 15-minute steps, from 14 days left down to 0 left.
        let now = T0 + 6.0 * DAY + step as f64 * 900.0;
        let u = f.evaluate(T0, now, 0);
        if step > 0 {
            assert!(u > 0.0 && u < 1.0 + 1e-12);
        }
        assert!(last == f64::MAX || u > last, "step {step}: {u} !> {last}");
        last = u;
    }
}

#[test]
fn the_undated_promise_grows_slowly_and_is_capped_below_a_dated_one_within_eight_days() {
    let f = UrgencyFn::undated_promise();
    assert!((f.evaluate(T0, T0, 0) - 0.10).abs() < 1e-9);
    assert!((f.evaluate(T0, T0 + 36.5 * DAY, 0) - 0.20).abs() < 1e-9);
    assert!((f.evaluate(T0, T0 + 10_000.0 * DAY, 0) - 0.20).abs() < 1e-9, "capped");
    let eight_days = UrgencyFn::dated(T0 + 8.0 * DAY).evaluate(T0, T0, 0);
    assert!(eight_days >= 0.2 - 1e-9, "{eight_days}");
}

#[test]
fn task_urgency_follows_the_priority_table() {
    assert_eq!([4, 3, 2, 1].map(super::salience::task_urgency), [0.60, 0.45, 0.30, 0.15]);
    assert_eq!(super::salience::task_urgency(99), 0.60);
    assert_eq!(super::salience::task_urgency(-3), 0.15);
}

#[test]
fn two_dated_promises_order_by_deadline_whichever_came_first_and_as_time_passes() {
    let c = mem();
    make_promises(&c);
    add_promise(&c, "in 5 days", T0 + 5.0 * DAY);
    add_promise(&c, "in 2 days", T0 + 2.0 * DAY);
    gather(&c, T0).expect("gather");
    for now in [T0, T0 + 6.0 * HOUR] {
        let titles: Vec<String> = ranked(&c, now, 5).expect("r").into_iter().map(|x| x.0.title).collect();
        assert_eq!(titles, ["in 2 days", "in 5 days"], "at {now}");
    }
}

#[test]
fn a_promise_due_tomorrow_beats_a_normal_task_even_at_weak_confidence() {
    for conf in [0.5, 0.6] {
        let c = mem();
        make_promises(&c);
        make_tasks(&c);
        let p = add_promise(&c, "tomorrow", T0 + DAY);
        set(&c, "UPDATE wm_commitments SET confidence = ?1 WHERE id = ?2", params![conf, p]);
        add_task(&c, "t1", "normal task", 2);
        gather(&c, T0).expect("gather");
        let titles: Vec<String> = ranked(&c, T0, 5).expect("r").into_iter().map(|x| x.0.title).collect();
        assert_eq!(titles, ["tomorrow", "normal task"], "confidence {conf}");
    }
}

#[test]
fn a_sixty_day_old_undated_promise_does_not_outrank_one_due_friday() {
    let c = mem();
    make_promises(&c);
    let old = add_promise(&c, "sometime", 0.0);
    set(&c, "UPDATE wm_commitments SET created_at = ?1 WHERE id = ?2", params![T0 - 60.0 * DAY, old]);
    add_promise(&c, "by Friday", T0 + 3.0 * DAY);
    gather(&c, T0).expect("gather");
    let r = ranked(&c, T0, 5).expect("r");
    assert_eq!(r[0].0.title, "by Friday", "{:?}", r.iter().map(|x| x.1.total).collect::<Vec<_>>());
}

#[test]
fn an_overdue_promise_ranks_above_one_due_in_an_hour() {
    let c = mem();
    make_promises(&c);
    add_promise(&c, "in an hour", T0 + HOUR);
    add_promise(&c, "late", T0 - 2.0 * DAY);
    gather(&c, T0).expect("gather");
    let r = ranked(&c, T0, 5).expect("r");
    assert_eq!(r[0].0.title, "late");
    assert!(r[0].1.terms[1].1 > 0.6, "overdue term {:?}", r[0].1.terms);
}

#[test]
fn ties_break_by_nearer_deadline_then_older_then_id() {
    let c = mem();
    store::migrate(&c).expect("m");
    let mut a = new("a", UrgencyFn::Constant { value: 0.5 }, 1.0);
    let mut b = new("b", UrgencyFn::Constant { value: 0.5 }, 1.0);
    let mut d = new("d", UrgencyFn::Constant { value: 0.5 }, 1.0);
    // All score alike, because the deadline term ignores due_at here and nothing is overdue.
    a.due_at = Some(T0 + 9.0 * DAY);
    b.due_at = Some(T0 + 3.0 * DAY);
    d.due_at = None;
    for e in [&a, &d, &b] {
        store::upsert(&c, e, T0).expect("up");
    }
    let titles: Vec<String> = ranked(&c, T0, 5).expect("r").into_iter().map(|x| x.0.title).collect();
    assert_eq!(titles, ["b", "a", "d"]);
}

#[test]
fn the_score_is_the_same_every_time() {
    let c = mem();
    store::migrate(&c).expect("m");
    for i in 0..20 {
        store::upsert(&c, &new(&format!("e{i}"), UrgencyFn::Constant { value: 0.5 }, 0.5), T0).expect("up");
    }
    let a: Vec<i64> = ranked(&c, T0 + 5.0, 20).expect("r").iter().map(|x| x.0.id).collect();
    let b: Vec<i64> = ranked(&c, T0 + 5.0, 20).expect("r").iter().map(|x| x.0.id).collect();
    assert_eq!(a, b);
    assert_eq!(a, (1..=20).collect::<Vec<i64>>(), "ties break on id");
}

#[test]
fn the_nearer_promise_ranks_first_with_its_deadline_term_largest() {
    let c = mem();
    make_promises(&c);
    add_promise(&c, "send Sam the report by Friday", T0 + 3.0 * DAY);
    add_promise(&c, "call Ana in an hour", T0 + HOUR);
    gather(&c, T0).expect("gather");
    let r = ranked(&c, T0, 5).expect("r");
    assert_eq!(r[0].0.title, "call Ana in an hour");
    assert_eq!(r[1].0.title, "send Sam the report by Friday");
    assert!(r[0].1.total > r[1].1.total);
    let term = |s: &super::Salience, n: &str| s.terms.iter().find(|t| t.0 == n).map(|t| t.1).unwrap_or(0.0);
    let largest = r[0].1.terms.iter().cloned().fold(("", f64::MIN), |a, t| if t.1 > a.1 { t } else { a });
    assert_eq!(largest.0, "deadline");
    assert!(term(&r[0].1, "deadline") > term(&r[1].1, "deadline"));
}

#[test]
fn five_thousand_active_entries_over_closed_history_on_a_file_are_gathered_and_ranked_in_a_bound() {
    let dir = std::env::temp_dir().join(format!("agenda-timing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let c = rusqlite::Connection::open(dir.join("m.db")).expect("open");
    make_promises(&c);
    // A year of history: thousands of finished promises, in the source and closed in the agenda.
    c.execute_batch("BEGIN").expect("b");
    for i in 0..3000 {
        add_promise(&c, &format!("old {i}"), T0 + (i as f64));
    }
    c.execute_batch("COMMIT").expect("c");
    gather(&c, T0).expect("history in");
    set(&c, "UPDATE wm_commitments SET status = 'completed'", []);
    gather(&c, T0 + 1.0).expect("history closed");
    let closed: i64 = c.query_row("SELECT COUNT(*) FROM agenda WHERE is_open = 0", [], |r| r.get(0)).expect("n");
    assert_eq!(closed, 3000);

    c.execute_batch("BEGIN").expect("b");
    for i in 0..5000 {
        add_promise(&c, &format!("promise {i}"), T0 + (i as f64) * 600.0);
    }
    c.execute_batch("COMMIT").expect("c");
    let t = std::time::Instant::now();
    gather(&c, T0 + 10.0).expect("first pass");
    let first = t.elapsed();
    let t = std::time::Instant::now();
    let rep = gather(&c, T0 + 60.0).expect("second pass");
    let r = ranked(&c, T0 + 60.0, 5).expect("ranked");
    let second = t.elapsed();
    assert_eq!(rep.sources[0].read, 5000, "the 3000 finished rows are not read");
    assert_eq!(r.len(), 5);
    assert_eq!(r[0].0.title, "promise 0");
    // The aim is a few milliseconds; the bound leaves a very wide margin for a slow CI machine.
    assert!(first.as_millis() < 3000, "first pass took {first:?}");
    assert!(second.as_millis() < 3000, "steady pass took {second:?}");
    eprintln!("5000 active over 3000 closed, file db: first pass {first:?}, steady pass + rank {second:?}");
    drop(c);
    let _ = std::fs::remove_dir_all(&dir);
}
