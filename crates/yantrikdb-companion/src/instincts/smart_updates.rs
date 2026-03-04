//! Smart Updates instinct — runs maintenance during idle, reports when user returns.

use std::sync::Mutex;

use crate::bond::BondLevel;
use crate::types::{CompanionState, UrgeSpec};
use super::Instinct;

pub struct SmartUpdatesInstinct {
    last_idle_task_ts: Mutex<f64>,
    last_report_ts: Mutex<f64>,
}

impl SmartUpdatesInstinct {
    pub fn new() -> Self {
        Self {
            last_idle_task_ts: Mutex::new(0.0),
            last_report_ts: Mutex::new(0.0),
        }
    }
}

/// Maintenance tasks in priority order.
const MAINTENANCE_TASKS: &[(&str, &str)] = &[
    ("memory_consolidation", "EXECUTE run_command: echo 'Memory consolidation check'"),
    ("disk_check", "EXECUTE system_info"),
    ("log_rotation", "EXECUTE run_command: find /var/log -name '*.log' -size +50M 2>/dev/null | head -5"),
    ("tmp_cleanup", "EXECUTE run_command: find /tmp -type f -mtime +7 -delete 2>/dev/null; echo 'Cleaned old temp files'"),
];

impl Instinct for SmartUpdatesInstinct {
    fn name(&self) -> &str {
        "smart_updates"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let mut urges = Vec::new();

        // Phase A: Idle maintenance (>30 min idle)
        if state.idle_seconds > 1800.0 {
            let mut last_ts = self.last_idle_task_ts.lock().unwrap();
            // One task per idle period (fire once then wait for next idle)
            if state.current_ts - *last_ts > 3600.0 {
                // Pick highest-priority task not recently done
                let done_tasks: Vec<String> = state
                    .maintenance_report
                    .iter()
                    .filter(|r| {
                        r.get("completed_at")
                            .and_then(|v| v.as_f64())
                            .map(|t| state.current_ts - t < 86400.0) // within 24h
                            .unwrap_or(false)
                    })
                    .filter_map(|r| r.get("task_name").and_then(|v| v.as_str()).map(String::from))
                    .collect();

                for (task_name, action) in MAINTENANCE_TASKS {
                    if done_tasks.contains(&task_name.to_string()) {
                        continue;
                    }

                    *last_ts = state.current_ts;
                    urges.push(
                        UrgeSpec::new("smart_updates", action, 0.8)
                            .with_cooldown(&format!("smart_updates:idle:{}", task_name))
                            .with_message(&format!("Running background maintenance: {}", task_name))
                            .with_context(serde_json::json!({
                                "phase": "idle",
                                "task_name": task_name,
                            })),
                    );
                    break; // One at a time
                }
            }
        }

        // Phase B: Return report (user came back from idle)
        if state.idle_seconds < 60.0 && state.conversation_turn_count <= 1 {
            let unreported: Vec<_> = state
                .maintenance_report
                .iter()
                .filter(|r| {
                    r.get("reported").and_then(|v| v.as_bool()).unwrap_or(false) == false
                        && r.get("status").and_then(|v| v.as_str()) == Some("completed")
                })
                .collect();

            if !unreported.is_empty() {
                let mut last_report = self.last_report_ts.lock().unwrap();
                if state.current_ts - *last_report > 86400.0 {
                    *last_report = state.current_ts;

                    let summaries: Vec<&str> = unreported
                        .iter()
                        .filter_map(|r| r.get("summary").and_then(|v| v.as_str()))
                        .take(3)
                        .collect();

                    let summary = if summaries.is_empty() {
                        "ran some background maintenance".to_string()
                    } else {
                        summaries.join(", ")
                    };

                    let msg = format!("While you were away, I {}.", summary);
                    urges.push(
                        UrgeSpec::new("smart_updates", &msg, 0.5)
                            .with_cooldown("smart_updates:report:daily")
                            .with_message(&msg)
                            .with_context(serde_json::json!({
                                "phase": "report",
                                "task_count": unreported.len(),
                            })),
                    );
                }
            }
        }

        urges
    }
}
