use super::*;
use crate::bond_table;

fn conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    BondTracker::ensure_tables(&conn);
    conn
}

/// What a card for `grade` said about its action: sensitive, nothing special.
fn facts(grade: &str, leaves: bool) -> ActionFacts {
    ActionFacts { grade: grade.into(), leaves_machine: leaves, ..Default::default() }
}

fn receipt(kind: ReceiptKind, app: &str, action: &str, facts: ActionFacts, id: &str) -> Receipt {
    Receipt { kind, app: app.into(), action: action.into(), facts, source_id: id.into() }
}

fn card_with(conn: &Connection, allowed: bool, app: &str, action: &str, facts: ActionFacts, id: &str) {
    let kind = if allowed { ReceiptKind::CardAllowed } else { ReceiptKind::CardRefused };
    assert!(BondTracker::record_receipt(conn, &receipt(kind, app, action, facts, id)), "{id} was counted");
}

fn card(conn: &Connection, allowed: bool, app: &str, action: &str, leaves: bool, id: &str) {
    card_with(conn, allowed, app, action, facts("sensitive", leaves), id);
}

/// The shell's own rule for whether a session rule may cover an action
/// (`approvals::session_rule_allowed`).
fn shell_allows(grade: &str, cannot_be_undone: bool) -> bool {
    grade != "dangerous" && !cannot_be_undone
}

fn score(conn: &Connection) -> f64 {
    BondTracker::get_state(conn).bond_score
}

/// Midnight UTC `days` calendar days ago, as a UNIX timestamp.
fn midnight_utc_days_ago(days: i64) -> f64 {
    (chrono::Utc::now().date_naive() - chrono::Duration::days(days))
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp() as f64
}

fn set_first_interaction(conn: &Connection, ts: f64) {
    conn.execute("UPDATE bond_state SET first_interaction_at = ?1 WHERE id = 1", params![ts]).unwrap();
}

// ── Absence and text ─────────────────────────────────────────────────

#[test]
fn thirty_days_of_absence_move_nothing() {
    let conn = conn();
    for n in 0..3 {
        card(&conn, true, "notes", "create", false, &format!("c{n}"));
    }
    let before = BondTracker::get_state(&conn);
    // Thirty days pass: every event is moved thirty days into the past, and the person never
    // comes back. Nothing that reads the clock can find anything to decay.
    conn.execute("UPDATE bond_events SET created_at = created_at - 30 * 86400.0", []).unwrap();
    conn.execute("UPDATE bond_state SET last_interaction_at = last_interaction_at - 30 * 86400.0", []).unwrap();
    let after = BondTracker::get_state(&conn);
    assert_eq!(before.bond_score, after.bond_score);
    assert_eq!(before.bond_level, after.bond_level);
    assert_eq!(before.receipts, after.receipts);
    assert!(before.bond_score > 0.0, "the test has something to lose");
}

#[test]
fn a_bond_nobody_has_earned_stays_at_zero_through_a_month_of_silence() {
    let conn = conn();
    set_first_interaction(&conn, midnight_utc_days_ago(30));
    assert_eq!(score(&conn), 0.0);
    assert_eq!(BondTracker::get_state(&conn).bond_level, BondLevel::Stranger);
}

#[test]
fn turns_move_nothing_whatever_was_said() {
    // `note_turn` takes no text at all, so "i trust you" and a wall of words cannot be told
    // apart from silence by it. A hundred turns are a hundred presences and zero trust.
    let conn = conn();
    for _ in 0..100 {
        BondTracker::note_turn(&conn);
    }
    let state = BondTracker::get_state(&conn);
    assert_eq!(state.bond_score, 0.0);
    assert_eq!(state.bond_level, BondLevel::Stranger);
    assert_eq!(state.total_interactions, 100);
    assert_eq!(state.receipts, 0);
}

#[test]
fn history_the_old_scorer_wrote_is_kept_and_not_counted() {
    let conn = conn();
    conn.execute(
        "UPDATE bond_state SET bond_score = 4.9, bond_level = 5, vulnerability_events = 7 WHERE id = 1",
        [],
    )
    .unwrap();
    for kind in ["interaction", "vulnerability", "humor_success", "milestone"] {
        conn.execute(
            "INSERT INTO bond_events (event_id, event_type, delta, context, created_at)
             VALUES (?1, ?2, 0.05, '{}', 1.0)",
            params![format!("old-{kind}"), kind],
        )
        .unwrap();
    }
    assert_eq!(score(&conn), 0.0, "old events are not receipts");
    let kept: i64 = conn.query_row("SELECT COUNT(*) FROM bond_events", [], |r| r.get(0)).unwrap();
    assert_eq!(kept, 4, "and they are still there");
}

// ── Receipts ─────────────────────────────────────────────────────────

#[test]
fn each_receipt_moves_the_score_the_right_way() {
    for kind in [ReceiptKind::CardAllowed, ReceiptKind::ForYouUseful, ReceiptKind::PreparedUsed] {
        let conn = conn();
        // A starting point above zero, so a fall has somewhere to go.
        for n in 0..5 {
            card(&conn, true, "notes", "create", false, &format!("base{n}"));
        }
        let before = score(&conn);
        assert!(BondTracker::record_receipt(&conn, &receipt(kind, "x", "y", facts("sensitive", false), "r")));
        assert!(score(&conn) > before, "{kind:?} raises it");
    }
    for kind in [ReceiptKind::CardRefused, ReceiptKind::ForYouNotUseful, ReceiptKind::Undone] {
        let conn = conn();
        for n in 0..5 {
            card(&conn, true, "notes", "create", false, &format!("base{n}"));
        }
        let before = score(&conn);
        assert!(BondTracker::record_receipt(&conn, &receipt(kind, "x", "y", facts("sensitive", false), "r")));
        assert!(score(&conn) < before, "{kind:?} lowers it");
    }
}

#[test]
fn the_score_stays_between_zero_and_the_ceiling() {
    let conn = conn();
    BondTracker::record_receipt(&conn, &receipt(ReceiptKind::Undone, "a", "b", facts("sensitive", false), "u"));
    assert_eq!(score(&conn), 0.0, "a fall from nothing is nothing");
    for n in 0..200 {
        card(&conn, true, "notes", &format!("create{n}"), false, &format!("c{n}"));
    }
    assert_eq!(score(&conn), SCORE_CEILING);
}

#[test]
fn the_same_press_is_counted_once() {
    let conn = conn();
    card(&conn, true, "notes", "create", false, "card-1");
    let r = receipt(ReceiptKind::CardAllowed, "notes", "create", facts("sensitive", false), "card-1");
    assert!(!BondTracker::record_receipt(&conn, &r));
    assert_eq!(BondTracker::get_state(&conn).receipts, 1);
}

#[test]
fn a_fall_keeps_the_reason_with_the_receipt_that_caused_it() {
    let conn = conn();
    card(&conn, true, "email", "send", true, "c1");
    card(&conn, false, "email", "send", true, "c2");
    BondTracker::record_receipt(&conn, &receipt(ReceiptKind::Undone, "files", "move", facts("sensitive", false), "u1"));
    let why = BondTracker::why_it_fell(&conn, 10);
    assert_eq!(why.len(), 2, "a rise needs no reason");
    assert!(why[0].contains("undid") && why[0].contains("files · move"));
    assert!(why[1].contains("refused") && why[1].contains("email · send"));
}

#[test]
fn levels_follow_the_receipt_score() {
    let conn = conn();
    assert_eq!(BondTracker::get_state(&conn).bond_level, BondLevel::Stranger);
    for n in 0..5 {
        card(&conn, true, "notes", "create", false, &format!("c{n}"));
    }
    assert_eq!(BondTracker::get_state(&conn).bond_level, BondLevel::Acquaintance);
}

// ── The table ────────────────────────────────────────────────────────

#[test]
fn the_table_matches_a_seeded_audit() {
    let conn = conn();
    for n in 0..4 {
        card(&conn, true, "notes", "create", false, &format!("n{n}"));
    }
    card(&conn, true, "email", "send", true, "e1");
    card(&conn, true, "files", "move", false, "f1");
    card(&conn, false, "files", "move", false, "f2");
    card(&conn, true, "files", "move", false, "f3");
    // Not a card: it has no row.
    BondTracker::record_receipt(&conn, &Receipt::bare(ReceiptKind::ForYouUseful, "fy"));

    let rows = bond_table::rows(&conn);
    let lines: Vec<String> = rows.iter().map(BondRow::line).collect();
    assert_eq!(
        lines,
        vec![
            "Email · send: answered 1, allowed 1",
            "Files · move: answered 3, allowed 2, refused 1",
            "Notes · create: answered 4, allowed 4",
        ]
    );
    let files = &rows[1];
    assert_eq!((files.answered, files.allowed, files.refused, files.clean_streak), (3, 2, 1, 1));
    assert_eq!(files.receipts, vec!["f3", "f2", "f1"], "the receipts behind it, newest first");
}

// ── The offer ────────────────────────────────────────────────────────

fn offered(conn: &Connection) -> Vec<String> {
    let rows = bond_table::rows(conn);
    bond_table::offers(&rows, shell_allows).iter().filter_map(|r| r.offer(shell_allows)).collect()
}

#[test]
fn the_offer_comes_at_four_of_four_and_not_before() {
    let conn = conn();
    for n in 0..3 {
        card(&conn, true, "notes", "create", false, &format!("n{n}"));
        assert!(offered(&conn).is_empty(), "{} of {} is not enough", n + 1, n + 1);
    }
    card(&conn, true, "notes", "create", false, "n3");
    assert_eq!(
        offered(&conn),
        vec!["Notes create has been allowed the last 4 times. Stop asking for this session?"]
    );
}

#[test]
fn a_refusal_takes_the_offer_away_and_starts_the_count_again() {
    let conn = conn();
    for n in 0..4 {
        card(&conn, true, "notes", "create", false, &format!("n{n}"));
    }
    assert_eq!(offered(&conn).len(), 1);
    card(&conn, false, "notes", "create", false, "no");
    assert!(offered(&conn).is_empty());
    for n in 0..3 {
        card(&conn, true, "notes", "create", false, &format!("m{n}"));
    }
    assert!(offered(&conn).is_empty(), "three since the refusal is not four");
    card(&conn, true, "notes", "create", false, "m3");
    assert_eq!(offered(&conn).len(), 1, "four clean answers since earn it again");
}

#[test]
fn what_leaves_the_machine_is_never_offered() {
    let conn = conn();
    for n in 0..12 {
        card(&conn, true, "email", "send", true, &format!("e{n}"));
    }
    let rows = bond_table::rows(&conn);
    assert_eq!(rows[0].clean_streak, 12, "it is a clean streak, and still no offer");
    assert!(offered(&conn).is_empty());
}

#[test]
fn what_ever_left_the_machine_stays_unoffered_even_if_the_newest_card_says_nothing() {
    // An app update drops the sentence for a moment: eight cards said it sends, the ninth does not.
    let conn = conn();
    for n in 0..8 {
        card(&conn, true, "email", "send", true, &format!("e{n}"));
    }
    card(&conn, true, "email", "send", false, "e8");
    let rows = bond_table::rows(&conn);
    assert!(!rows[0].facts.leaves_machine, "the newest card is what the gate reads now");
    assert!(rows[0].ever.leaves_machine, "and what any card said stays said");
    assert!(offered(&conn).is_empty());
}

#[test]
fn an_action_the_app_says_is_not_recoverable_is_never_offered() {
    // calendar.delete_event: graded sensitive, "It is not recoverable". The card has no session
    // button, so the offer must not promise one.
    let conn = conn();
    let gone = ActionFacts { cannot_be_undone: true, ..facts("sensitive", false) };
    for n in 0..6 {
        card_with(&conn, true, "calendar", "delete_event", gone.clone(), &format!("d{n}"));
    }
    assert_eq!(bond_table::rows(&conn)[0].clean_streak, 6);
    assert!(offered(&conn).is_empty());
}

#[test]
fn a_dangerous_action_is_never_offered() {
    let conn = conn();
    for n in 0..6 {
        card_with(&conn, true, "shell", "kill", facts("dangerous", false), &format!("k{n}"));
    }
    assert!(offered(&conn).is_empty());
}

#[test]
fn an_action_that_fetches_an_address_is_never_offered() {
    let conn = conn();
    let fetch = ActionFacts { fetches_address: true, ..facts("sensitive", false) };
    for n in 0..6 {
        card_with(&conn, true, "downloads", "add", fetch.clone(), &format!("f{n}"));
    }
    assert!(offered(&conn).is_empty());
}

#[test]
fn a_row_with_no_recorded_grade_is_not_offered() {
    let conn = conn();
    for n in 0..6 {
        card_with(&conn, true, "notes", "create", ActionFacts::default(), &format!("g{n}"));
    }
    assert!(offered(&conn).is_empty());
}

#[test]
fn the_facts_come_back_from_the_store_as_they_went_in() {
    let conn = conn();
    let all = ActionFacts {
        grade: "sensitive".into(),
        cannot_be_undone: true,
        open_ended: true,
        fetches_address: true,
        leaves_machine: true,
    };
    card_with(&conn, true, "x", "y", all.clone(), "c1");
    let row = &bond_table::rows(&conn)[0];
    assert_eq!(row.facts, all);
    assert_eq!(row.ever, all);
}

// ── Restarts, ids, order ─────────────────────────────────────────────

#[test]
fn a_restart_does_not_make_a_new_card_look_like_an_old_one() {
    // Boot 1 numbers its cards from 1 and so does boot 2; the shell makes the ids differ
    // (`approvals::Store::with_boot`), and the store counts both. Four allows, then, after a
    // restart, a refusal of the "same" first card for the same pair.
    let conn = conn();
    for n in 1..=4 {
        card(&conn, true, "notes", "create", false, &format!("appr-boot1-{n}"));
    }
    assert_eq!(offered(&conn).len(), 1);
    card(&conn, false, "notes", "create", false, "appr-boot2-1");
    let rows = bond_table::rows(&conn);
    assert_eq!((rows[0].answered, rows[0].allowed, rows[0].refused), (5, 4, 1));
    assert!(offered(&conn).is_empty(), "the refusal took the offer away");
    assert_eq!(BondTracker::why_it_fell(&conn, 5).len(), 1);
    // And the same card pressed twice in one boot is still one answer.
    assert!(!BondTracker::record_receipt(
        &conn,
        &receipt(ReceiptKind::CardRefused, "notes", "create", facts("sensitive", false), "appr-boot2-1")
    ));
}

#[test]
fn every_undo_counts_on_its_own() {
    let conn = conn();
    for n in 0..5 {
        card(&conn, true, "notes", "create", false, &format!("c{n}"));
        card(&conn, true, "files", "move", false, &format!("m{n}"));
    }
    let high = score(&conn);
    for _ in 0..2 {
        // No record of its own: the store gives each one an id.
        assert!(BondTracker::record_receipt(
            &conn,
            &receipt(ReceiptKind::Undone, "models", "wire_harness", ActionFacts::default(), "")
        ));
    }
    assert!((high - score(&conn) - 0.30).abs() < 1e-9, "two undos, two falls");
    assert_eq!(BondTracker::why_it_fell(&conn, 10).len(), 2);
}

#[test]
fn the_order_is_the_order_written_not_the_clock() {
    let conn = conn();
    for n in 0..4 {
        card(&conn, true, "notes", "create", false, &format!("n{n}"));
    }
    card(&conn, false, "notes", "create", false, "no");
    // The clock steps back: the refusal now reads as older than the allows before it.
    conn.execute("UPDATE bond_events SET created_at = created_at - 3600.0 WHERE event_id LIKE '%:no'", [])
        .unwrap();
    assert_eq!(bond_table::rows(&conn)[0].clean_streak, 0, "the refusal is still the last answer");
}

// ── The score, kept honest ───────────────────────────────────────────

#[test]
fn one_trivial_card_raised_over_and_over_cannot_buy_a_level() {
    let conn = conn();
    for n in 0..500 {
        card(&conn, true, "notes", "create", false, &format!("c{n}"));
    }
    assert_eq!(score(&conn), PAIR_RISE_CAP);
    assert_eq!(BondTracker::get_state(&conn).receipts, 500, "every one is still a receipt");
    card(&conn, false, "notes", "create", false, "no");
    assert!(score(&conn) < PAIR_RISE_CAP, "a refusal still counts in full");
}

// ── Keeping and forgetting ───────────────────────────────────────────

#[test]
fn receipts_are_kept_for_a_hundred_and_eighty_days() {
    let conn = conn();
    card(&conn, true, "notes", "create", false, "old");
    card(&conn, true, "files", "move", false, "recent");
    let days = |n: i64| (n as f64) * 86_400.0;
    conn.execute(
        "UPDATE bond_events SET created_at = created_at - ?1 WHERE event_id LIKE '%:old'",
        params![days(RECEIPT_RETENTION_DAYS + 1)],
    )
    .unwrap();
    conn.execute(
        "UPDATE bond_events SET created_at = created_at - ?1 WHERE event_id LIKE '%:recent'",
        params![days(RECEIPT_RETENTION_DAYS - 1)],
    )
    .unwrap();
    // Reading drops nothing: only a write (or a start) forgets.
    assert_eq!(BondTracker::get_state(&conn).receipts, 2);
    card(&conn, true, "email", "send", true, "new");
    let left: Vec<String> = bond_table::rows(&conn).iter().map(|r| r.app.clone()).collect();
    assert_eq!(left, vec!["email", "files"], "the 181-day-old receipt went, the 179-day-old stayed");
}

#[test]
fn forgetting_empties_the_receipts_and_keeps_the_presence() {
    let conn = conn();
    BondTracker::note_turn(&conn);
    for n in 0..5 {
        card(&conn, true, "notes", "create", false, &format!("c{n}"));
    }
    card(&conn, false, "files", "move", false, "no");
    assert_eq!(BondTracker::forget_receipts(&conn), 6);
    let state = BondTracker::get_state(&conn);
    assert_eq!((state.bond_score, state.receipts, state.bond_level), (0.0, 0, BondLevel::Stranger));
    assert!(bond_table::rows(&conn).is_empty());
    assert!(BondTracker::why_it_fell(&conn, 10).is_empty());
    assert_eq!(state.total_interactions, 1, "that the person was here is not a receipt");
}

// ── The ceiling ──────────────────────────────────────────────────────

#[test]
fn nothing_in_the_bond_can_touch_a_permission_ceiling() {
    // The bond reads receipts and writes receipts. The test is that it cannot name a permission:
    // no code here mentions the ceiling, so no path through it can raise one. Offers are applied
    // by the person's press in the shell, which is where the ceiling's own test lives.
    let bond = include_str!("bond.rs");
    let table = include_str!("bond_table.rs");
    let core = include_str!("../../yantrik-companion-core/src/bond.rs");
    for (name, source) in [("bond.rs", bond), ("bond_table.rs", table), ("core bond.rs", core)] {
        let code = yantrik_companion_core::text::code_only(source);
        for word in ["max_permission", "tool_permission", "PermissionLevel", "ceiling"] {
            assert!(!code.contains(word), "{name} must not reach `{word}`");
        }
    }
}

// ── Presence ─────────────────────────────────────────────────────────

#[test]
fn the_day_you_meet_is_day_one() {
    let conn = conn();
    BondTracker::note_turn(&conn);
    let state = BondTracker::get_state(&conn);
    assert!(state.first_interaction_at.is_some());
    assert_eq!(state.days_together, 1.0);
}

#[test]
fn days_together_counts_calendar_days_from_first_interaction() {
    let conn = conn();
    set_first_interaction(&conn, midnight_utc_days_ago(1));
    assert_eq!(BondTracker::get_state(&conn).days_together, 2.0);
    set_first_interaction(&conn, midnight_utc_days_ago(2));
    assert_eq!(BondTracker::get_state(&conn).days_together, 3.0);
}

#[test]
fn no_first_interaction_means_no_days_together() {
    assert_eq!(BondTracker::get_state(&conn()).days_together, 0.0);
}

#[test]
fn a_clock_jump_cannot_make_the_days_negative() {
    let conn = conn();
    set_first_interaction(&conn, (chrono::Utc::now() + chrono::Duration::hours(6)).timestamp() as f64);
    assert_eq!(BondTracker::get_state(&conn).days_together, 1.0);
}

#[test]
fn the_newest_interaction_event_is_when_the_person_was_last_here() {
    let conn = conn();
    assert_eq!(BondTracker::last_interaction_at(&conn), None);
    // A receipt is the person acting, but it is not a conversation turn: only a turn counts.
    card(&conn, true, "notes", "create", false, "c1");
    assert_eq!(BondTracker::last_interaction_at(&conn), None);

    BondTracker::note_turn(&conn);
    let two_days_ago = now_ts() - 2.0 * 86400.0;
    conn.execute(
        "UPDATE bond_events SET created_at = ?1 WHERE event_type = 'interaction'",
        params![two_days_ago],
    )
    .unwrap();
    let last = BondTracker::last_interaction_at(&conn).expect("one interaction now");
    assert!((last - two_days_ago).abs() < 1.0);
}
