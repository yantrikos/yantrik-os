//! Morning Brief instinct — daily keystone message with weather, schedule, and system status.
//!
//! Fires once per day during the morning window (default 6:00–10:00 local time).
//! Uses EXECUTE to call real tools (weather, calendar, email, system_info) at
//! delivery time for fresh data.

use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};

pub struct MorningBriefInstinct {
    /// Hour to start the morning window (inclusive). Default: 6.
    morning_start: u32,
    /// Hour to end the morning window (exclusive). Default: 10.
    morning_end: u32,
    /// Date string of last delivered brief (e.g. "2026-03-02"). Prevents double delivery.
    last_brief_date: Mutex<String>,
}

impl MorningBriefInstinct {
    pub fn new() -> Self {
        Self {
            morning_start: 6,
            morning_end: 10,
            last_brief_date: Mutex::new(String::new()),
        }
    }
}

impl Instinct for MorningBriefInstinct {
    fn name(&self) -> &str {
        "morning_brief"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = chrono::Local::now();
        let hour = now.format("%H").to_string().parse::<u32>().unwrap_or(12);
        let today = now.format("%Y-%m-%d").to_string();

        if hour < self.morning_start || hour >= self.morning_end {
            return vec![];
        }

        {
            let last = self.last_brief_date.lock().unwrap();
            if *last == today {
                return vec![];
            }
        }
        {
            let mut last = self.last_brief_date.lock().unwrap();
            *last = today.clone();
        }

        // EXECUTE prefix triggers tool-calling in generate_proactive_message,
        // so the brief gets real-time weather, calendar, email, and system data.
        let reason = format!(
            "EXECUTE Deliver {}'s morning brief. Call get_weather, calendar_today, \
             email_check, and system_info. Then compose a concise daily briefing \
             with weather, today's events, unread emails, and system status. \
             Be warm and natural — this is {} starting their day.",
            state.config_user_name, state.config_user_name
        );

        vec![UrgeSpec::new("morning_brief", &reason, 0.9)
            .guaranteed()
            .with_cooldown("morning_brief:daily")]
    }
}
