//! Automation instinct — converts fired automations into executable urges.
//!
//! When a scheduled automation fires (via SchedulerInstinct) or an event automation
//! matches, this instinct creates high-urgency urges with the automation's steps
//! as actionable instructions for the LLM to execute.

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct AutomationInstinct;

impl Instinct for AutomationInstinct {
    fn name(&self) -> &str {
        "automation"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        state
            .pending_triggers
            .iter()
            .filter(|trigger| {
                trigger
                    .get("trigger_type")
                    .and_then(|v| v.as_str())
                    == Some("automation")
            })
            .map(|trigger| {
                let automation_id = trigger
                    .get("automation_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let name = trigger
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Automation");
                let steps = trigger
                    .get("steps")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let condition = trigger
                    .get("condition")
                    .and_then(|v| v.as_str());

                let message = if let Some(cond) = condition {
                    format!(
                        "EXECUTE automation '{}' (check condition first: {}): {}",
                        name, cond, steps
                    )
                } else {
                    format!("EXECUTE automation '{}': {}", name, steps)
                };

                // Automations get high urgency to ensure execution
                UrgeSpec::new("automation", &message, 0.85)
                    .with_cooldown(&format!("auto:{}", automation_id))
                    .with_message(&message)
                    .with_context(trigger.clone())
            })
            .collect()
    }
}
