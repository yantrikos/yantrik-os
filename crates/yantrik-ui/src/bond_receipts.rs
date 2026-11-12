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
use yantrik_companion::CompanionService;
use yantrik_ipc_contracts::control_surface::{fetches_address, leaves_machine};
use yantrik_ipc_contracts::notifications::notice::FeedbackKind;

use crate::approvals::{self, Card};
use crate::bridge::CompanionBridge;

static SINK: OnceLock<Arc<CompanionBridge>> = OnceLock::new();

/// Give the door the companion to send to. Called once, where the shell wires its approvals.
pub fn attach(bridge: Arc<CompanionBridge>) {
    let _ = SINK.set(bridge);
}

/// What the person can do to the bond: add a receipt, or forget them all. One command, so the
/// bridge carries one variant for both (`CompanionCommand::Bond`).
pub enum BondCommand {
    Receipt(Receipt),
    /// "Forget the bond history": the person's button only (see [`forget`]).
    Forget,
}

impl BondCommand {
    /// Run it on the companion's own thread, where the store lives.
    pub fn apply(self, companion: &mut CompanionService) {
        match self {
            Self::Receipt(receipt) => companion.record_bond_receipt(&receipt),
            Self::Forget => {
                companion.forget_bond_history();
            }
        }
    }
}

fn send(receipt: Receipt) {
    if let Some(bridge) = SINK.get() {
        bridge.bond_command(BondCommand::Receipt(receipt));
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
        bridge.bond_command(BondCommand::Forget);
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
        no_session_rule: !card.can_session,
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
/// (`approvals_bond::withdraw`), is neither and never comes through here.
pub fn card_answered(card: Option<&Card>, allowed: bool) {
    if let Some(card) = card {
        send(card_receipt(card, allowed));
    }
}

/// The receipt for feedback on a For-you item, or `None` for the kinds that say nothing about
/// whether it was useful ("not now" is about timing, "never" about the family).
///
/// It is filed under the notice's sender harness (`harness`, empty when unknown), so one Mind
/// posting many notices that a person habitually marks Useful is held to the same per-source
/// cap as one card raised over and over ([`yantrik_companion::bond::PAIR_RISE_CAP`]).
pub(crate) fn for_you_receipt(id: &str, harness: &str, kind: &FeedbackKind) -> Option<Receipt> {
    let kind = match kind {
        FeedbackKind::Useful => ReceiptKind::ForYouUseful,
        FeedbackKind::NotUseful => ReceiptKind::ForYouNotUseful,
        _ => return None,
    };
    let mut receipt = Receipt::bare(kind, id);
    receipt.app = if harness.is_empty() { "unknown-harness".to_string() } else { harness.to_string() };
    receipt.action = "for_you".to_string();
    Some(receipt)
}

/// The person pressed Useful or Not useful on a For-you item.
/// `harness` is the id of the harness that sent the notice.
pub fn for_you_answered(id: &str, harness: &str, kind: &FeedbackKind) {
    if let Some(receipt) = for_you_receipt(id, harness, kind) {
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
    use crate::approvals_bond;
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
    fn a_card_id_carries_the_boot_so_a_restart_cannot_repeat_one() {
        // Both boots count from 1; only the boot part of the id tells them apart.
        let mut store = Store::new();
        let card = raise(&mut store, "notes", "create", "Make a note.");
        assert_eq!(card.id, format!("appr-{}-1", approvals_bond::boot()));
        assert_ne!(approvals_bond::fresh_boot(), approvals_bond::boot());
        let earlier = format!("appr-{}-1", approvals_bond::fresh_boot());
        assert_ne!(card_receipt(&card, true).source_id, earlier);
    }

    #[test]
    fn a_card_without_the_session_button_says_so_in_its_receipt() {
        let (mut store, now) = (Store::new(), Instant::now());
        let id = store
            .request("m", Verified::default(), "notes", "create", serde_json::json!({}), "sensitive", "Make a note.", "", "Because of this one", now, "12:00")
            .unwrap()
            .id;
        let explained = store.cards(now).into_iter().find(|c| c.id == id).unwrap();
        assert!(!explained.can_session, "the premise: a per-call sentence closes the standing yes");
        assert!(card_receipt(&explained, true).facts.no_session_rule);
        let plain = raise(&mut Store::new(), "notes", "create", "Make a note.");
        assert!(plain.can_session, "the premise: an ordinary card draws the session button");
        assert!(!card_receipt(&plain, true).facts.no_session_rule);
    }

    #[test]
    fn the_receipt_carries_what_the_app_said_about_the_action() {
        let mut store = Store::new();
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
        assert!(for_you_receipt("n1", "mind-a", &FeedbackKind::Useful).is_some());
        assert!(for_you_receipt("n1", "mind-a", &FeedbackKind::NotUseful).is_some());
        assert!(for_you_receipt("n1", "mind-a", &FeedbackKind::NotNow).is_none());
    }

    #[test]
    fn a_for_you_receipt_belongs_to_its_senders_harness_so_it_is_capped() {
        let one = for_you_receipt("n1", "mind-a", &FeedbackKind::Useful).unwrap();
        assert_eq!((one.app.as_str(), one.action.as_str()), ("mind-a", "for_you"));
        let unknown = for_you_receipt("n2", "", &FeedbackKind::Useful).unwrap();
        assert!(!unknown.app.is_empty(), "an unknown sender is still one source, never uncapped");
        // Many notices from one harness, all marked Useful: the fold stops at the cap.
        let mut fold = yantrik_companion::bond::ScoreFold::default();
        for _ in 0..1000 {
            fold.add(one.kind.delta(), &one.app, &one.action);
        }
        assert!(fold.finish().0 <= yantrik_companion::bond::PAIR_RISE_CAP + 1e-9);
    }
}
