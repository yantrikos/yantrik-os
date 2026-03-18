//! DecisionLab instinct — "The one factor most people overlook."
//!
//! Activates when the user is deliberating on a decision: buying something, making
//! a career move, choosing between technologies, planning travel, picking a service.
//! DecisionLab does structured comparative research and finds the hidden tradeoff —
//! the factor that most comparison articles and reviews overlook.
//!
//! This instinct embodies the idea that the best decisions aren't made with MORE
//! information, but with the RIGHT information. Most people research the obvious
//! factors and miss the one that actually matters long-term.
//!
//! Example output: "You're deciding between Framework and ThinkPad. Most comparisons
//! focus on specs, but the hidden factor is repairability longevity — Framework parts
//! are guaranteed available for 10 years, Lenovo averages 3."

use std::sync::Mutex;

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec, ModelTier};

pub struct DecisionLabInstinct {
    /// Seconds between decision lab evaluations.
    interval_secs: f64,
    /// Last evaluation timestamp.
    last_check_ts: Mutex<f64>,
}

impl DecisionLabInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for DecisionLabInstinct {
    fn name(&self) -> &str {
        "DecisionLab"
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

        // Gate: need conversation history to detect deliberation
        if state.conversation_turn_count <= 3 {
            return vec![];
        }

        // Gate: bond level — at least Acquaintance
        if state.bond_level < BondLevel::Acquaintance {
            return vec![];
        }

        let user = &state.config_user_name;

                let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE First, use recall with query \"deciding considering choosing between should I option compare\" \
             to find any active decisions {user} is deliberating on.\n\
             \n\
             If you find an active decision:\n\
             1. Identify the specific options being considered\n\
             2. Use web_search to research the ONE factor most people overlook in that decision\n\
             3. Look for: hidden costs, long-term implications, counterintuitive data, \
                the tradeoff that comparison articles don't cover\n\
             \n\
             Deliver a structured but concise insight in 2-3 sentences:\n\
             - Name the decision and options\n\
             - Identify what most comparisons focus on\n\
             - Reveal the hidden factor with specific data or reasoning\n\
             \n\
             Example tone: \"You're deciding between Framework and ThinkPad. Most comparisons \
             focus on specs, but the hidden factor is repairability longevity — Framework parts \
             are guaranteed available for 10 years, Lenovo averages 3.\"\n\
             \n\
             If no active decisions found in memory, respond with just \"No decision lab today.\"\n\
             After you're done, call browser_cleanup to free resources.",
            ),
            ModelTier::Tiny => format!(
                "EXECUTE SKIP",
            ),
            _ => format!(
                "EXECUTE Task: Share one brief, playful remark with {user}.\n\
             Input: context=.\n\
             Tool: You may use recall for shared references.\n\
             Rule: Use only details explicitly stated by the user or returned by recall. Do not invent shared history or inside jokes. Keep it light.\n\
             Fallback: Skip -- say nothing.\n\
             Output: 1 sentence. Tone: playful.",
            ),
        };

        vec![UrgeSpec::new(self.name(), &execute_msg, 0.5)
            .with_cooldown("decision_lab:analysis")]
    }
}
