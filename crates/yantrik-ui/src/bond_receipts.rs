//! Where the person's answers become bond receipts.
//!
//! The bond is scored only from what the person *did*: a card Allowed or refused, a For-you item
//! marked Useful or Not useful, something the machine did that was undone. Each of those already
//! has a place it is decided; this is the one door they hand a receipt through, to the companion
//! thread that owns the store. Nothing here reads a word the person typed.
//!
//! It only writes receipts. It never grants anything: an offer to stop asking (`bond_describe`)
//! is applied by the person's own press on a card, through `mind_mode::person_add_rule`, and no
//! path from here reaches a tool permission or the machine ceiling.

use std::sync::{Arc, OnceLock};

use yantrik_companion::bond::{Receipt, ReceiptKind};
use yantrik_ipc_contracts::control_surface::leaves_machine;
use yantrik_ipc_contracts::notifications::notice::FeedbackKind;

use crate::approvals::Card;
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

/// The receipt for a person's answer to a card. Pure, so each way in is a test.
pub(crate) fn card_receipt(card: &Card, allowed: bool) -> Receipt {
    Receipt {
        kind: if allowed { ReceiptKind::CardAllowed } else { ReceiptKind::CardRefused },
        app: card.app.clone(),
        action: card.action.clone(),
        // The app's own sentence, whole: `said`, never the caller's paraphrase.
        leaves_machine: leaves_machine(&card.said),
        source_id: card.id.clone(),
    }
}

/// The person pressed Allow or Deny on a card. A card that expired unanswered is neither.
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
    Some(Receipt {
        kind,
        app: String::new(),
        action: String::new(),
        leaves_machine: false,
        source_id: id.to_string(),
    })
}

/// The person pressed Useful or Not useful on a For-you item.
pub fn for_you_answered(id: &str, kind: &FeedbackKind) {
    if let Some(receipt) = for_you_receipt(id, kind) {
        send(receipt);
    }
}

/// The person undid something the machine did.
pub fn undone(what: &str, app: &str, action: &str) {
    send(Receipt {
        kind: ReceiptKind::Undone,
        app: app.to_string(),
        action: action.to_string(),
        leaves_machine: false,
        source_id: what.to_string(),
    });
}
