//! The bond table: what each app was asked, for each family of action, and what the person
//! answered.
//!
//! Built from the card receipts in `bond_events`, the same rows the score is made of, so the
//! table and the score cannot disagree. A row reads like "Notes · create: answered 4, allowed 4".
//!
//! The table only reports, and offers. [`offers`] names the rows whose clean streak reached
//! [`OFFER_CLEAN_STREAK`]; it grants nothing. Only the person's press on a card applies one,
//! through the shell's `person_add_rule`, and nothing here reaches a tool permission or
//! `max_permission`.

use std::collections::BTreeMap;

use rusqlite::{params, Connection};

use crate::bond::{ActionFacts, BondRow, ReceiptKind, RECEIPT_EVENT};

/// One card answer, read back from a receipt row.
struct Answer {
    allowed: bool,
    facts: ActionFacts,
    source_id: String,
}

/// Every (app, action) the person has answered a card for, as table rows sorted by app then
/// action.
pub fn rows(conn: &Connection) -> Vec<BondRow> {
    // In the order written (`rowid`), never the wall clock: a clock stepped back must not move a
    // refusal ahead of older allows and so restore a streak.
    let Ok(mut stmt) = conn.prepare(
        "SELECT json_extract(context, '$.kind'), json_extract(context, '$.app'),
                json_extract(context, '$.action'), json_extract(context, '$.source_id'),
                json_extract(context, '$.grade'), json_extract(context, '$.cannot_be_undone'),
                json_extract(context, '$.open_ended'), json_extract(context, '$.fetches_address'),
                json_extract(context, '$.leaves_machine')
         FROM bond_events WHERE event_type = ?1 ORDER BY rowid",
    ) else {
        return Vec::new();
    };
    let Ok(found) = stmt.query_map(params![RECEIPT_EVENT], |row| {
        let flag = |at: usize| row.get::<_, Option<bool>>(at).map(Option::unwrap_or_default);
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            ActionFacts {
                grade: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                cannot_be_undone: flag(5)?,
                open_ended: flag(6)?,
                fetches_address: flag(7)?,
                leaves_machine: flag(8)?,
            },
        ))
    }) else {
        return Vec::new();
    };

    let mut by_pair: BTreeMap<(String, String), Vec<Answer>> = BTreeMap::new();
    for (kind, app, action, source_id, facts) in found.flatten() {
        let allowed = match kind.as_deref().and_then(ReceiptKind::parse) {
            Some(ReceiptKind::CardAllowed) => true,
            Some(ReceiptKind::CardRefused) => false,
            _ => continue,
        };
        let (Some(app), Some(action)) = (app, action) else { continue };
        by_pair.entry((app, action)).or_default().push(Answer {
            allowed,
            facts,
            source_id: source_id.unwrap_or_default(),
        });
    }

    by_pair.into_iter().map(|((app, action), answers)| row_of(app, action, answers)).collect()
}

/// `answers` is oldest first.
fn row_of(app: String, action: String, answers: Vec<Answer>) -> BondRow {
    let allowed = answers.iter().filter(|a| a.allowed).count() as u32;
    let answered = answers.len() as u32;
    let clean_streak = answers.iter().rev().take_while(|a| a.allowed).count() as u32;
    // The newest card is what the gate would read for this action now; the cautious union of
    // every card is what an offer is held to, so an app that drops a sentence for a moment does
    // not make a row that once said "leaves the machine" offerable.
    let facts = answers.last().map(|a| a.facts.clone()).unwrap_or_default();
    let mut ever = ActionFacts::default();
    for answer in &answers {
        ever.absorb(&answer.facts);
    }
    BondRow {
        app,
        action,
        answered,
        allowed,
        refused: answered - allowed,
        clean_streak,
        facts,
        ever,
        receipts: answers.into_iter().rev().map(|a| a.source_id).collect(),
    }
}

/// The rows the companion may offer to stop asking about. An offer, never a grant.
/// `may_offer_session_rule` is the shell's own test (`approvals::session_rule_allowed`).
pub fn offers(rows: &[BondRow], may_offer_session_rule: impl Fn(&str, bool) -> bool + Copy) -> Vec<&BondRow> {
    rows.iter().filter(|row| row.offerable(may_offer_session_rule)).collect()
}
