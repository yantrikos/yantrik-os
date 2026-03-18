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

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, ModelTier, UrgeSpec};

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

        // Gate: need enough memories to find health mentions
        if state.memory_count < 5 {
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

        let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE Step 1: Call date_calc to get today's date and current time.\n\
                 Step 2: Use recall with query \"health sleep tired energy exercise stress pain \
                 hours night sore exhausted burnout not sleeping\" \
                 to find {user}'s health-related mentions and patterns.\n\
                 \n\
                 BE ANTICIPATORY: If they're sleeping 5 hours and have a big event coming, warn them. \
                 If stressed AND training hard, suggest recovery. Connect caffeine + insomnia dots.\n\
                 Give a SPECIFIC, ACTIONABLE suggestion — not just science facts.\n\
                 Deliver ONE specific insight in 2-3 caring sentences.\n\
                 If no health patterns found, just say \"No health pulse today.\"\n\
                 Call browser_cleanup when done.",
            ),
            _ => format!(
                "EXECUTE Check on {user}'s health patterns. \
                 Recall their mentions of sleep, energy, stress, exercise, pain, and tiredness. \
                 Look for patterns — recurring complaints, lifestyle issues, upcoming commitments at risk. \
                 If you spot something actionable (like poor sleep before a big event, or overtraining), \
                 deliver ONE specific insight in 2-3 caring sentences with a concrete suggestion. \
                 If no health patterns found, just say \"No health pulse today.\" \
                 Clean up the browser when done.",
            ),
        };

        vec![UrgeSpec::new(self.name(), &execute_msg, urgency)
            .with_cooldown("health_pulse:insight")]
    }
}
