//! Conflict alerting instinct — surfaces when too many memory conflicts pile up.

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct ConflictAlertingInstinct {
    threshold: usize,
}

impl ConflictAlertingInstinct {
    pub fn new(threshold: usize) -> Self {
        Self { threshold }
    }
}

impl Instinct for ConflictAlertingInstinct {
    fn name(&self) -> &str {
        "conflict_alerting"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        if state.open_conflicts_count < self.threshold {
            return vec![];
        }

        let urgency = (state.open_conflicts_count as f64 / 10.0).min(0.8);

        vec![UrgeSpec::new(
            "conflict_alerting",
            &format!(
                "{} memory conflicts need your help",
                state.open_conflicts_count
            ),
            urgency,
        )
        .with_cooldown("conflict_alert")
        .with_context(serde_json::json!({
            "open_count": state.open_conflicts_count,
        }))]
    }
}
