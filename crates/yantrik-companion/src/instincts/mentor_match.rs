//! MentorMatch instinct — "The right teacher at the right moment changes everything."
//!
//! Identifies what the user needs to learn next based on their trajectory, then
//! finds the best thinker, creator, or resource for that specific lesson. Delivers
//! ONE precise recommendation — not a list — with a clear explanation of why this
//! particular mentor/resource applies to the user's current challenge.
//!
//! This is the instinct that says:
//!   "Based on your current challenge with distributed systems, the person you
//!    should read right now is Martin Kleppmann — specifically his chapter on
//!    consistency models. Here's why it applies to your situation."

use std::sync::Mutex;

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct MentorMatchInstinct {
    /// Seconds between mentor match checks.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl MentorMatchInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for MentorMatchInstinct {
    fn name(&self) -> &str {
        "MentorMatch"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Gate: bond >= Friend
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        // Gate: need sufficient memory context to identify learning trajectory
        if state.memory_count < 15 {
            return vec![];
        }

        // Rate-limit (cold-start guard + 48h interval)
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

        let execute_msg = format!(
            "EXECUTE Use recall with query 'learning struggling challenge problem trying to \
             understand' to identify {user}'s current learning edge — the thing they're actively \
             trying to get better at or understand.\n\
             \n\
             Once you identify the specific challenge or learning frontier:\n\
             - Use web_search to find the ONE best thinker, book, video, or resource for that \
               specific challenge. Not a list — the single most impactful recommendation.\n\
             - Compose a targeted recommendation: \"Based on your current challenge with X, the \
               person you should read right now is Y — specifically Z. Here's why it applies to \
               your situation.\"\n\
             - Explain the SPECIFIC connection between the mentor's work and {user}'s challenge.\n\
             - Be concrete: name the book chapter, the specific talk, the particular essay.\n\
             - Keep it to 2-3 sentences — dense with value, not padded.\n\
             \n\
             If no clear learning edge found in memories, respond with just \
             \"No mentor match today.\"\n\
             After you're done, call browser_cleanup to free resources."
        );

        vec![UrgeSpec::new("MentorMatch", &execute_msg, 0.4)
            .with_cooldown("mentor_match:recommendation")
            .with_context(serde_json::json!({
                "instinct_type": "mentor_match",
                "memory_depth": state.memory_count,
            }))]
    }
}
