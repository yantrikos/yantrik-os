//! Activity Reflector instinct — surfaces multi-day behavioral patterns.
//!
//! At Friend level or above, this instinct observes `active_patterns` from
//! the companion state and surfaces interesting behavioral summaries. Unlike
//! pattern_surfacing (which is immediate), this instinct focuses on patterns
//! that have persisted across multiple days.
//!
//! Example: "Over the past 3 days, you've been using the terminal more than usual."

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};

pub struct ActivityReflectorInstinct;

impl Instinct for ActivityReflectorInstinct {
    fn name(&self) -> &str {
        "activity_reflector"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Only activate at Friend level or above
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        // Need at least a few patterns to surface something meaningful
        if state.active_patterns.len() < 2 {
            return vec![];
        }

        // Look for patterns that span multiple days
        let mut urges = Vec::new();

        for pattern in &state.active_patterns {
            let desc = match pattern.get("description").and_then(|v| v.as_str()) {
                Some(d) => d,
                None => continue,
            };

            // Check if pattern has multi-day context
            let days = pattern
                .get("days_observed")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            if days < 2 {
                continue;
            }

            let urgency = match state.bond_level {
                BondLevel::Friend => 0.35,
                BondLevel::Confidant => 0.4,
                BondLevel::PartnerInCrime => 0.45,
                _ => 0.3,
            };

            let mut context = serde_json::Map::new();
            context.insert(
                "activity_summary".into(),
                serde_json::Value::String(desc.to_string()),
            );
            context.insert(
                "pattern_description".into(),
                serde_json::Value::String(desc.to_string()),
            );
            context.insert(
                "days".into(),
                serde_json::Value::Number(serde_json::Number::from(days)),
            );

            urges.push(
                UrgeSpec::new(
                    "activity_reflector",
                    &format!("Multi-day pattern: {}", desc),
                    urgency,
                )
                .with_cooldown(&format!(
                    "activity_reflector:{}",
                    &desc[..desc.len().min(30)]
                ))
                .with_context(serde_json::Value::Object(context)),
            );

            break; // One reflection per cycle
        }

        urges
    }
}
