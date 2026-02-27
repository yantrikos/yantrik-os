//! Check-in instinct — urges companion to reach out when idle.

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct CheckInInstinct {
    hours_threshold: f64,
}

impl CheckInInstinct {
    pub fn new(hours_threshold: f64) -> Self {
        Self { hours_threshold }
    }
}

impl Instinct for CheckInInstinct {
    fn name(&self) -> &str {
        "check_in"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let hours_since = (state.current_ts - state.last_interaction_ts) / 3600.0;

        if hours_since < self.hours_threshold {
            return vec![];
        }

        // Urgency: 0 at threshold, caps at 0.8
        let urgency = ((hours_since - self.hours_threshold) / (self.hours_threshold * 2.0)).min(0.8);

        vec![UrgeSpec::new(
            "check_in",
            &format!(
                "{} hasn't interacted in {:.0} hours",
                state.config_user_name, hours_since
            ),
            urgency,
        )
        .with_cooldown("check_in")
        .with_context(serde_json::json!({
            "hours_since_interaction": (hours_since * 10.0).round() / 10.0,
        }))]
    }
}
