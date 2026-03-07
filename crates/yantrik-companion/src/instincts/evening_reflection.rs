//! Evening Reflection instinct — end-of-day debrief paired with morning_brief.
//!
//! Summarizes the day's activity and looks ahead to tomorrow.
//! Creates a bookend with the morning_brief for a natural daily rhythm.

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

use std::sync::Mutex;

pub struct EveningReflectionInstinct {
    last_reflection_date: Mutex<Option<u32>>,
}

impl EveningReflectionInstinct {
    pub fn new() -> Self {
        Self {
            last_reflection_date: Mutex::new(None),
        }
    }
}

impl Instinct for EveningReflectionInstinct {
    fn name(&self) -> &str {
        "EveningReflection"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Only at Friend+
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        // Only during evening hours (19-22 / 7 PM - 10 PM)
        if state.current_hour < 19 || state.current_hour > 22 {
            return vec![];
        }

        // Once per day
        let today = state.current_day_of_week;
        if let Ok(mut last) = self.last_reflection_date.lock() {
            if *last == Some(today) {
                return vec![];
            }
            *last = Some(today);
        }

        // Build context about the day
        let events_today: Vec<&str> = state
            .recent_events
            .iter()
            .filter(|(_, ts, _)| {
                let age_hours = (state.current_ts - ts) / 3600.0;
                age_hours < 14.0 // Events from the last 14 hours (covers the workday)
            })
            .map(|(desc, _, _)| desc.as_str())
            .collect();

        let events_summary = if events_today.is_empty() {
            "No major events tracked today.".to_string()
        } else {
            events_today.join("; ")
        };

        let urgency = match state.bond_level {
            BondLevel::Friend => 0.45,
            BondLevel::Confidant => 0.5,
            BondLevel::PartnerInCrime => 0.55,
            _ => 0.4,
        };

        vec![
            UrgeSpec::new(
                self.name(),
                &format!(
                    "EXECUTE Give a brief end-of-day reflection. Today's events: [{}]. \
                     Summarize what stood out, acknowledge effort or progress, and optionally \
                     suggest what tomorrow might bring. Keep it warm but concise — 2-3 sentences. \
                     Don't be overly enthusiastic or use exclamation marks. \
                     Speak like a friend winding down the day together.",
                    events_summary
                ),
                urgency,
            )
            .with_cooldown("evening_reflection:daily"),
        ]
    }
}
