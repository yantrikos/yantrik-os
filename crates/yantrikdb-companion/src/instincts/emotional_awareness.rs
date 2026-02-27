//! Emotional awareness instinct — detects negative valence trends.

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct EmotionalAwarenessInstinct;

impl Instinct for EmotionalAwarenessInstinct {
    fn name(&self) -> &str {
        "emotional_awareness"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Look for valence_trend triggers from think()
        let valence_triggers: Vec<&serde_json::Value> = state
            .pending_triggers
            .iter()
            .filter(|t| t.get("trigger_type").and_then(|v| v.as_str()) == Some("valence_trend"))
            .collect();

        if valence_triggers.is_empty() {
            return vec![];
        }

        let trigger = &valence_triggers[0];
        let ctx = trigger.get("context").cloned().unwrap_or_default();
        let direction = ctx
            .get("direction")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if direction != "negative" {
            return vec![];
        }

        let delta = ctx
            .get("delta")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            .abs();
        let urgency = delta.min(0.9);

        vec![UrgeSpec::new(
            "emotional_awareness",
            &format!("Emotional tone shifted negative (delta: {delta:.2})"),
            urgency,
        )
        .with_cooldown("emotional_awareness:negative")
        .with_context(ctx)]
    }
}
