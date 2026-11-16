//! How much of the watcher's time each connection may have.

use super::*;


#[test]
fn a_connection_is_asked_only_its_ration_in_a_second() {
    let start = Instant::now();
    let mut patience = Patience::new(start);
    let asked = (0..PER_CONNECTION * 3).filter(|_| patience.allows(":1.7", start)).count();
    assert_eq!(asked as u32, PER_CONNECTION);
    assert!(patience.allows(":1.7", start + Duration::from_secs(1)), "a new second, a new ration");
}

#[test]
fn one_greedy_connection_does_not_spend_another_ones_ration() {
    let start = Instant::now();
    let mut patience = Patience::new(start);
    while patience.allows(":1.7", start) {}
    assert!(patience.allows(":1.8", start), "the other application is still asked");
}

#[test]
fn a_connection_that_timed_out_is_left_alone_for_a_while() {
    let start = Instant::now();
    let mut patience = Patience::new(start);
    assert!(patience.allows(":1.7", start));
    patience.timed_out(":1.7", start);
    assert!(!patience.allows(":1.7", start + Duration::from_secs(5)), "still slow");
    assert!(patience.allows(":1.8", start + Duration::from_secs(5)), "nobody else pays for it");
    assert!(patience.allows(":1.7", start + SLOW_FOR), "asked again once the time has passed");
}

#[test]
fn the_table_of_connections_does_not_grow_without_bound() {
    let start = Instant::now();
    let mut patience = Patience::new(start);
    for n in 0..10_000u64 {
        patience.allows(&format!(":1.{n}"), start + Duration::from_secs(n));
    }
    assert!(patience.tracked() <= MAX_TRACKED, "{}", patience.tracked());
}

/// #1257 review, deviation 1: `timed_out` used to change only a connection that was already
/// tracked, and one first met in a listing never was.
#[test]
fn a_connection_that_did_not_answer_a_listing_is_left_alone() {
    let start = Instant::now();
    let mut patience = Patience::new(start);
    assert!(patience.timed_out(":1.77", start), "newly marked");
    assert!(!patience.allows(":1.77", start + Duration::from_secs(1)), "never asked before, and now left alone");
    assert!(patience.is_slow(":1.77", start + Duration::from_secs(1)));
    assert!(patience.allows(":1.77", start + SLOW_FOR), "and asked again when its time is up");
}

/// #1257 review, deviation 1: 256 slow connections, from one concurrent listing, used to be
/// able to refuse every connection not already tracked for ten seconds.
#[test]
fn a_table_full_of_slow_connections_still_has_room_for_an_honest_one() {
    let start = Instant::now();
    let mut patience = Patience::new(start);
    for n in 0..1000u64 {
        patience.timed_out(&format!(":1.{n}"), start + Duration::from_millis(n));
    }
    assert!(patience.tracked() <= MAX_TRACKED, "{}", patience.tracked());
    assert!(patience.allows(":1.honest", start + Duration::from_secs(1)), "the honest application is asked");
    assert!(patience.is_slow(":1.999", start + Duration::from_secs(1)), "the newest slow ones are still remembered");
    assert!(!patience.is_slow(":1.0", start + Duration::from_secs(1)), "the one let off soonest made room");
}

/// #1257 review, N2: one connection that answers every question in 390 ms was never marked
/// slow, and could be asked 20 times a second: 7.8 s of the watcher's time every second.
#[test]
fn a_connection_that_answers_just_inside_the_limit_gets_only_its_share_of_time() {
    let start = Instant::now();
    let mut patience = Patience::new(start);
    let answer = Duration::from_millis(390);
    let (mut spent, seconds) = (Duration::ZERO, 60u64);
    for ms in (0..seconds * 1000).step_by(5) {
        let now = start + Duration::from_millis(ms);
        if patience.allows(":1.7", now) {
            patience.spent(":1.7", answer, now);
            spent += answer;
        }
    }
    // Its share, and the one overrun it may borrow.
    let share = CONNECTION_TIME_PER_SECOND * seconds as u32 + answer;
    assert!(spent <= share, "{spent:?} of the watcher's time in {seconds} s, share {share:?}");
    assert!(spent >= answer, "it was asked at all");
}

/// #1257 review, N2: bus connections cost nothing, so 50 of them could each spend their own
/// share and 25 of them 400 ms each.
#[test]
fn fifty_connections_cannot_spend_more_than_the_total_time() {
    let start = Instant::now();
    let mut patience = Patience::new(start);
    let answer = Duration::from_millis(40);
    let (mut spent, seconds) = (Duration::ZERO, 20u64);
    for ms in (0..seconds * 1000).step_by(5) {
        let now = start + Duration::from_millis(ms);
        for n in 0..50 {
            let connection = format!(":1.{n}");
            if patience.allows(&connection, now) {
                patience.spent(&connection, answer, now);
                spent += answer;
            }
        }
    }
    // The total, and the loan the light may take against it.
    let share = TOTAL_TIME_PER_SECOND * seconds as u32 + TOTAL_CEILING + answer;
    assert!(spent <= share, "{spent:?} of the watcher's time in {seconds} s, share {share:?}");
    assert!(spent >= TOTAL_TIME_PER_SECOND * (seconds as u32 - 1), "and the time is used: {spent:?}");
}

#[test]
fn answers_slower_than_a_tenth_of_a_second_count_towards_marking_slow() {
    let start = Instant::now();
    let mut patience = Patience::new(start);
    let slow = SLOW_ANSWER + Duration::from_millis(1);
    for n in 0..STRIKES - 1 {
        assert!(!patience.spent(":1.7", slow, start + Duration::from_secs(u64::from(n) * 5)));
    }
    assert!(!patience.is_slow(":1.7", start + Duration::from_secs(9)), "not yet");
    assert!(patience.spent(":1.7", slow, start + Duration::from_secs(10)), "the last strike marks it");
    assert!(patience.is_slow(":1.7", start + Duration::from_secs(11)));
    assert_eq!(patience.marks(), 1);
    // Quick answers, however many, are never a strike.
    for n in 0..1000u64 {
        patience.spent(":1.8", SLOW_ANSWER, start + Duration::from_millis(n));
    }
    assert!(!patience.is_slow(":1.8", start + Duration::from_secs(2)));
}

#[test]
fn slow_answers_far_apart_do_not_add_up() {
    let start = Instant::now();
    let mut patience = Patience::new(start);
    let slow = SLOW_ANSWER * 2;
    for n in 0..10u64 {
        patience.spent(":1.7", slow, start + (SLOW_FOR + Duration::from_secs(1)) * n as u32);
    }
    assert_eq!(patience.marks(), 0, "a connection slow now and then is not a slow connection");
}

#[test]
fn marking_slow_is_counted_once_a_stretch_not_once_a_question() {
    let start = Instant::now();
    let mut patience = Patience::new(start);
    for n in 0..50u64 {
        patience.timed_out(":1.7", start + Duration::from_millis(n * 100));
    }
    assert_eq!(patience.marks(), 1);
    patience.timed_out(":1.7", start + SLOW_FOR + Duration::from_secs(6));
    assert_eq!(patience.marks(), 2, "and again once it has been let off");
}

#[test]
fn a_thousand_requests_for_a_listing_are_one_listing_a_second() {
    let start = Instant::now();
    let mut relists = Relists::new(start);
    let mut ran = 0;
    // A forged signal every millisecond for ten seconds, each one asking for a listing.
    for ms in 0..10_000u64 {
        let now = start + Duration::from_millis(ms);
        relists.want();
        if relists.due(now) {
            ran += 1;
            relists.ran(true, Duration::ZERO, now);
        }
    }
    assert_eq!(ran, 10);
}

#[test]
fn a_listing_that_failed_stays_wanted_and_is_retried_with_a_growing_pause() {
    let start = Instant::now();
    let mut relists = Relists::new(start);
    relists.want();
    assert!(relists.due(start));
    relists.ran(false, Duration::ZERO, start);
    assert!(!relists.due(start + Duration::from_millis(900)), "not at once");
    assert!(relists.due(start + RELIST_GAP), "but again after the first pause");
    relists.ran(false, Duration::ZERO, start + RELIST_GAP);
    assert_eq!(relists.next(), Some(start + RELIST_GAP + RELIST_GAP * 2), "the pause doubles");
    relists.ran(true, Duration::ZERO, start + Duration::from_secs(5));
    assert!(relists.next().is_none(), "one that worked settles it");
    for _ in 0..20 {
        relists.want();
        relists.ran(false, Duration::ZERO, start);
    }
    assert_eq!(relists.next(), Some(start + RELIST_RETRY_CAP), "and the pause has a ceiling");
}

/// #1257 review, H1 (RX4 inverted): six connections that answer at once used the 120 questions
/// of a second between them, and an honest application's window was refused.
#[test]
fn six_connections_spending_every_question_do_not_stop_a_quiet_one() {
    let t = Instant::now();
    let mut p = Patience::new(t);
    for c in 0..6 {
        let name = format!(":1.{c}");
        while p.allows(&name, t) {
            p.spent(&name, Duration::ZERO, t);
        }
    }
    assert!(p.allows(":1.honest", t + Duration::from_millis(10)));
}

/// #1257 review, H1 (RX5 inverted): ten connections that each answer in 49 ms, inside their own
/// 50 ms, kept the 250 ms total spent, and a quiet connection that asked every 100 ms was let
/// in fewer than one time in ten.
#[test]
fn connections_within_their_share_do_not_starve_a_quiet_one() {
    let start = Instant::now();
    let mut p = Patience::new(start);
    let (mut asked, mut let_in) = (0, 0);
    for ms in 0..18_000u64 {
        let now = start + Duration::from_millis(ms);
        for n in 0..10 {
            let name = format!(":1.{n}");
            if p.allows(&name, now) {
                p.spent(&name, Duration::from_millis(49), now);
            }
        }
        // About every 100 ms, and not in step with anything the others do.
        if ms % 97 == 37 {
            asked += 1;
            if p.allows(":1.quiet", now) {
                let_in += 1;
                p.spent(":1.quiet", Duration::from_millis(1), now);
            }
        }
    }
    assert!(let_in * 10 >= asked * 9, "the quiet one was let in {let_in} times of {asked}");
}

/// With the total spent, one that owes next to nothing is let in if the watcher is not so far
/// behind that its expected cost passes the ceiling, and a stranger is expected to cost
/// [`EXPECTED_ANSWER`].
#[test]
fn with_the_total_spent_those_that_have_shown_they_are_quick_are_let_in_ahead() {
    let start = Instant::now();
    let mut p = Patience::new(start);
    p.spent(":1.known", Duration::from_millis(1), start);
    p.spent(":1.big", Duration::from_millis(49), start);
    p.spent(":1.some", Duration::from_millis(4), start);
    let t = start + Duration::from_secs(1);
    // Everyone together is 980 ms behind.
    p.listing_took(TOTAL_CEILING - Duration::from_millis(20), t);
    assert!(p.allows(":1.known", t), "it owes nothing and answers in a millisecond");
    assert!(!p.allows(":1.big", t), "it owes nothing, but its last answer took 49 ms");
    assert!(!p.allows(":1.stranger", t), "and nobody has asked this one yet: it is expected to take 25");
    assert!(p.allows(":1.some", t), "owing 4 ms is still less than a tenth of a share");
    p.spent(":1.some", Duration::from_millis(40), t);
    assert!(!p.allows(":1.some", t), "owing 40 is not");
    let behind_by_less = start + Duration::from_secs(5);
    p.listing_took(TOTAL_TIME_PER_SECOND * 3, behind_by_less);
    assert!(p.allows(":1.big", behind_by_less), "behind by 750, even the slow are let in if they owe nothing");
    assert!(p.allows(":1.stranger", behind_by_less));
}

/// #1257 review, M1 (RX6 inverted): at the 257th connection, every entry that was not slow was
/// forgotten, strikes and debt with it, so applications answering in 390 ms were never marked.
#[test]
fn a_full_table_does_not_forget_strikes() {
    let start = Instant::now();
    let mut p = Patience::new(start);
    let slow = LISTED_SLOW_ANSWER + Duration::from_millis(1);
    // One that has struck twice and is not slow yet, and a table full of connections that owe
    // nothing and were used since.
    p.listed(":1.slow", Duration::from_millis(390), start);
    p.listed(":1.slow", Duration::from_millis(390), start);
    for n in 0..MAX_TRACKED as u64 - 1 {
        p.allows(&format!(":1.{n}"), start + Duration::from_secs(5) + Duration::from_millis(n));
    }
    assert_eq!(p.tracked(), MAX_TRACKED);
    for n in 0..50u64 {
        p.allows(&format!(":1.new{n}"), start + Duration::from_secs(6) + Duration::from_millis(n));
        assert_eq!(p.tracked(), MAX_TRACKED, "one forgotten for one that came");
    }
    assert!(p.listed(":1.slow", slow, start + Duration::from_secs(7)), "its strikes were kept: the third marks it");
    assert!(p.is_slow(":1.slow", start + Duration::from_secs(8)));
}

#[test]
fn the_entry_forgotten_is_the_one_that_owes_least_and_was_used_longest_ago() {
    let start = Instant::now();
    let mut p = Patience::new(start);
    for n in 0..MAX_TRACKED as u64 {
        p.allows(&format!(":1.{n}"), start + Duration::from_millis(n));
    }
    p.listed(":1.0", Duration::from_millis(30), start + Duration::from_millis(300));
    p.allows(":1.new", start + Duration::from_millis(301));
    assert!(p.connections.contains_key(":1.0"), "it owes something, so it is kept though it is the oldest");
    assert!(!p.connections.contains_key(":1.1"), "the next oldest, that owes nothing, went");
    assert!(p.connections.contains_key(":1.2") && p.connections.contains_key(":1.new"));
}

/// #1257 review, M1: an honest `GetChildren` takes a few milliseconds.
#[test]
fn a_listing_answer_over_a_quarter_of_slow_is_a_strike() {
    let start = Instant::now();
    let mut p = Patience::new(start);
    let answer = Duration::from_millis(60);
    assert!(answer < SLOW_ANSWER && answer > LISTED_SLOW_ANSWER);
    for n in 0..STRIKES - 1 {
        assert!(!p.listed(":1.7", answer, start + Duration::from_secs(u64::from(n))));
    }
    assert!(p.listed(":1.7", answer, start + Duration::from_secs(3)), "three strikes in a listing's time mark it");
    assert!(!p.spent(":1.8", answer, start), "asked on its own, 60 ms is not a strike");
    assert!(!p.listed(":1.9", LISTED_SLOW_ANSWER, start), "and 25 ms is not one in a listing");
}

#[test]
fn a_connection_that_owes_its_share_is_busy_for_a_listing_until_it_drains() {
    let start = Instant::now();
    let mut p = Patience::new(start);
    p.listed(":1.7", Duration::from_millis(60), start);
    assert!(p.is_busy(":1.7", start), "it owes more than its share");
    assert!(!p.is_busy(":1.7", start + Duration::from_secs(1)), "and a second later it does not");
    assert!(!p.is_busy(":1.unknown", start));
    p.timed_out(":1.8", start);
    assert!(p.is_busy(":1.8", start), "slow is busy");
}

/// #1257 review, M1: a listing blocks the watcher, and the questions that follow are refused
/// until the total has drained.
#[test]
fn a_listing_is_charged_to_everyone_together() {
    let start = Instant::now();
    let mut p = Patience::new(start);
    p.listing_took(Duration::from_secs(1), start);
    p.spent(":1.8", Duration::from_millis(10), start);
    assert!(!p.allows(":1.8", start), "the total is spent, and it owes more than a tenth of its share");
    assert!(p.allows(":1.7", start + Duration::from_secs(4)), "and a few seconds later it has drained");
}

/// #1257 review, M1: listings take no more than a quarter of the watcher's time.
#[test]
fn listings_take_no_more_than_a_quarter_of_the_watchers_time() {
    let start = Instant::now();
    let mut relists = Relists::new(start);
    let took = Duration::from_millis(990);
    let (mut inside, seconds) = (Duration::ZERO, 120u64);
    let mut now = start;
    while now < start + Duration::from_secs(seconds) {
        // A throwaway connection asks for a listing every 10 ms.
        relists.want();
        if relists.due(now) {
            inside += took;
            now += took;
            relists.ran(true, took, now);
        }
        now += Duration::from_millis(10);
    }
    let elapsed = now.duration_since(start);
    assert!(inside * 4 <= elapsed + took * RELIST_SPACING, "{inside:?} of {elapsed:?} inside listings");
    assert!(inside * 8 >= elapsed, "and they still run: {inside:?} of {elapsed:?}");
}

#[test]
fn a_quick_listing_is_still_one_a_second_and_a_failed_slow_one_waits_longer() {
    let start = Instant::now();
    let mut relists = Relists::new(start);
    relists.want();
    relists.ran(true, Duration::from_millis(30), start);
    relists.want();
    assert!(!relists.due(start + Duration::from_millis(990)) && relists.due(start + RELIST_GAP));
    relists.ran(false, Duration::from_secs(2), start + RELIST_GAP);
    assert_eq!(relists.next(), Some(start + RELIST_GAP + Duration::from_secs(6)), "three times what it took");
}
