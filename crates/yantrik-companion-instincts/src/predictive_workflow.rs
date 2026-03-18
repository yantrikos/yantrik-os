//! Predictive Workflow instinct — learns activity patterns by time-of-day
//! and suggests what the user usually does at this hour.

use std::sync::Mutex;

use yantrik_companion_core::bond::BondLevel;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};
use crate::Instinct;

pub struct PredictiveWorkflowInstinct {
    last_fired_hour: Mutex<Option<u32>>,
}

impl PredictiveWorkflowInstinct {
    pub fn new() -> Self {
        Self {
            last_fired_hour: Mutex::new(None),
        }
    }
}

impl Instinct for PredictiveWorkflowInstinct {
    fn name(&self) -> &str {
        "predictive_workflow"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Gate: Acquaintance+ bond
        if state.bond_level < BondLevel::Acquaintance {
            return vec![];
        }

        let hour = state.current_hour;

        // Only fire once per hour
        {
            let mut last = self.last_fired_hour.lock().unwrap();
            if *last == Some(hour) {
                return vec![];
            }
            *last = Some(hour);
        }

        // Need workflow hints for current hour with 3+ days of data
        let matching: Vec<_> = state
            .workflow_hints
            .iter()
            .filter(|h| h.get("hour").and_then(|v| v.as_u64()) == Some(hour as u64))
            .filter(|h| h.get("days_observed").and_then(|v| v.as_u64()).unwrap_or(0) >= 3)
            .collect();

        if matching.is_empty() {
            return vec![];
        }

        // Pick the most frequent activity
        let best = matching
            .iter()
            .max_by(|a, b| {
                let da = a.get("days_observed").and_then(|v| v.as_u64()).unwrap_or(0);
                let db = b.get("days_observed").and_then(|v| v.as_u64()).unwrap_or(0);
                da.cmp(&db)
            })
            .unwrap();

        let activity = best
            .get("activity")
            .and_then(|v| v.as_str())
            .unwrap_or("something");
        let days = best
            .get("days_observed")
            .and_then(|v| v.as_u64())
            .unwrap_or(3);

        // Scale urgency: 3 days = 0.45, 7+ days = 0.6
        let urgency = (0.45 + (days as f64 - 3.0).min(4.0) * 0.0375).min(0.6);

        let label = activity_label(activity);
        let msg = format!(
            "It's about that time \u{2014} you usually {} around now.",
            label
        );

        vec![
            UrgeSpec::new("predictive_workflow", &msg, urgency)
                .with_cooldown(&format!("predictive_workflow:{}:{}", hour, activity))
                .with_message(&msg)
                .with_context(serde_json::json!({
                    "hour": hour,
                    "activity": activity,
                    "days_observed": days,
                })),
        ]
    }
}

/// Human-readable label for activity categories.
fn activity_label(activity: &str) -> &str {
    match activity {
        "coding" => "dive into code",
        "communication" => "catch up on messages",
        "research" => "do some research",
        "system_admin" => "do system maintenance",
        "planning" => "plan things out",
        _ => "get to work",
    }
}
