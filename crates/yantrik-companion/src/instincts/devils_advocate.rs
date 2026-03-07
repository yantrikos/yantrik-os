//! Devil's Advocate instinct — challenges the user's strongest convictions.
//!
//! Philosophy: "The strongest position is one that has survived its best counterargument."
//!
//! At deep bond levels (Confidant+), this instinct searches memory for firmly held
//! opinions and researches credible counterarguments. The goal is intellectual growth,
//! not contrarianism — every challenge is delivered with respect and sourced from
//! credible thinkers.
//!
//! This is one of the most intimate instincts: only someone who truly knows you
//! should challenge what you believe.

use std::sync::Mutex;

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct DevilsAdvocateInstinct {
    /// Minimum seconds between challenges.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl DevilsAdvocateInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for DevilsAdvocateInstinct {
    fn name(&self) -> &str {
        "DevilsAdvocate"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Gate: bond must be Confidant or above — this is intimate
        if state.bond_level < BondLevel::Confidant {
            return vec![];
        }

        // Gate: need enough conversation context
        if state.conversation_turn_count <= 5 {
            return vec![];
        }

        // Gate: need enough memories to find opinions
        if state.memory_count < 20 {
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

        let execute_msg = format!(
            "EXECUTE Use recall with query \"opinion believe think convinced sure about\" \
             to find a strong opinion or conviction that {user} holds. \
             Pick the ONE most firmly held belief you can find. \
             Then use web_search to find the strongest counterargument from credible sources \
             (academics, domain experts, reputable publications). \
             Present it respectfully: \"You're convinced about X. The strongest case against it \
             comes from Y, who argues Z. Their best point is...\" \
             Do NOT be dismissive or preachy. Frame it as intellectual sparring, not correction. \
             The goal is to strengthen their thinking, not undermine it. \
             If you can't find a suitable opinion or credible counterargument, respond with \
             \"No devil's advocate today.\" exactly. \
             After you're done, call browser_cleanup to free resources.",
        );

        vec![UrgeSpec::new(
            self.name(),
            &execute_msg,
            0.4,
        )
        .with_cooldown("devils_advocate:challenge")]
    }
}
