//! Pattern surfacing instinct — surfaces newly discovered patterns.

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct PatternSurfacingInstinct;

impl Instinct for PatternSurfacingInstinct {
    fn name(&self) -> &str {
        "pattern_surfacing"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        state
            .pending_triggers
            .iter()
            .filter(|t| {
                t.get("trigger_type").and_then(|v| v.as_str()) == Some("pattern_discovered")
            })
            .take(2)
            .map(|trigger| {
                let ctx = trigger.get("context").cloned().unwrap_or_default();
                let desc = ctx
                    .get("description")
                    .and_then(|v| v.as_str())
                    .or_else(|| trigger.get("reason").and_then(|v| v.as_str()))
                    .unwrap_or("A new pattern");
                let urgency = trigger
                    .get("urgency")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.3);
                let pattern_type = ctx
                    .get("pattern_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let cooldown = format!("pattern:{}:{}", pattern_type, &desc[..desc.len().min(30)]);

                UrgeSpec::new("pattern_surfacing", &format!("Pattern noticed: {desc}"), urgency)
                    .with_cooldown(&cooldown)
                    .with_context(ctx)
            })
            .collect()
    }
}
