//! Dream Keeper instinct — turns vague aspirations into concrete next steps.
//!
//! Philosophy: "Your dreams deserve more than 'someday'."
//!
//! Searches memory for expressed aspirations — things the user said they want to do,
//! places they want to visit, things they want to build or learn — and researches
//! actionable next steps. The magic is making dreams feel achievable by breaking
//! them into concrete actions with real timelines and costs.

use std::sync::Mutex;

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec, ModelTier};

pub struct DreamKeeperInstinct {
    /// Minimum seconds between dream actionizations.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl DreamKeeperInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for DreamKeeperInstinct {
    fn name(&self) -> &str {
        "DreamKeeper"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Gate: bond must be Friend or above
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        // Gate: need enough memories to find expressed dreams
        if state.memory_count < 15 {
            return vec![];
        }

        // Rate-limit (cold-start guard)
        let now = state.current_ts;
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

        let user = &state.config_user_name;

                let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE Use recall with query \"dream someday want wish hope plan travel build create\" \
             to find aspirations {user} has expressed — places they want to visit, skills they want \
             to learn, things they want to build, experiences they want to have. \
             Pick the ONE most actionable dream. Then use web_search to find specific next steps: \
             costs, timelines, seasonal windows, prerequisites, or opportunities. \
             Compose a message that makes the dream feel concrete and achievable. \
             Examples of good messages: \
             - \"6 months ago you said you wanted to visit Japan. Cherry blossom season is in 7 weeks. \
               Round-trip flights from your area are around $800 right now — that's below average.\" \
             - \"You mentioned wanting to learn piano. There's a method called Simply Piano that gets \
               beginners playing songs in 2 weeks. The investment is 15 minutes a day.\" \
             - \"You've talked about building a cabin someday. The first real step is land — \
               undeveloped lots in your state start around $15K. Want me to look into it?\" \
             Make dreams feel close, not distant. Include specific numbers, dates, or actions. \
             If no suitable dreams are found in memory, respond with \"No dream update today.\" exactly. \
             After you're done, call browser_cleanup to free resources.",
            ),
            ModelTier::Tiny => format!(
                "EXECUTE Remind {user} about one provided goal. If no goal, say: \"Nothing actionable.\" Output: 1 sentence.",
            ),
            _ => format!(
                "EXECUTE Task: Give one useful nudge about a pending goal, plan, or commitment.\n\
             Input: time=(now).\n\
             Tool: Use recall for one pending item.\n\
             Rule: Use only details explicitly stated by the user or returned by recall. Do not invent deadlines, progress, or tasks.\n\
             Fallback: \"Nothing actionable right now.\"\n\
             Output: 1 sentence, under 20 words.",
            ),
        };

        // Urgency bumped slightly — dreams with actionable windows are time-sensitive
        let urgency = 0.4;

        vec![UrgeSpec::new(
            self.name(),
            &execute_msg,
            urgency,
        )
        .with_cooldown("dream_keeper:actionize")]
    }
}
