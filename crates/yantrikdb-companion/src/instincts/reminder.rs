//! Reminder instinct — surfaces reminders coming due.

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct ReminderInstinct;

impl Instinct for ReminderInstinct {
    fn name(&self) -> &str {
        "reminder"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        state
            .pending_triggers
            .iter()
            .filter(|trigger| {
                let ctx = trigger.get("context").unwrap_or(&serde_json::Value::Null);
                let domain = ctx.get("domain").and_then(|v| v.as_str()).unwrap_or("");
                let reason = trigger
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                domain == "reminder" || reason.to_lowercase().contains("remind")
            })
            .map(|trigger| {
                let ctx = trigger.get("context").cloned().unwrap_or_default();
                let reason = trigger
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("A reminder is due");
                let urgency = trigger
                    .get("urgency")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.5)
                    .max(0.6);
                let trigger_id = trigger
                    .get("trigger_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                UrgeSpec::new("reminder", reason, urgency)
                    .with_cooldown(&format!("reminder:{trigger_id}"))
                    .with_context(ctx)
            })
            .collect()
    }
}
