//! What the commitment executor reads and keeps in the database (#1184): the commitments in or
//! near their prep window, one commitment by id, forgetting one (and what was prepared from it),
//! and the two answers the person gives about preparing at all.
//!
//! The two answers are kept in one small table:
//! - `enabled`, the switch for the instinct: on unless the person turned it off;
//! - `consent`, whether the person said yes to "Shall I get these ready for promises like this?".
//!   It is earned, never assumed: until it is `yes`, a promise is only recorded as what would be
//!   prepared, and nothing is opened or drafted.

use rusqlite::{params, Connection};

use crate::anticipation::AnticipationEngine;
use crate::world_model::{Commitment, WorldModel};

/// The question the first prepared item asks, once.
pub const QUESTION: &str = "Shall I get these ready for promises like this?";

const ENABLED: &str = "enabled";
const CONSENT: &str = "consent";
const ASKED: &str = "asked";
const ASKING: &str = "asking";
const ANSWERED_BY: &str = "answered_by";
const ANSWERED_ITEM: &str = "answered_item";

/// Commitments whose deadline falls within `hours` of `now` and has not passed.
pub fn approaching_at(conn: &Connection, hours: f64, now: f64) -> Vec<Commitment> {
    let cutoff = now + hours * 3600.0;
    WorldModel::query_commitments_params(
        conn,
        "WHERE status IN ('pending', 'in_progress')
         AND deadline > 0 AND deadline <= ?1 AND deadline > ?2
         ORDER BY deadline ASC",
        params![cutoff, now],
    )
}

/// One commitment by id, whatever its status.
pub fn commitment(conn: &Connection, id: i64) -> Option<Commitment> {
    WorldModel::query_commitments_params(conn, "WHERE id = ?1", params![id]).into_iter().next()
}

/// Forget a commitment: it is deleted, and so is the item prepared from it. `true` if it was there.
pub fn forget(conn: &Connection, id: i64) -> bool {
    // The draft it made in Email goes with it: the clear is owed, and paid by the worker.
    if AnticipationEngine::for_commitment(conn, &id.to_string()).is_some_and(|i| i.data["draft"].is_object()) {
        crate::commitment_slot::owe(conn, &id.to_string());
    }
    AnticipationEngine::delete_for_commitment(conn, &id.to_string());
    conn.execute("DELETE FROM wm_commitments WHERE id = ?1", params![id]).unwrap_or(0) > 0
}

fn ensure(conn: &Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS commitment_prep_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    );
}

fn get(conn: &Connection, key: &str) -> Option<String> {
    ensure(conn);
    conn.query_row("SELECT value FROM commitment_prep_settings WHERE key = ?1", params![key], |r| r.get(0)).ok()
}

fn set(conn: &Connection, key: &str, value: &str) {
    ensure(conn);
    let _ = conn.execute(
        "INSERT INTO commitment_prep_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    );
}

/// The switch: preparing is on unless the person turned it off.
pub fn enabled(conn: &Connection) -> bool {
    get(conn, ENABLED).as_deref() != Some("off")
}

/// The person said yes to having promises prepared.
pub fn consented(conn: &Connection) -> bool {
    get(conn, CONSENT).as_deref() == Some("yes")
}

/// Refusing is safe for anyone to do, a model turn included: nothing is opened or drafted. It is
/// the only answer the model-callable tool gives; the yes is `commitment_consent::person_answer`.
pub fn refuse(conn: &Connection) {
    set(conn, CONSENT, "no");
    set(conn, ENABLED, "off");
    // Not the person's choice: the brief offers the way back, which it does not after their own No.
    set(conn, ANSWERED_BY, "model");
    set(conn, ANSWERED_ITEM, "");
    remove(conn, ASKING);
}

/// The person's answer, with who gave it and which item's question it answered. Only
/// `commitment_consent` calls this.
pub(crate) fn record_answer(conn: &Connection, yes: bool, by: &str, item: &str) {
    set(conn, CONSENT, if yes { "yes" } else { "no" });
    set(conn, ENABLED, if yes { "on" } else { "off" });
    set(conn, ANSWERED_BY, by);
    set(conn, ANSWERED_ITEM, item);
    remove(conn, ASKING);
}

/// Who gave the answer, and to which item's question.
pub fn answered(conn: &Connection) -> Option<(String, String)> {
    Some((get(conn, ANSWERED_BY)?, get(conn, ANSWERED_ITEM)?))
}

/// The item whose question is outstanding: it was shown to the person and not yet answered.
pub fn asking(conn: &Connection) -> Option<String> {
    get(conn, ASKING)
}

/// Whether the question has been put to the person already: shown, not stored.
pub fn asked(conn: &Connection) -> bool {
    get(conn, ASKED).is_some()
}

/// The question of `item` was shown to the person.
pub(crate) fn mark_asked(conn: &Connection, item: &str) {
    set(conn, ASKED, "yes");
    set(conn, ASKING, item);
}

fn remove(conn: &Connection, key: &str) {
    ensure(conn);
    let _ = conn.execute("DELETE FROM commitment_prep_settings WHERE key = ?1", params![key]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anticipation::{PreparationTrigger, PreparedItem, PreparedKind};

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        WorldModel::ensure_tables(&conn);
        AnticipationEngine::ensure_table(&conn);
        conn
    }

    #[test]
    fn preparing_is_on_until_switched_off_and_consent_is_never_assumed() {
        let conn = db();
        assert!(enabled(&conn));
        assert!(!consented(&conn), "never assumed");
        assert!(!asked(&conn));
        mark_asked(&conn, "commitment-prep-1");
        assert!(asked(&conn));
        record_answer(&conn, true, "person", "commitment-prep-1");
        assert!(consented(&conn));
        assert_eq!(answered(&conn), Some(("person".into(), "commitment-prep-1".into())));
        refuse(&conn);
        assert!(!enabled(&conn) && !consented(&conn));
    }

    #[test]
    fn forgetting_a_commitment_deletes_its_item_too() {
        let conn = db();
        let item = PreparedItem {
            id: "commitment-prep-7".into(),
            trigger: PreparationTrigger::CommitmentDeadline { commitment_id: "7".into() },
            kind: PreparedKind::CommitmentPrep,
            description: "x".into(),
            data: serde_json::json!({}),
            relevant_at: 0.0,
            expires_at: 1e12,
            surfaced: false,
            outcome: None,
            created_at: 0.0,
        };
        AnticipationEngine::store(&conn, &item);
        assert!(AnticipationEngine::for_commitment(&conn, "7").is_some());
        forget(&conn, 7);
        assert!(AnticipationEngine::for_commitment(&conn, "7").is_none());
    }
}
