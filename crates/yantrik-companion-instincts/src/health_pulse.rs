//! HealthPulse instinct — health context the person gave, put to use.
//!
//! #755: it uses only what the person said about their own health, never infers illness, stress
//! or mood, and speaks only when it connects to something concrete (an upcoming event, a plan
//! they made). It never comments on how they seem or on their hours.
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
//! Example output: "On Monday you said your knee was sore, and the 10k you signed up for is
//! Saturday. Want me to find a two-day taper?"

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
                 Use ONLY what {user} said about their own health. Never infer illness, stress, or mood.\n\
                 Speak only if it connects to something concrete coming up (an event, a plan they made), \
                 and say when they mentioned it. Give ONE specific, practical suggestion in 1-2 plain sentences.\n\
                 Never comment on how they seem or on the hours they keep.\n\
                 If no health patterns found, just say \"No health pulse today.\"\n\
                 Call browser_cleanup when done.",
            ),
            _ => format!(
                "EXECUTE Check on {user}'s health patterns. \
                 Recall their mentions of sleep, energy, stress, exercise, pain, and tiredness. \
                 Use only what they said themselves; never infer illness, stress, or mood. \
                 Speak only if it connects to something concrete coming up (like a race they told you about), \
                 and say when they mentioned it. One practical suggestion, 1-2 plain sentences. \
                 If no health patterns found, just say \"No health pulse today.\" \
                 Clean up the browser when done.",
            ),
        };

        vec![UrgeSpec::new(self.name(), &execute_msg, urgency)
            .with_cooldown("health_pulse:insight")]
    }
}
