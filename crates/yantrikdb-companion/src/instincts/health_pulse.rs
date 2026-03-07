//! HealthPulse instinct — "Your body is trying to tell you something — I'm listening."
//!
//! Notices health-related mentions in conversation history (tired, headache, stress,
//! sleep, exercise, pain, energy). Tracks patterns over weeks and researches the
//! underlying science behind what the user reports.
//!
//! This is NOT a medical advisor. It surfaces awareness — the biological mechanisms
//! (enzymes, hormones, neural pathways) behind everyday health observations. The goal
//! is to help the user connect the dots between their habits and how they feel, backed
//! by peer-reviewed science rather than folk wisdom.
//!
//! Example output: "You've mentioned being tired 3 times this week, always after late
//! nights. Sleep researchers found that each hour of delayed sleep reduces next-day
//! cognitive performance by 25% — it's the adenosine buildup in your prefrontal cortex."

use std::sync::Mutex;

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

/// Health-related keywords to detect in conversation history.
const HEALTH_KEYWORDS: &[&str] = &[
    "tired", "headache", "stress", "sleep", "exercise", "pain",
    "energy", "exhausted", "insomnia", "anxious", "sore", "sick",
    "workout", "migraine", "fatigue", "burnout", "back pain",
];

pub struct HealthPulseInstinct {
    /// Seconds between health pulse evaluations.
    interval_secs: f64,
    /// Last evaluation timestamp.
    last_check_ts: Mutex<f64>,
}

impl HealthPulseInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for HealthPulseInstinct {
    fn name(&self) -> &str {
        "HealthPulse"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // Rate-limit with cold-start guard
        {
            let mut last = self.last_check_ts.lock().unwrap();
            if *last == 0.0 {
                *last = now;
                return vec![];
            }
            if now - *last < self.interval_secs {
                return vec![];
            }
            *last = now;
        }

        // Gate: need conversation history to find health mentions
        if state.conversation_turn_count <= 5 {
            return vec![];
        }

        // Gate: bond level — at least Acquaintance to discuss health
        if state.bond_level < BondLevel::Acquaintance {
            return vec![];
        }

        // Check recent sent messages for health topics (to detect if user
        // recently mentioned health — boosts urgency)
        let recent_health_mention = state.recent_sent_messages.iter().any(|msg| {
            let lower = msg.to_lowercase();
            HEALTH_KEYWORDS.iter().any(|kw| lower.contains(kw))
        });

        let urgency = if recent_health_mention { 0.55 } else { 0.45 };

        let user = &state.config_user_name;

        let execute_msg = format!(
            "EXECUTE First, use recall with query \"health sleep tired energy exercise stress pain\" \
             to find {user}'s health-related mentions and patterns over time.\n\
             \n\
             Analyze what you find:\n\
             - Are there recurring complaints (e.g., tiredness always on certain days)?\n\
             - Any correlation between habits mentioned and symptoms?\n\
             - What has {user} specifically reported recently?\n\
             \n\
             Then use web_search to research the underlying science behind ONE pattern you found.\n\
             Look for the specific biological mechanism — name the enzyme, hormone, or neural \
             pathway involved. Cite the mechanism, not just the symptom.\n\
             \n\
             Deliver ONE specific insight in 2-3 sentences. Frame it as awareness, NOT medical \
             advice. Connect their specific pattern to the science.\n\
             \n\
             Example tone: \"You've mentioned being tired 3 times this week, always after late \
             nights. Sleep researchers found that each hour of delayed sleep reduces next-day \
             cognitive performance by 25% — it's the adenosine buildup in your prefrontal cortex.\"\n\
             \n\
             If no health patterns found in memory, respond with just \"No health pulse today.\"\n\
             After you're done, call browser_cleanup to free resources.",
        );

        vec![UrgeSpec::new(self.name(), &execute_msg, urgency)
            .with_cooldown("health_pulse:insight")]
    }
}
