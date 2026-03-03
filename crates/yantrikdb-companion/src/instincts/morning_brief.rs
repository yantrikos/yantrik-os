//! Morning Brief instinct — daily keystone message with weather, schedule, and system status.
//!
//! Fires once per day during the morning window (default 6:00–10:00 local time).
//! Gathers weather, today's scheduled tasks, and system observations into
//! context data slots consumed by the `morning_brief` template.

use std::sync::Mutex;

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

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
        // Check if we're in the morning window
        let now = chrono::Local::now();
        let hour = now.format("%H").to_string().parse::<u32>().unwrap_or(12);
        let today = now.format("%Y-%m-%d").to_string();

        // Skip if outside morning window
        if hour < self.morning_start || hour >= self.morning_end {
            return vec![];
        }

        // Skip if we already delivered today
        {
            let last = self.last_brief_date.lock().unwrap();
            if *last == today {
                return vec![];
            }
        }
        // Mark as delivered for today (set early to prevent double-fire)
        {
            let mut last = self.last_brief_date.lock().unwrap();
            *last = today.clone();
        }

        // Gather data for the template slots
        let mut context = serde_json::Map::new();

        // Weather line — quick fetch from wttr.in
        let weather_line = fetch_weather_brief();
        context.insert("weather_line".into(), serde_json::Value::String(weather_line));

        // Schedule line — check for today's upcoming tasks
        let schedule_line = build_schedule_line(state);
        if !schedule_line.is_empty() {
            context.insert(
                "schedule_line".into(),
                serde_json::Value::String(schedule_line),
            );
        }

        // System note — any resource concerns from pending triggers
        let system_note = build_system_note(state);
        if !system_note.is_empty() {
            context.insert(
                "system_note".into(),
                serde_json::Value::String(system_note),
            );
        }

        // Pattern note — any active behavioral patterns to mention
        let pattern_note = build_pattern_note(state);
        if !pattern_note.is_empty() {
            context.insert(
                "pattern_note".into(),
                serde_json::Value::String(pattern_note),
            );
        }

        // Mark as morning_brief trigger type so bridge can update last_brief_date
        context.insert(
            "trigger_type".into(),
            serde_json::Value::String("morning_brief".into()),
        );
        context.insert(
            "date".into(),
            serde_json::Value::String(today),
        );

        let reason = format!(
            "Morning briefing for {}",
            state.config_user_name
        );

        vec![UrgeSpec::new("morning_brief", &reason, 0.65)
            .with_cooldown("morning_brief:daily")
            .with_message("") // Template engine will compose the message
            .with_context(serde_json::Value::Object(context))]
    }
}

/// Quick weather fetch — one-liner from wttr.in (same API as weather tool).
fn fetch_weather_brief() -> String {
    let url = "https://wttr.in/?format=%C+%t+humidity:%h";
    match std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "5", "--connect-timeout", "3", url])
        .env("LANG", "en_US.UTF-8")
        .output()
    {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if text.is_empty() || text.contains("Unknown") {
                "Weather data unavailable.".into()
            } else {
                format!("Weather: {}.", text)
            }
        }
        _ => "Weather data unavailable.".into(),
    }
}

/// Check scheduled tasks in pending_triggers for today's items.
fn build_schedule_line(state: &CompanionState) -> String {
    let tasks: Vec<&str> = state
        .pending_triggers
        .iter()
        .filter(|t| {
            t.get("trigger_type")
                .and_then(|v| v.as_str())
                == Some("scheduled_task")
        })
        .filter_map(|t| t.get("label").and_then(|v| v.as_str()))
        .collect();

    match tasks.len() {
        0 => String::new(),
        1 => format!("You have 1 task today: {}.", tasks[0]),
        n => format!("You have {} tasks today: {}.", n, tasks.join(", ")),
    }
}

/// Summarize any system resource concerns from triggers.
fn build_system_note(state: &CompanionState) -> String {
    let notes: Vec<String> = state
        .pending_triggers
        .iter()
        .filter(|t| {
            matches!(
                t.get("trigger_type").and_then(|v| v.as_str()),
                Some("cpu_high" | "memory_high" | "disk_high" | "battery_low")
            )
        })
        .filter_map(|t| {
            t.get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .take(2) // Keep brief
        .collect();

    if notes.is_empty() {
        String::new()
    } else {
        format!(" Note: {}", notes.join(". "))
    }
}

/// Pick the first active pattern to mention.
fn build_pattern_note(state: &CompanionState) -> String {
    if let Some(pattern) = state.active_patterns.first() {
        if let Some(desc) = pattern.get("description").and_then(|v| v.as_str()) {
            return format!("I've noticed: {}", desc);
        }
    }
    String::new()
}
