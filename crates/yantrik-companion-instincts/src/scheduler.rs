//! Scheduler instinct — converts due scheduled tasks into urges.

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec, ModelTier};

pub struct SchedulerInstinct;

impl Instinct for SchedulerInstinct {
    fn name(&self) -> &str {
        "scheduler"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        state
            .pending_triggers
            .iter()
            .filter(|trigger| {
                trigger
                    .get("trigger_type")
                    .and_then(|v| v.as_str())
                    == Some("scheduled_task")
            })
            .map(|trigger| {
                let task_id = trigger
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let label = trigger
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Scheduled task");
                let description = trigger
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let urgency = trigger
                    .get("urgency")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.6)
                    .max(0.5);
                let action = trigger
                    .get("action")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());

                // Build message — if action present, include execution instructions
                let message = if let Some(ref act) = action {
                    if act.starts_with("automation:") {
                        // Linked automation — AutomationInstinct handles this
                        if description.is_empty() {
                            format!("Scheduled: {}", label)
                        } else {
                            format!("Scheduled: {} \u{2014} {}", label, description)
                        }
                    } else {
                        match state.model_tier {
                            ModelTier::Large => format!(
                                "EXECUTE scheduled action '{}': {}", label, act
                            ),
                            ModelTier::Tiny => format!(
                                "EXECUTE Run scheduled action '{}'. Output: 1 sentence.", label
                            ),
                            _ => format!(
                                "EXECUTE Task: Run scheduled action '{}'.\n\
                                 Input: action={}.\n\
                                 Rule: Do not invent causes or speculate.\n\
                                 Fallback: \"No action needed.\"\n\
                                 Output: 1 sentence.",
                                label, act
                            ),
                        }
                    }
                } else if description.is_empty() {
                    format!("Scheduled: {}", label)
                } else {
                    format!("Scheduled: {} \u{2014} {}", label, description)
                };

                // Boost urgency for action-bearing tasks to ensure execution
                let final_urgency = if action.is_some() { urgency.max(0.8) } else { urgency };

                let mut spec = UrgeSpec::new("scheduler", &message, final_urgency)
                    .with_cooldown(&format!("sched:{}", task_id))
                    .with_message(&message)
                    .with_context(trigger.clone());

                if let Some(act) = action {
                    spec.action = Some(act);
                }

                spec
            })
            .collect()
    }
}
