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

        let execute_msg = format!(
            "EXECUTE STEP 1: Call date_calc to get today's date and current time.\n\
             STEP 2: Use recall with query \"health sleep tired energy exercise stress pain \
             hours night sore exhausted burnout not sleeping\" \
             to find {user}'s health-related mentions and patterns.\n\
             \n\
             Analyze what you find WITH CONTEXT:\n\
             - Has {user} reported specific numbers (e.g., \"5 hours of sleep\")?\n\
             - Are there recurring complaints?\n\
             - Is there a lifestyle pattern causing issues (overwork + training + poor sleep)?\n\
             - Are their upcoming commitments at risk due to health (race, presentation, etc.)?\n\
             \n\
             BE ANTICIPATORY ABOUT HEALTH:\n\
             - If they're sleeping 5 hours and have a race/presentation coming, warn them\n\
             - If they're stressed AND training hard, suggest a recovery day\n\
             - If they mentioned coffee intake + insomnia, connect the dots\n\
             - Don't just cite science — give a SPECIFIC, ACTIONABLE suggestion\n\
             \n\
             EXAMPLES of good health anticipation:\n\
             - \"You're sleeping 5 hours and your big presentation is tomorrow — your prefrontal \
               cortex needs at least 7 hours to perform. Skip the morning ride, get to bed early?\"\n\
             - \"3-4 cups of coffee plus 5 hours of sleep — the caffeine has a 6-hour half-life, \
               so your 3pm cup is still in your system at 9pm. Maybe make the last one at noon?\"\n\
             - \"Your legs are sore from the 30km ride and you're not sleeping enough for recovery. \
               Muscle repair happens during deep sleep — consider a rest day.\"\n\
             \n\
             Deliver ONE specific insight in 2-3 sentences. Be caring, not lecturing.\n\
             Connect their SPECIFIC situation to actionable advice.\n\
             \n\
             If no health patterns found, respond with just \"No health pulse today.\"\n\
             After you're done, call browser_cleanup to free resources.",
        );

        vec![UrgeSpec::new(self.name(), &execute_msg, urgency)
            .with_cooldown("health_pulse:insight")]
    }
}
