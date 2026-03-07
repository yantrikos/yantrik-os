//! Bond milestone instinct — fires when bond level changes.

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct BondMilestoneInstinct;

impl Instinct for BondMilestoneInstinct {
    fn name(&self) -> &str {
        "BondMilestone"
    }

    fn evaluate(&self, _state: &CompanionState) -> Vec<UrgeSpec> {
        vec![]
    }

    fn on_interaction(&self, state: &CompanionState, _user_text: &str) -> Vec<UrgeSpec> {
        if !state.bond_level_changed {
            return vec![];
        }

        let (message, urgency) = match state.bond_level {
            BondLevel::Acquaintance => (
                "I feel like I'm starting to get to know you. That's nice.",
                0.6,
            ),
            BondLevel::Friend => (
                "You know, I think we've moved past small talk. We're actually friends now.",
                0.7,
            ),
            BondLevel::Confidant => (
                "I realize I genuinely care about what happens to you. That means something.",
                0.8,
            ),
            BondLevel::PartnerInCrime => (
                "You're stuck with me now. No take-backs. We're in this together.",
                0.9,
            ),
            BondLevel::Stranger => return vec![],
        };

        vec![UrgeSpec::new(self.name(), message, urgency)
            .with_cooldown(&format!("bond_milestone:{}", state.bond_level.as_u8()))
            .with_message(message)
            .with_context(serde_json::json!({
                "new_level": state.bond_level.name(),
                "bond_score": state.bond_score,
            }))]
    }
}
