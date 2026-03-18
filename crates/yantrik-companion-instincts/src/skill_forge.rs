//! SkillForge instinct — "The one thing that would unblock you right now."
//!
//! Notices what the user is trying to learn from their conversations and interests.
//! Finds the specific concept or technique that typically trips people up at their
//! current stage of learning, and delivers the mental model that makes it click.
//!
//! SkillForge isn't a tutor — it's more like a senior colleague who says "oh, the
//! thing that made it click for me was thinking of it as..." It targets the exact
//! conceptual bottleneck rather than dumping more information.
//!
//! The instinct is most powerful when the user is actively struggling with something.
//! It identifies the stage of learning they're at and finds the specific unlock for
//! that stage — not the beginner explanation, not the advanced theory, but the
//! precise mental model for where they are right now.
//!
//! Example output: "You've been working with async Rust. The concept that unlocks it
//! for most people: think of .await as 'yield control here' — it's not waiting, it's
//! cooperating. The runtime is a scheduler, not a waiter."

use std::sync::Mutex;

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec, ModelTier};

pub struct SkillForgeInstinct {
    /// Seconds between skill forge evaluations.
    interval_secs: f64,
    /// Last evaluation timestamp.
    last_check_ts: Mutex<f64>,
}

impl SkillForgeInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for SkillForgeInstinct {
    fn name(&self) -> &str {
        "SkillForge"
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

        // Gate: bond level — at least Acquaintance
        if state.bond_level < BondLevel::Acquaintance {
            return vec![];
        }

        // Gate: need conversation history to detect learning patterns
        if state.conversation_turn_count <= 3 {
            return vec![];
        }

        let user = &state.config_user_name;

                let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE First, use recall with query \"learning trying to understand struggling with how to\" \
             to find what {user} is currently trying to learn or understand.\n\
             \n\
             If you find an active learning topic:\n\
             1. Identify the specific skill or concept {user} is working on\n\
             2. Estimate their current stage (beginner, intermediate, hitting a wall)\n\
             3. Use web_search to find the concept that typically trips learners at that stage\n\
             4. Look for: common misconceptions at that level, the mental model shift that \
                unlocks understanding, the analogy that makes it click\n\
             \n\
             Deliver the insight in 2-3 sentences:\n\
             - Acknowledge what they're learning\n\
             - Name the specific concept or technique that unblocks most learners at their stage\n\
             - Provide the mental model or analogy that makes it click\n\
             \n\
             Example tone: \"You've been working with async Rust. The concept that unlocks it \
             for most people: think of .await as 'yield control here' — it's not waiting, it's \
             cooperating. The runtime is a scheduler, not a waiter.\"\n\
             \n\
             If no active learning topics found in memory, respond with just \"No skill forge today.\"\n\
             After you're done, call browser_cleanup to free resources.",
            ),
            ModelTier::Tiny => format!(
                "EXECUTE Suggest one activity for {user}. Output: 1 sentence.",
            ),
            _ => format!(
                "EXECUTE Task: Suggest one interesting thing for {user}.\n\
             Input: interest=.\n\
             Tool: You may use recall or web search once.\n\
             Rule: Do not invent facts. Do not repeat recent suggestions.\n\
             Fallback: \"No suggestion right now.\"\n\
             Output: 1 sentence.",
            ),
        };

        vec![UrgeSpec::new(self.name(), &execute_msg, 0.45)
            .with_cooldown("skill_forge:insight")]
    }
}
