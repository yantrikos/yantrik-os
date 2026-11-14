//! Clearing the draft a promise left in Email (#1184, round 3: HIGH-A).
//!
//! The prepared draft lives in Email's own slot, tagged with the commitment it came from. When the
//! person forgets the promise, or its item expires, the slot must not outlive it. A tool cannot
//! call another tool, so the end of a promise only *owes* a clear (a row in a small table); the
//! worker's housekeeping pays it by asking Email, through the same gated `app_action` door as every
//! other step, to forget the draft of that commitment, but only while Email is already running:
//! Email is never started just to pay a clear. Email clears it only if the draft is still the one
//! made for that commitment, the person has not made it their own, and they do not have it open.

use rusqlite::{params, Connection};
use serde_json::json;
use yantrik_companion_core::tools::{ToolContext, ToolRegistry};

use crate::anticipation::PreparedKind;
use crate::commitment_hands::Hands;

fn ensure(conn: &Connection) {
    let _ = conn.execute_batch("CREATE TABLE IF NOT EXISTS commitment_prep_slot_owed (commitment_id TEXT PRIMARY KEY)");
}

/// A clear is owed for this commitment's draft.
pub fn owe(conn: &Connection, commitment_id: &str) {
    ensure(conn);
    let _ = conn.execute("INSERT OR IGNORE INTO commitment_prep_slot_owed (commitment_id) VALUES (?1)", params![commitment_id]);
}

/// The clears owed.
pub fn owed(conn: &Connection) -> Vec<String> {
    ensure(conn);
    let Ok(mut stmt) = conn.prepare("SELECT commitment_id FROM commitment_prep_slot_owed ORDER BY commitment_id") else { return Vec::new() };
    stmt.query_map([], |r| r.get::<_, String>(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
}

fn paid(conn: &Connection, commitment_id: &str) {
    let _ = conn.execute("DELETE FROM commitment_prep_slot_owed WHERE commitment_id = ?1", params![commitment_id]);
}

/// Items past their expiry that made a draft which has not been cleared yet owe a clear.
pub fn owe_for_expired(conn: &Connection, now: f64) {
    let Ok(mut stmt) = conn.prepare("SELECT data FROM prepared_items WHERE kind = ?1 AND expires_at < ?2") else { return };
    let kind = PreparedKind::CommitmentPrep.as_str();
    let datas: Vec<String> = stmt.query_map(params![kind, now], |r| r.get::<_, String>(0)).map(|r| r.flatten().collect()).unwrap_or_default();
    for data in datas {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) else { continue };
        if v["draft"].is_object() && v["slot_cleared"] != true {
            if let Some(id) = v["commitment_id"].as_i64() {
                owe(conn, &id.to_string());
            }
        }
    }
}

/// The clear was paid: the item says so, so an expired item does not owe it again.
fn mark_cleared(conn: &Connection, commitment_id: &str) {
    let Ok(id) = commitment_id.parse::<i64>() else { return };
    let item = crate::commitment_prep::item_id(id);
    let Ok(data) = conn.query_row("SELECT data FROM prepared_items WHERE id = ?1", params![item], |r| r.get::<_, String>(0)) else { return };
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&data) else { return };
    v["slot_cleared"] = json!(true);
    let _ = conn.execute("UPDATE prepared_items SET data = ?1 WHERE id = ?2", params![v.to_string(), item]);
}

/// Pay what is owed: ask Email to forget each draft. A clear that cannot be done now stays owed; one
/// that cannot be done at all is dropped, for the person can still discard the draft themselves.
///
/// Email is never started to pay a clear: with `email_running` false nothing is asked and everything
/// stays owed until Email is next running. A running Email refuses while the person has the draft
/// open (`Failed::Later`), which also leaves it owed.
pub fn pay(registry: &ToolRegistry, ctx: &ToolContext<'_>, budget: std::time::Duration, email_running: bool) -> usize {
    if !email_running {
        return 0;
    }
    let owed = owed(&ctx.db.conn());
    let mut hands = Hands::new(registry, ctx, budget);
    let mut done = 0;
    for id in owed {
        match hands.act_unlisted("email", "forget_prepared", json!({ "commitment": id })) {
            Err(failed) if failed.is_later() => continue,
            _ => {}
        }
        mark_cleared(&ctx.db.conn(), &id);
        paid(&ctx.db.conn(), &id);
        done += 1;
    }
    done
}
