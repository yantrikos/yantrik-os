//! Memory Weaver instinct — drives proactive memory graph building during idle time.
//!
//! Two modes of operation:
//! 1. WEAVING: During idle time, finds connections between memories from different
//!    conversations and surfaces surprising links.
//! 2. ON THIS DAY: Once daily, recalls what was happening in the user's life on this
//!    date in previous weeks/months — a personal time capsule.

use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec, ModelTier};

pub struct MemoryWeaverInstinct {
    /// Minimum idle time (seconds) before weaving urges fire.
    idle_threshold_secs: f64,
    /// Minimum memories before weaving is worthwhile.
    min_memories: i64,
    /// Last "On This Day" check timestamp.
    last_on_this_day_ts: Mutex<f64>,
}

impl MemoryWeaverInstinct {
    pub fn new(idle_threshold_minutes: f64, min_memories: i64) -> Self {
        Self {
            idle_threshold_secs: idle_threshold_minutes * 60.0,
            min_memories,
            last_on_this_day_ts: Mutex::new(0.0),
        }
    }

    /// Check if "On This Day" should fire (once per day, morning preferred).
    fn should_on_this_day(&self, state: &CompanionState) -> bool {
        // Need enough memories for meaningful lookback
        if state.memory_count < 20 {
            return false;
        }

        // Prefer morning delivery (7-11 AM)
        if state.current_hour < 7 || state.current_hour > 11 {
            return false;
        }

        // Once per day (cold-start guard + 20-hour cooldown)
        let mut last = self.last_on_this_day_ts.lock().unwrap();
        if *last == 0.0 {
            *last = state.current_ts;
            return false;
        }
        if state.current_ts - *last < 20.0 * 3600.0 {
            return false;
        }
        *last = state.current_ts;
        true
    }
}

impl Instinct for MemoryWeaverInstinct {
    fn name(&self) -> &str {
        "MemoryWeaver"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let mut urges = Vec::new();

        // --- "On This Day" mode ---
        if self.should_on_this_day(state) {
            let user = &state.config_user_name;
            let on_this_day_msg = format!(
                "EXECUTE Search through {user}'s memory for what was happening around this \
                 time in previous weeks and months. Use recall with queries like \
                 \"a week ago\", \"two weeks ago\", \"a month ago\", \"last month\". \
                 \nLook for:\n\
                 - What {user} was working on\n\
                 - What they were feeling or thinking about\n\
                 - Problems they were solving\n\
                 - Interests they were exploring\n\
                 \nIf you find something interesting from the past, share it as a brief \
                 'On this day' reflection in 2-3 sentences:\n\
                 - What was happening then\n\
                 - How things have changed (or haven't)\n\
                 - A brief observation about the journey\n\
                 \nIf nothing meaningful is found in the lookback, respond with just \
                 \"No time capsule today.\"\n\
                 IMPORTANT: Be specific — reference actual topics, projects, or feelings \
                 from the recalled memories. Don't be generic.",
            );

            urges.push(
                UrgeSpec::new(self.name(), &on_this_day_msg, 0.45)
                    .with_cooldown("weaver:on_this_day")
                    .with_context(serde_json::json!({
                        "mode": "on_this_day",
                        "memory_count": state.memory_count,
                    })),
            );
        }

        // --- Standard weaving mode ---
        let idle_secs = state.current_ts - state.last_interaction_ts;
        if idle_secs >= self.idle_threshold_secs && state.memory_count >= self.min_memories {
            let idle_factor = ((idle_secs - self.idle_threshold_secs) / 3600.0).min(1.0);
            let urgency = 0.2 + idle_factor * 0.2;

            let has_conflicts = state.open_conflicts_count > 0;
            let has_patterns = !state.active_patterns.is_empty();

                        let execute_msg = match state.model_tier {
                ModelTier::Large => format!(
                    "EXECUTE Review my memory graph ({} memories{}{}). \
                 Find one interesting connection between memories from different conversations \
                 and share it as a brief, natural insight (1-2 sentences). \
                 If nothing interesting stands out, say nothing.",
                state.memory_count,
                if has_conflicts {
                    format!(", {} conflicts to resolve", state.open_conflicts_count)
                } else {
                    String::new()
                },
                if has_patterns {
                    format!(", {} active patterns", state.active_patterns.len())
                } else {
                    String::new()
                },
                ),
                ModelTier::Tiny => format!(
                    "EXECUTE SKIP",
                ),
                _ => format!(
                    "EXECUTE Task: Surface one interesting memory connection for .\n\
             Tool: Use recall to find one relevant past memory.\n\
             Rule: Use only details explicitly stated by the user or returned by recall. Do not invent memories or connections.\n\
             Fallback: \"Nothing to surface right now.\"\n\
             Output: 1 sentence.",
                ),
            };

            urges.push(
                UrgeSpec::new(self.name(), &execute_msg, urgency)
                    .with_cooldown("weaver:digest")
                    .with_context(serde_json::json!({
                        "mode": "weaving",
                        "idle_seconds": idle_secs,
                        "memory_count": state.memory_count,
                        "conflicts": state.open_conflicts_count,
                        "patterns": state.active_patterns.len(),
                    })),
            );
        }

        urges
    }
}
