//! The bond table as the shell publishes it: per app and action family, what was asked and what
//! the person answered, whether it still asks, and the one offer a clean streak earns.
//!
//! The rows come from the companion's receipts (`yantrik_companion::bond_table`), pushed here
//! whenever a receipt lands. This file only reads them back and says, from the gate's own
//! decision, whether each row still asks. It decides nothing and applies nothing: an offer is
//! applied only when the person presses "Allow for this session" on a card, which is
//! `mind_mode::person_add_rule`. Nothing here reaches a tool permission or the machine ceiling.
//!
//! Two readers, two views. A Mind reading `describe shell` gets [`table_for_minds`]: counts and
//! whether each row still asks, nothing it could time or phrase an ask by. The person's own
//! desktop gets [`desktop_view`]: the receipt ids, the offers, and the sentences that say why
//! trust fell.

use std::sync::Mutex;

use yantrik_companion::bond::BondRow;

use crate::mind_mode::{Declared, Decision};

/// How many reasons the score fell are published to the desktop.
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

/// What the gate would do with this row's action now: the same decision, from the same table
/// (`mind_mode::decide`), with the facts the card carried. An action with no recorded grade
/// reads as asking.
fn gate_says(row: &BondRow) -> Decision {
    if row.facts.grade.is_empty() {
        return Decision::Ask;
    }
    let declared = Declared {
        cannot_be_undone: row.facts.cannot_be_undone,
        open_ended: row.facts.open_ended,
    };
    crate::mind_mode::decide(&row.facts.grade, &row.app, &row.action, declared)
}

/// One row, given what the gate says about it. `with_receipts` is for the desktop only.
pub(crate) fn row_json(row: &BondRow, gate: &Decision, with_receipts: bool) -> serde_json::Value {
    let (posture, suffix) = match gate {
        Decision::Ask => ("asks", " · still asks"),
        Decision::Run { .. } => ("runs", ""),
        Decision::Refuse { .. } => ("refused", " · not run in this mode"),
    };
    let mut json = serde_json::json!({
        "app": row.app,
        "action": row.action,
        "answered": row.answered,
        "allowed": row.allowed,
        "refused": row.refused,
        "leaves_machine": row.ever.leaves_machine,
        "still_asks": posture == "asks",
        "posture": posture,
        "line": format!("{}{suffix}", row.line()),
    });
    if with_receipts {
        json["receipts"] = row.receipts.clone().into();
    }
    json
}

/// The offers worth making: rows that earned one (`BondRow::offer`, held to the same test as the
/// "Allow for this session" button) and that the gate would still ask about. In a mode that
/// already runs the action, or refuses it, there is nothing to stop asking about.
pub(crate) fn offers_json(rows: &[BondRow], gate: impl Fn(&BondRow) -> Decision) -> Vec<serde_json::Value> {
    rows.iter()
        .filter(|row| gate(row) == Decision::Ask)
        .filter_map(|row| {
            row.offer(crate::approvals_bond::session_rule_allowed)
                .map(|text| serde_json::json!({ "app": row.app, "action": row.action, "text": text }))
        })
        .collect()
}

/// What `describe shell` publishes under `bond_table`: for a Mind, which reads it as it would
/// anything else on the surface.
pub fn table_for_minds() -> serde_json::Value {
    let latest = latest();
    latest.rows.iter().map(|row| row_json(row, &gate_says(row), false)).collect::<Vec<_>>().into()
}

/// What the desktop's own screens read: the table with its receipt ids, the offers, and why the
/// score last fell. Never published on the control surface. The Bond screen is a later issue;
/// until it lands nothing draws this.
#[allow(dead_code)]
pub fn desktop_view() -> serde_json::Value {
    let latest = latest();
    serde_json::json!({
        "table": latest.rows.iter().map(|row| row_json(row, &gate_says(row), true)).collect::<Vec<_>>(),
        "offers": offers_json(&latest.rows, gate_says),
        "why_it_fell": latest.fell,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_companion::bond::{ActionFacts, BondTracker, Receipt, ReceiptKind};

    fn seeded() -> Vec<BondRow> {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        BondTracker::ensure_tables(&conn);
        let card = |app: &str, action: &str, facts: ActionFacts, id: &str| {
            BondTracker::record_receipt(
                &conn,
                &Receipt { kind: ReceiptKind::CardAllowed, app: app.into(), action: action.into(), facts, source_id: id.into() },
            );
        };
        let sensitive = || ActionFacts { grade: "sensitive".into(), ..Default::default() };
        for n in 0..4 {
            card("notes", "create", sensitive(), &format!("n{n}"));
        }
        card("email", "send", ActionFacts { leaves_machine: true, ..sensitive() }, "e1");
        for n in 0..4 {
            card("calendar", "delete_event", ActionFacts { cannot_be_undone: true, ..sensitive() }, &format!("d{n}"));
        }
        BondTracker::record_receipt(&conn, &Receipt::bare(ReceiptKind::Undone, ""));
        yantrik_companion::bond_table::rows(&conn)
    }

    fn row<'a>(rows: &'a [BondRow], app: &str) -> &'a BondRow {
        rows.iter().find(|r| r.app == app).unwrap()
    }

    /// The gate in each mode, as `Modes::decide` answers it: ask runs nothing unasked, auto runs
    /// the routine sensitive things, plan refuses.
    fn auto_runs_notes(row: &BondRow) -> Decision {
        if row.app == "notes" {
            Decision::Run { unasked: true }
        } else {
            Decision::Ask
        }
    }

    #[test]
    fn a_row_says_what_the_gate_says_it_does() {
        let rows = seeded();
        let notes = row(&rows, "notes");
        assert_eq!(row_json(notes, &Decision::Ask, false)["line"], "Notes · create: answered 4, allowed 4 · still asks");
        let auto = row_json(notes, &auto_runs_notes(notes), false);
        assert_eq!(auto["line"], "Notes · create: answered 4, allowed 4", "auto runs it unasked, so it does not still ask");
        assert_eq!(auto["still_asks"], false);
        let plan = row_json(notes, &Decision::Refuse { why: "plan".into() }, false);
        assert_eq!(plan["posture"], "refused");
        assert_eq!(plan["still_asks"], false);
    }

    #[test]
    fn the_real_gate_is_the_one_asked() {
        // `gate_says` is `mind_mode::decide`: in the process-wide default mode a sensitive action
        // asks, and a row with no grade is read as asking rather than refused.
        let rows = seeded();
        let mut bare = row(&rows, "notes").clone();
        bare.facts.grade.clear();
        assert_eq!(gate_says(&bare), Decision::Ask);
    }

    #[test]
    fn the_offer_goes_when_the_mode_already_runs_it_and_is_never_made_for_what_cannot_take_it() {
        let rows = seeded();
        let asks = |_: &BondRow| Decision::Ask;
        let offers = offers_json(&rows, asks);
        assert_eq!(offers.len(), 1, "notes only: email leaves the machine, delete_event cannot be undone");
        assert_eq!(offers[0]["text"], "Notes create has been allowed the last 4 times. Stop asking for this session?");
        assert!(offers_json(&rows, auto_runs_notes).is_empty(), "auto already runs it: nothing to offer");
        assert!(offers_json(&rows, |_| Decision::Refuse { why: String::new() }).is_empty());
    }

    #[test]
    fn a_mind_is_told_counts_and_never_ids_offers_or_reasons() {
        let rows = seeded();
        let for_minds = row_json(row(&rows, "notes"), &Decision::Ask, false);
        assert!(for_minds.get("receipts").is_none());
        assert!(for_minds["line"].as_str().unwrap().contains("answered 4"));
        let for_desktop = row_json(row(&rows, "notes"), &Decision::Ask, true);
        assert_eq!(for_desktop["receipts"].as_array().unwrap().len(), 4);

        let code = yantrik_companion::code_only(include_str!("control.rs"));
        for word in ["bond_offers", "bond_why_it_fell", "bond_score", "desktop_view", "forget"] {
            assert!(!code.contains(word), "`describe shell` must not publish or reach `{word}`");
        }
    }

    #[test]
    fn applying_an_offer_through_the_persons_rule_leaves_the_ceiling_where_it_was() {
        // The only way an offer takes effect is the person's "Allow for this session", which is
        // `person_add_rule`. It adds a rule for one app and action; the ceiling is the machine's
        // setting and the rule has no way to name it. The decision above it still refuses what
        // is over the ceiling, with the rule in place.
        let ceiling = crate::control_approvals::machine_ceiling();
        let mut modes = crate::mind_mode::Modes::default();
        modes.person_add_rule("notes", "create", "sensitive", "Make a note.").unwrap();
        assert_eq!(modes.rules().len(), 1);
        assert_eq!(crate::control_approvals::machine_ceiling(), ceiling);
        let past = modes.decide("sensitive", "notes", "create", false, "safe", std::time::Instant::now());
        assert!(matches!(past, Decision::Refuse { .. }), "a rule does not lift the ceiling: {past:?}");

        for (name, source) in [("bond_describe.rs", include_str!("bond_describe.rs")), ("bond_receipts.rs", include_str!("bond_receipts.rs"))] {
            let code = yantrik_companion::code_only(source);
            for word in ["max_permission", "tool_permission", "set_ceiling"] {
                assert!(!code.contains(word), "{name} must not reach `{word}`");
            }
        }
    }

    /// The person's "Forget the bond history" is a button on the Interaction history screen, and
    /// the only caller of the callback is that button's `clicked` handler (after its confirm).
    #[test]
    fn the_forget_button_has_exactly_one_caller_and_it_is_a_click() {
        let screen = include_str!("../../yantrik-ui-slint/ui/bond.slint");
        let calls: Vec<usize> = screen.match_indices("root.forget-bond-history()").map(|(at, _)| at).collect();
        assert_eq!(calls.len(), 1, "bond.slint calls forget-bond-history exactly once");
        let before = &screen[..calls[0]];
        let click = before.rfind("clicked => {").expect("the call sits in a `clicked` handler");
        assert!(!before[click..].contains('}'), "and in that handler's own block, not after it");
        assert!(screen.contains("It can't be undone."), "the confirm says what it does");
        assert!(screen.contains("Forget the bond history"), "and the button is drawn");

        // App forwards the screen's callback to the Rust handler, in its own handler, once.
        let app = include_str!("../../yantrik-ui-slint/ui/app.slint");
        assert_eq!(app.matches("root.forget-bond-history()").count(), 1);
        assert!(app.contains("forget-bond-history => { root.forget-bond-history(); }"));
        // Rust: one handler, and nothing ever presses it.
        assert_eq!(include_str!("bond_receipts.rs").matches("on_forget_bond_history").count(), 1);
        for (name, source) in [("control.rs", include_str!("control.rs")), ("bridge.rs", include_str!("bridge.rs"))] {
            assert!(!source.contains("invoke_forget_bond_history"), "{name} must not press the button");
        }
    }
}
