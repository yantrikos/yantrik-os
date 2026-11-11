use super::*;
use crate::bond_table;

fn conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    BondTracker::ensure_tables(&conn);
    conn
}

fn receipt(kind: ReceiptKind, app: &str, action: &str, leaves: bool, id: &str) -> Receipt {
    Receipt {
        kind,
        app: app.into(),
        action: action.into(),
        leaves_machine: leaves,
        source_id: id.into(),
    }
}

fn card(conn: &Connection, allowed: bool, app: &str, action: &str, leaves: bool, id: &str) {
    let kind = if allowed { ReceiptKind::CardAllowed } else { ReceiptKind::CardRefused };
    assert!(BondTracker::record_receipt(conn, &receipt(kind, app, action, leaves, id)));
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
        assert!(BondTracker::record_receipt(&conn, &receipt(kind, "x", "y", false, "r")));
        assert!(score(&conn) > before, "{kind:?} raises it");
    }
    for kind in [ReceiptKind::CardRefused, ReceiptKind::ForYouNotUseful, ReceiptKind::Undone] {
        let conn = conn();
        for n in 0..5 {
            card(&conn, true, "notes", "create", false, &format!("base{n}"));
        }
        let before = score(&conn);
        assert!(BondTracker::record_receipt(&conn, &receipt(kind, "x", "y", false, "r")));
        assert!(score(&conn) < before, "{kind:?} lowers it");
    }
}

#[test]
fn the_score_stays_between_zero_and_the_ceiling() {
    let conn = conn();
    BondTracker::record_receipt(&conn, &receipt(ReceiptKind::Undone, "a", "b", false, "u"));
    assert_eq!(score(&conn), 0.0, "a fall from nothing is nothing");
    for n in 0..200 {
        card(&conn, true, "notes", "create", false, &format!("c{n}"));
    }
    assert_eq!(score(&conn), SCORE_CEILING);
}

#[test]
fn the_same_press_is_counted_once() {
    let conn = conn();
    card(&conn, true, "notes", "create", false, "card-1");
    let r = receipt(ReceiptKind::CardAllowed, "notes", "create", false, "card-1");
    assert!(!BondTracker::record_receipt(&conn, &r));
    assert_eq!(BondTracker::get_state(&conn).receipts, 1);
}

#[test]
fn a_fall_keeps_the_reason_with_the_receipt_that_caused_it() {
    let conn = conn();
    card(&conn, true, "email", "send", true, "c1");
    card(&conn, false, "email", "send", true, "c2");
    BondTracker::record_receipt(&conn, &receipt(ReceiptKind::Undone, "files", "move", false, "u1"));
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
    BondTracker::record_receipt(&conn, &receipt(ReceiptKind::ForYouUseful, "", "", false, "fy"));

    let rows = bond_table::rows(&conn);
    let lines: Vec<String> = rows.iter().map(BondRow::line).collect();
    assert_eq!(
        lines,
        vec![
            "Email · send: asked 1, allowed 1",
            "Files · move: asked 3, allowed 2, refused 1",
            "Notes · create: asked 4, allowed 4",
        ]
    );
    let files = &rows[1];
    assert_eq!((files.asked, files.allowed, files.refused, files.clean_streak), (3, 2, 1, 1));
    assert_eq!(files.receipts, vec!["f3", "f2", "f1"], "the receipts behind it, newest first");
}

// ── The offer ────────────────────────────────────────────────────────

fn offered(conn: &Connection) -> Vec<String> {
    let rows = bond_table::rows(conn);
    bond_table::offers(&rows).iter().filter_map(|r| r.offer()).collect()
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
        vec!["Notes create has been allowed 4 of 4 times. Stop asking for this session?"]
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
        let code: String = source
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
            .split("#[cfg(test)]")
            .next()
            .unwrap()
            .to_string();
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
