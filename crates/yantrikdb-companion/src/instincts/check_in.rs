//! Check-in instinct — urges companion to reach out when idle.
//!
//! Uses EXECUTE to produce contextual check-ins rather than generic greetings.

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

        // Urgency: starts at 0.3, caps at 0.5 — check-ins are low priority
        let excess_ratio = ((hours_since - self.hours_threshold) / self.hours_threshold).min(1.0);
        let urgency = 0.3 + excess_ratio * 0.2;

        let execute_msg = format!(
            "EXECUTE {} hasn't been around for {:.0} hours. \
             If you remember something relevant about what they were working on or interested in, \
             send a brief, natural check-in that references it (1 sentence). \
             If you can't recall anything specific, just say nothing — don't send a generic greeting.",
            state.config_user_name, hours_since,
        );

        vec![UrgeSpec::new(
            "check_in",
            &execute_msg,
            urgency,
        )
        .with_cooldown("check_in")
        .with_context(serde_json::json!({
            "hours_since_interaction": (hours_since * 10.0).round() / 10.0,
        }))]
    }
}
