//! The bond table: what each app was asked, for each family of action, and what the person
//! answered.
//!
//! Built from the card receipts in `bond_events`, the same rows the score is made of, so the
//! table and the score cannot disagree. A row reads like "Notes · create: asked 4, allowed 4".
//!
//! The table only reports, and offers. [`offers`] names the rows whose clean streak reached
//! [`OFFER_CLEAN_STREAK`]; it grants nothing. Only the person's press on a card applies one,
//! through the shell's `person_add_rule`, and nothing here reaches a tool permission or
//! `max_permission`.

use std::collections::BTreeMap;

use rusqlite::{params, Connection};

use crate::bond::{BondRow, ReceiptKind, RECEIPT_EVENT};

/// One card answer, read back from a receipt row.
struct Answer {
    allowed: bool,
    leaves_machine: bool,
    source_id: String,
}

/// Every (app, action) the person has answered a card for, as table rows sorted by app then
/// action.
pub fn rows(conn: &Connection) -> Vec<BondRow> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT json_extract(context, '$.kind'), json_extract(context, '$.app'),
                json_extract(context, '$.action'), json_extract(context, '$.leaves_machine'),
                json_extract(context, '$.source_id')
         FROM bond_events WHERE event_type = ?1 ORDER BY created_at, rowid",
    ) else {
        return Vec::new();
    };
    let Ok(found) = stmt.query_map(params![RECEIPT_EVENT], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<bool>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    }) else {
        return Vec::new();
    };

    let mut by_pair: BTreeMap<(String, String), Vec<Answer>> = BTreeMap::new();
    for (kind, app, action, leaves, source_id) in found.flatten() {
        let allowed = match kind.as_deref().and_then(ReceiptKind::parse) {
            Some(ReceiptKind::CardAllowed) => true,
            Some(ReceiptKind::CardRefused) => false,
            _ => continue,
        };
        let (Some(app), Some(action)) = (app, action) else { continue };
        by_pair.entry((app, action)).or_default().push(Answer {
            allowed,
            leaves_machine: leaves.unwrap_or(false),
            source_id: source_id.unwrap_or_default(),
        });
    }

    by_pair.into_iter().map(|((app, action), answers)| row_of(app, action, answers)).collect()
}

/// `answers` is oldest first.
fn row_of(app: String, action: String, answers: Vec<Answer>) -> BondRow {
    let allowed = answers.iter().filter(|a| a.allowed).count() as u32;
    let asked = answers.len() as u32;
    let clean_streak = answers.iter().rev().take_while(|a| a.allowed).count() as u32;
    // The newest word on whether it leaves the machine wins: an app can change what an action
    // does, and a row must never offer what now sends something away.
    let leaves_machine = answers.last().is_some_and(|a| a.leaves_machine);
    BondRow {
        app,
        action,
        asked,
        allowed,
        refused: asked - allowed,
        clean_streak,
        leaves_machine,
        receipts: answers.into_iter().rev().map(|a| a.source_id).collect(),
    }
}

/// The rows the companion may offer to stop asking about. An offer, never a grant.
pub fn offers(rows: &[BondRow]) -> Vec<&BondRow> {
    rows.iter().filter(|row| row.offerable()).collect()
}
