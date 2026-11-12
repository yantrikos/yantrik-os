//! Where the person's answers become bond receipts.
//!
//! The bond is scored only from what the person *did*: a card Allowed or refused, a For-you item
//! marked Useful or Not useful, something the machine did on its own that they undid. Each of
//! those already has a place it is decided; this is the one door they hand a receipt through, to
//! the companion thread that owns the store. Nothing here reads a word the person typed.
//!
//! It only writes receipts. It never grants anything: an offer to stop asking (`bond_describe`)
//! is applied by the person's own press on a card, through `mind_mode::person_add_rule`, and no
//! path from here reaches a tool permission or the machine ceiling. The one other thing a person
//! can do with the bond is forget it ([`forget`]), from a button.

use std::sync::{Arc, OnceLock};

use yantrik_companion::bond::{ActionFacts, Receipt, ReceiptKind};
use yantrik_ipc_contracts::control_surface::{fetches_address, leaves_machine};
use yantrik_ipc_contracts::notifications::notice::FeedbackKind;

use crate::approvals::{self, Card};
use crate::bridge::CompanionBridge;

static SINK: OnceLock<Arc<CompanionBridge>> = OnceLock::new();

/// Give the door the companion to send to. Called once, where the shell wires its approvals.
pub fn attach(bridge: Arc<CompanionBridge>) {
    let _ = SINK.set(bridge);
}

fn send(receipt: Receipt) {
    if let Some(bridge) = SINK.get() {
        bridge.bond_receipt(receipt);
    }
}

/// Wire the one callback that forgets, and give the door its companion. Called once, where the
/// shell wires its approvals.
pub fn wire(ui: &crate::App, bridge: Arc<CompanionBridge>) {
    attach(bridge);
    ui.on_forget_bond_history(forget);
}

/// The person pressed "forget the bond history". A UI callback and nothing else: it is not on the
/// control socket, so no Mind can call it, and it empties the receipts, the table and the offers.
pub fn forget() {
    if let Some(bridge) = SINK.get() {
        bridge.forget_bond_history();
    }
}

/// What the card says about its action, read the one way the gate reads it: the app's own
/// sentence, whole (`said`), never the caller's paraphrase.
pub(crate) fn card_facts(card: &Card) -> ActionFacts {
    ActionFacts {
        grade: card.grade.clone(),
        cannot_be_undone: approvals::unrecoverable(&card.said),
        open_ended: approvals::open_ended(&card.said),
        fetches_address: fetches_address(&card.said).is_some(),
        leaves_machine: leaves_machine(&card.said),
    }
}

/// The receipt for a person's answer to a card. Pure, so each way in is a test.
pub(crate) fn card_receipt(card: &Card, allowed: bool) -> Receipt {
    Receipt {
        kind: if allowed { ReceiptKind::CardAllowed } else { ReceiptKind::CardRefused },
        app: card.app.clone(),
        action: card.action.clone(),
        facts: card_facts(card),
        source_id: card.id.clone(),
    }
}

/// The person pressed Allow or Deny on a card. A card that expired unanswered, or was withdrawn
/// (`approvals::withdraw`), is neither and never comes through here.
pub fn card_answered(card: Option<&Card>, allowed: bool) {
    if let Some(card) = card {
        send(card_receipt(card, allowed));
    }
}

/// The receipt for feedback on a For-you item, or `None` for the kinds that say nothing about
/// whether it was useful ("not now" is about timing, "never" about the family).
pub(crate) fn for_you_receipt(id: &str, kind: &FeedbackKind) -> Option<Receipt> {
    let kind = match kind {
        FeedbackKind::Useful => ReceiptKind::ForYouUseful,
        FeedbackKind::NotUseful => ReceiptKind::ForYouNotUseful,
        _ => return None,
    };
    Some(Receipt::bare(kind, id))
}

/// The person pressed Useful or Not useful on a For-you item.
pub fn for_you_answered(id: &str, kind: &FeedbackKind) {
    if let Some(receipt) = for_you_receipt(id, kind) {
        send(receipt);
    }
}

/// The receipt for an undo. It has no record of its own, so it carries no id and the store gives
/// it a fresh one: pressing Undo on the next thing the machine did is another receipt, however
/// alike the two are.
pub(crate) fn undo_receipt(app: &str, action: &str) -> Receipt {
    Receipt {
        kind: ReceiptKind::Undone,
        app: app.to_string(),
        action: action.to_string(),
        facts: ActionFacts::default(),
        source_id: String::new(),
    }
}

/// The person undid something the machine did on its own: a wired model, a Mind's archive or
/// trash. Never the person's own archive or trash, which is theirs.
pub fn undone(app: &str, action: &str) {
    send(undo_receipt(app, action));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::{Store, Verified};
    use std::time::Instant;

    fn raise(store: &mut Store, app: &str, action: &str, purpose: &str) -> Card {
        let now = Instant::now();
        let id = store
            .request("m", Verified::default(), app, action, serde_json::json!({}), "sensitive", purpose, "", "", now, "12:00")
            .unwrap()
            .id;
        store.cards(now).into_iter().find(|c| c.id == id).unwrap()
    }

    #[test]
    fn a_card_after_a_restart_has_an_id_no_earlier_boot_used() {
        let (mut first, mut second) = (Store::with_boot("a1"), Store::with_boot("b2"));
        let before = raise(&mut first, "notes", "create", "Make a note.");
        let after = raise(&mut second, "notes", "create", "Make a note.");
        assert_ne!(before.id, after.id, "both boots count from 1; the ids must still differ");
        assert_ne!(card_receipt(&before, true).source_id, card_receipt(&after, false).source_id);
    }

    #[test]
    fn the_receipt_carries_what_the_app_said_about_the_action() {
        let mut store = Store::with_boot("t");
        let gone = raise(&mut store, "calendar", "delete_event", "Delete an event. It is not recoverable.");
        let facts = card_receipt(&gone, true).facts;
        assert!(facts.cannot_be_undone && !facts.leaves_machine);
        assert_eq!(facts.grade, "sensitive");
        let plain = raise(&mut store, "notes", "create", "Make a note.");
        assert_eq!(card_receipt(&plain, true).facts, ActionFacts { grade: "sensitive".into(), ..Default::default() });
    }

    #[test]
    fn an_undo_has_no_id_of_its_own_so_each_one_counts() {
        assert!(undo_receipt("models", "wire_harness").source_id.is_empty());
        assert_eq!(undo_receipt("desktop", "archive").kind, ReceiptKind::Undone);
    }

    #[test]
    fn only_useful_and_not_useful_are_receipts() {
        assert!(for_you_receipt("n1", &FeedbackKind::Useful).is_some());
        assert!(for_you_receipt("n1", &FeedbackKind::NotUseful).is_some());
        assert!(for_you_receipt("n1", &FeedbackKind::NotNow).is_none());
    }
}
