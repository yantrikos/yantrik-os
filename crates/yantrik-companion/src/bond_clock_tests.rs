use super::tests::{card, conn};
use super::*;
use crate::bond_table;

// Receipt retention and clock faults.

#[test]
fn receipts_are_kept_for_a_hundred_and_eighty_days() {
    let conn = conn();
    card(&conn, true, "notes", "create", false, "old");
    card(&conn, true, "files", "move", false, "recent");
    card(&conn, true, "notes", "list", false, "newest");
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
    assert_eq!(BondTracker::get_state(&conn).receipts, 3);
    card(&conn, true, "email", "send", true, "new");
    let left: Vec<String> = bond_table::rows(&conn)
        .iter()
        .map(|r| r.app.clone())
        .collect();
    assert_eq!(
        left,
        vec!["email", "files", "notes"],
        "the 181-day-old receipt went, the 179-day-old stayed"
    );
}

fn stamps(conn: &Connection) -> Vec<f64> {
    let mut stmt = conn
        .prepare("SELECT created_at FROM bond_events WHERE event_type = 'receipt' ORDER BY rowid")
        .unwrap();
    stmt.query_map([], |row| row.get::<_, f64>(0))
        .unwrap()
        .flatten()
        .collect()
}

fn backdate(conn: &Connection, id: &str, days: f64) {
    conn.execute(
        "UPDATE bond_events SET created_at = created_at - ?1 WHERE event_id LIKE ?2",
        params![days * 86_400.0, format!("%:{id}")],
    )
    .unwrap();
}

#[test]
fn a_receipt_backdated_181_days_is_gone_after_one_write_and_a_restart() {
    let conn = conn();
    card(&conn, false, "notes", "create", false, "old");
    backdate(&conn, "old", 181.0);
    card(&conn, true, "notes", "create", false, "a1");
    BondTracker::ensure_tables(&conn); // a restart
    assert!(
        BondTracker::why_it_fell(&conn, 5).is_empty(),
        "the old refusal and its why are gone"
    );
    assert_eq!(BondTracker::get_state(&conn).receipts, 1);
}

#[test]
fn a_receipt_that_ages_out_while_nothing_is_written_goes_at_startup() {
    let conn = conn();
    card(&conn, true, "notes", "create", false, "old");
    backdate(&conn, "old", 400.0);
    assert_eq!(
        BondTracker::get_state(&conn).receipts,
        1,
        "reading drops nothing"
    );
    BondTracker::ensure_tables(&conn);
    assert_eq!(BondTracker::get_state(&conn).receipts, 0);
}

#[test]
fn a_clock_a_little_behind_the_stamps_deletes_nothing_and_keeps_the_order_written() {
    let conn = conn();
    card(&conn, false, "notes", "create", false, "no");
    backdate(&conn, "no", -0.5); // stamped half a day ahead of the clock
    card(&conn, true, "notes", "create", false, "a1");
    BondTracker::ensure_tables(&conn);
    assert_eq!(
        BondTracker::get_state(&conn).receipts,
        2,
        "nothing within a day of now is dropped"
    );
    assert_eq!(
        bond_table::rows(&conn)[0].clean_streak,
        1,
        "and the order is the order written"
    );
}

#[test]
fn one_receipt_stamped_in_the_future_does_not_stop_retention() {
    let conn = conn();
    card(&conn, true, "notes", "create", false, "a");
    backdate(&conn, "a", -3650.0); // stamped ten years ahead, as after a bad clock
    card(&conn, true, "notes", "create", false, "b");
    backdate(&conn, "b", 181.0);
    card(&conn, true, "files", "move", false, "c");
    BondTracker::ensure_tables(&conn); // a restart
    let at = stamps(&conn);
    assert_eq!(at.len(), 1, "A (future) and B (181 days) are gone: {at:?}");
    assert!(
        (at[0] - now_ts()).abs() < 30.0,
        "C carries the real clock, not A's stamp"
    );
}
