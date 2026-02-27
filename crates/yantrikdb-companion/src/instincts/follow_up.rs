//! Follow-up instinct — surfaces important memories that are fading.

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct FollowUpInstinct;

impl Instinct for FollowUpInstinct {
    fn name(&self) -> &str {
        "follow_up"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let decay_triggers: Vec<&serde_json::Value> = state
            .pending_triggers
            .iter()
            .filter(|t| t.get("trigger_type").and_then(|v| v.as_str()) == Some("decay_review"))
            .take(2)
            .collect();

        decay_triggers
            .iter()
            .map(|trigger| {
                let ctx = trigger.get("context").cloned().unwrap_or_default();
                let reason = trigger
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("An important memory is fading");
                let urgency = trigger
                    .get("urgency")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.4)
                    .min(0.7);
                let trigger_id = trigger
                    .get("trigger_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                UrgeSpec::new("follow_up", &format!("Follow up: {reason}"), urgency)
                    .with_cooldown(&format!("follow_up:{trigger_id}"))
                    .with_context(ctx)
            })
            .collect()
    }
}
