//! Routine instinct — recognizes morning/evening routines from workflow
//! observations and suggests the user's established patterns.

use std::sync::Mutex;

use yantrik_companion_core::bond::BondLevel;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};
use crate::Instinct;

pub struct RoutineInstinct {
    /// Track last fired window to avoid repeats within the same day-window.
    last_fired: Mutex<Option<(u32, String)>>, // (day_of_year, "morning"|"evening")
}

impl RoutineInstinct {
    pub fn new() -> Self {
        Self {
            last_fired: Mutex::new(None),
        }
    }
}

impl Instinct for RoutineInstinct {
    fn name(&self) -> &str {
        "routine"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Gate: Friend+ bond
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        let hour = state.current_hour;
        let window = if (6..10).contains(&hour) {
            "morning"
        } else if (18..22).contains(&hour) {
            "evening"
        } else {
            return vec![];
        };

        // One per window per day
        let day_of_year = state.current_day_of_week; // approximate; good enough for dedup
        {
            let mut last = self.last_fired.lock().unwrap();
            let key = (day_of_year, window.to_string());
            if last.as_ref() == Some(&key) {
                return vec![];
            }
            *last = Some(key);
        }

        // Gather activities for current time window (hours 6-10 or 18-22)
        let window_hours: Vec<u64> = if window == "morning" {
            (6..10).map(|h| h as u64).collect()
        } else {
            (18..22).map(|h| h as u64).collect()
        };

        let mut activity_days: Vec<(&str, u64)> = Vec::new();
        for hint in &state.workflow_hints {
            let h = hint.get("hour").and_then(|v| v.as_u64()).unwrap_or(99);
            if !window_hours.contains(&h) {
                continue;
            }
            let days = hint.get("days_observed").and_then(|v| v.as_u64()).unwrap_or(0);
            if days < 5 {
                continue;
            }
            if let Some(act) = hint.get("activity").and_then(|v| v.as_str()) {
                activity_days.push((act, days));
            }
        }

        if activity_days.is_empty() {
            return vec![];
        }

        // Sort by frequency, take top 3 unique activities
        activity_days.sort_by(|a, b| b.1.cmp(&a.1));
        let mut seen = Vec::new();
        let mut top: Vec<&str> = Vec::new();
        for (act, _) in &activity_days {
            if !seen.contains(act) {
                seen.push(act);
                top.push(act);
                if top.len() >= 3 {
                    break;
                }
            }
        }

        let summary = top
            .iter()
            .map(|a| routine_label(a))
            .collect::<Vec<_>>()
            .join(", then ");

        let msg = format!(
            "Your usual {} routine: {}.",
            window, summary
        );

        vec![
            UrgeSpec::new("routine", &msg, 0.5)
                .with_cooldown(&format!("routine:{}:daily", window))
                .with_message(&msg)
                .with_context(serde_json::json!({
                    "window": window,
                    "activities": top,
                })),
        ]
    }
}

fn routine_label(activity: &str) -> &str {
    match activity {
        "coding" => "code",
        "communication" => "messages",
        "research" => "research",
        "system_admin" => "system checks",
        "planning" => "planning",
        _ => "work",
    }
}
