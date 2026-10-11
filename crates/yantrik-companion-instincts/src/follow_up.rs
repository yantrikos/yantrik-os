//! Follow-up instinct — surfaces important memories that are fading.

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};

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
                    .with_message(&format!("I was thinking about something from before — {reason}"))
                    .with_context(ctx)
                    .from_memories(crate::trigger_memory_ids(trigger))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The urge carries the memories its trigger was made from, so "Don't use this memory" can
    /// exclude them (#754).
    #[test]
    fn a_follow_up_carries_the_memories_it_came_from() {
        let mut state = crate::test_util::state_idle_for(None);
        state.pending_triggers = vec![serde_json::json!({
            "trigger_type": "decay_review",
            "trigger_id": "t1",
            "reason": "the move to Austin",
            "source_rids": ["rid-1", "rid-2"],
        })];
        let urges = FollowUpInstinct.evaluate(&state);
        assert_eq!(urges.len(), 1);
        assert_eq!(urges[0].memory_ids, vec!["rid-1".to_string(), "rid-2".to_string()]);
        assert!(!urges[0].safety);
        assert!(crate::trigger_memory_ids(&serde_json::json!({})).is_empty());
    }
}
