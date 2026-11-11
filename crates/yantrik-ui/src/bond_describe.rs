//! The bond table as `describe shell` publishes it: per app and action family, what was asked
//! and what the person answered, whether it still asks, and the one offer a clean streak earns.
//!
//! The rows come from the companion's receipts (`yantrik_companion::bond_table`), pushed here
//! whenever a receipt lands. This file only reads them back and says, from the session's own
//! rules, whether each row still asks. It decides nothing and applies nothing: an offer is
//! applied only when the person presses "Allow for this session" on a card, which is
//! `mind_mode::person_add_rule`. Nothing here reaches a tool permission or the machine ceiling,
//! and `leaves_machine` rows are never offered.

use std::sync::Mutex;

use yantrik_companion::bond::BondRow;

/// How many reasons the score fell are published.
pub const FELL_PUBLISHED: usize = 5;

#[derive(Default)]
struct Latest {
    rows: Vec<BondRow>,
    fell: Vec<String>,
}

static LATEST: Mutex<Latest> = Mutex::new(Latest { rows: Vec::new(), fell: Vec::new() });

fn latest() -> std::sync::MutexGuard<'static, Latest> {
    LATEST.lock().unwrap_or_else(|e| e.into_inner())
}

/// Keep what the store says now. Called from the companion thread after every receipt.
pub fn publish(rows: Vec<BondRow>, fell: Vec<String>) {
    let mut latest = latest();
    latest.rows = rows;
    latest.fell = fell;
}

/// One row as `describe shell` carries it. `allows` says whether the session already allows the
/// app and action, which is what "still asks" means.
pub(crate) fn row_json(row: &BondRow, allows: impl Fn(&str, &str) -> bool) -> serde_json::Value {
    let runs_unasked = allows(&row.app, &row.action);
    let mut line = row.line();
    if !runs_unasked {
        line.push_str(" · still asks");
    }
    serde_json::json!({
        "app": row.app,
        "action": row.action,
        "asked": row.asked,
        "allowed": row.allowed,
        "refused": row.refused,
        "receipts": row.receipts,
        "leaves_machine": row.leaves_machine,
        "still_asks": !runs_unasked,
        "line": line,
    })
}

/// The offers still worth making: rows that earned one and are not already allowed.
pub(crate) fn offers_json(rows: &[BondRow], allows: impl Fn(&str, &str) -> bool) -> Vec<serde_json::Value> {
    rows.iter()
        .filter(|row| !allows(&row.app, &row.action))
        .filter_map(|row| {
            row.offer().map(|text| {
                serde_json::json!({ "app": row.app, "action": row.action, "text": text })
            })
        })
        .collect()
}

/// What `describe shell` publishes under `bond_table`, `bond_offers` and `bond_why_it_fell`.
pub fn describe() -> (serde_json::Value, serde_json::Value, serde_json::Value) {
    let latest = latest();
    let allows = crate::mind_mode::session_allows;
    (
        latest.rows.iter().map(|row| row_json(row, allows)).collect::<Vec<_>>().into(),
        offers_json(&latest.rows, allows).into(),
        latest.fell.clone().into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_companion::bond::{BondTracker, Receipt, ReceiptKind};

    fn seeded() -> Vec<BondRow> {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        BondTracker::ensure_tables(&conn);
        let card = |kind, app: &str, action: &str, leaves, id: &str| {
            BondTracker::record_receipt(
                &conn,
                &Receipt { kind, app: app.into(), action: action.into(), leaves_machine: leaves, source_id: id.into() },
            );
        };
        for n in 0..4 {
            card(ReceiptKind::CardAllowed, "notes", "create", false, &format!("n{n}"));
        }
        card(ReceiptKind::CardAllowed, "email", "send", true, "e1");
        yantrik_companion::bond_table::rows(&conn)
    }

    #[test]
    fn a_row_says_whether_it_still_asks() {
        let rows = seeded();
        let none = |_: &str, _: &str| false;
        let lines: Vec<String> =
            rows.iter().map(|r| row_json(r, none)["line"].as_str().unwrap().to_string()).collect();
        assert_eq!(
            lines,
            vec!["Email · send: asked 1, allowed 1 · still asks", "Notes · create: asked 4, allowed 4 · still asks"]
        );
        let notes_allowed = |app: &str, action: &str| app == "notes" && action == "create";
        assert_eq!(row_json(&rows[1], notes_allowed)["line"], "Notes · create: asked 4, allowed 4");
        assert_eq!(row_json(&rows[1], notes_allowed)["still_asks"], false);
    }

    #[test]
    fn the_offer_goes_when_the_person_has_allowed_it_and_is_never_made_for_what_leaves() {
        let rows = seeded();
        let offers = offers_json(&rows, |_, _| false);
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0]["text"], "Notes create has been allowed 4 of 4 times. Stop asking for this session?");
        assert!(offers_json(&rows, |app, _| app == "notes").is_empty(), "already allowed: nothing to offer");
    }

    #[test]
    fn applying_an_offer_through_the_persons_rule_leaves_the_ceiling_where_it_was() {
        // The only way an offer takes effect is the person's "Allow for this session", which is
        // `person_add_rule`. It adds a rule for one app and action; the ceiling is the machine's
        // setting and the rule has no way to name it.
        let ceiling = crate::control_approvals::machine_ceiling();
        let mut modes = crate::mind_mode::Modes::default();
        modes.person_add_rule("notes", "create", "sensitive", "Make a note.").unwrap();
        assert_eq!(modes.rules().len(), 1);
        assert_eq!(crate::control_approvals::machine_ceiling(), ceiling);

        let own = include_str!("bond_describe.rs").split("#[cfg(test)]").next().unwrap();
        let receipts = include_str!("bond_receipts.rs");
        for (name, source) in [("bond_describe.rs", own), ("bond_receipts.rs", receipts)] {
            for word in ["max_permission", "tool_permission", "set_ceiling"] {
                let code: String =
                    source.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
                assert!(!code.contains(word), "{name} must not reach `{word}`");
            }
        }
    }
}
