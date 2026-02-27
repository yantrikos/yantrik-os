//! Self-awareness instinct — surfaces interesting self-observations.

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct SelfAwarenessInstinct;

impl Instinct for SelfAwarenessInstinct {
    fn name(&self) -> &str {
        "SelfAwareness"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let mut urges = Vec::new();

        // Surface self-related patterns from the think cycle
        for trigger in &state.pending_triggers {
            let ttype = trigger
                .get("trigger_type")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Look for patterns that involve the companion itself
            if ttype == "pattern_discovered" {
                if let Some(desc) = trigger.get("description").and_then(|v| v.as_str()) {
                    let lower = desc.to_lowercase();
                    if lower.contains("self")
                        || lower.contains("companion")
                        || lower.contains("conversation")
                        || lower.contains("interaction")
                    {
                        let urgency = trigger
                            .get("urgency")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.4);

                        urges.push(
                            UrgeSpec::new(
                                self.name(),
                                &format!("Self-observation: {desc}"),
                                urgency,
                            )
                            .with_cooldown(&format!("self_aware:{}", &desc[..desc.len().min(30)]))
                            .with_context(trigger.clone()),
                        );
                    }
                }
            }
        }

        // Milestone-based self-awareness
        if state.conversation_turn_count == 10 {
            urges.push(
                UrgeSpec::new(
                    self.name(),
                    "We've had 10 exchanges now. I'm starting to understand your communication style.",
                    0.4,
                )
                .with_cooldown("self_aware:10_turns"),
            );
        }

        urges.into_iter().take(1).collect()
    }
}
