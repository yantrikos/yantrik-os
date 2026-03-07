//! Serendipity instinct — surfaces unexpected memory connections.
//!
//! When the think cycle finds an interesting older memory, this instinct
//! converts the `serendipity` trigger into a proactive message that
//! connects the past memory to the present context.
//!
//! Only fires at Friend bond level or above (bond 3+).

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct SerendipityInstinct;

impl Instinct for SerendipityInstinct {
    fn name(&self) -> &str {
        "serendipity"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        state
            .pending_triggers
            .iter()
            .filter(|t| {
                t.get("trigger_type").and_then(|v| v.as_str()) == Some("serendipity")
            })
            .filter_map(|t| {
                let memory_text = t.get("memory_text").and_then(|v| v.as_str())?;
                if memory_text.len() < 10 {
                    return None;
                }

                // Truncate long memories
                let display = if memory_text.len() > 120 {
                    format!("{}...", &memory_text[..memory_text.floor_char_boundary(117)])
                } else {
                    memory_text.to_string()
                };

                let urgency = match state.bond_level {
                    BondLevel::Friend => 0.3,
                    BondLevel::Confidant => 0.35,
                    BondLevel::PartnerInCrime => 0.4,
                    _ => 0.25,
                };

                Some(
                    UrgeSpec::new(
                        "serendipity",
                        &format!("You mentioned before: \"{}\"", display),
                        urgency,
                    )
                    .with_cooldown("serendipity:connection")
                    .with_message(&format!(
                        "Something came to mind \u{2014} you once said: \"{}\"",
                        display
                    ))
                    .with_context(serde_json::json!({
                        "memory_text": memory_text,
                    })),
                )
            })
            .take(1) // Only one serendipity per cycle
            .collect()
    }
}
