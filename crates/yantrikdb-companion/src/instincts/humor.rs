//! Humor instinct — at bond level 3+, occasionally suggests callbacks
//! to shared references or inside jokes.

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct HumorInstinct;

impl Instinct for HumorInstinct {
    fn name(&self) -> &str {
        "Humor"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Only activate at Friend level or above
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        let mut urges = Vec::new();

        // Surface shared references as potential callbacks
        if state.shared_references_count > 0 {
            let urgency = match state.bond_level {
                BondLevel::Friend => 0.3,
                BondLevel::Confidant => 0.4,
                BondLevel::PartnerInCrime => 0.5,
                _ => 0.2,
            };

            urges.push(
                UrgeSpec::new(
                    self.name(),
                    "You have inside jokes you could reference.",
                    urgency,
                )
                .with_cooldown("humor:callback"),
            );
        }

        // Self-deprecating humor opportunity at higher bond levels
        if state.bond_level >= BondLevel::Confidant {
            for pattern in &state.active_patterns {
                if let Some(desc) = pattern.get("description").and_then(|v| v.as_str()) {
                    let lower = desc.to_lowercase();
                    if lower.contains("repeat") || lower.contains("often") || lower.contains("always") {
                        urges.push(
                            UrgeSpec::new(
                                self.name(),
                                &format!("Playful self-observation: {desc}"),
                                0.35,
                            )
                            .with_cooldown(&format!(
                                "humor:pattern:{}",
                                &desc[..desc.len().min(20)]
                            )),
                        );
                        break; // One pattern joke per cycle
                    }
                }
            }
        }

        urges.into_iter().take(1).collect()
    }
}
