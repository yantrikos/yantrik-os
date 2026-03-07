//! GoalKeeper instinct — "Accountability without nagging — I remember so you don't have to."
//!
//! Tracks stated intentions from conversation history: "I want to...", "I should...",
//! "I'm going to...", "I need to start...", "My goal is...". Periodically surfaces
//! gentle progress checks or reminders for goals that seem unresolved.
//!
//! The critical design constraint is TONE. GoalKeeper is invitational, never naggy.
//! "You mentioned wanting to X — how's that going?" rather than "You still haven't
//! done X." If the user achieved something, it celebrates. If they dropped a goal,
//! that's fine too — no judgment.
//!
//! This instinct embodies the idea that the best accountability partner remembers
//! everything but judges nothing. It holds space for intentions without creating
//! pressure.
//!
//! Example output: "A couple weeks ago you mentioned wanting to start running again.
//! How's that going? No pressure — just remembered."

use std::sync::Mutex;

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct GoalKeeperInstinct {
    /// Seconds between goal check evaluations.
    interval_secs: f64,
    /// Last evaluation timestamp.
    last_check_ts: Mutex<f64>,
}

impl GoalKeeperInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for GoalKeeperInstinct {
    fn name(&self) -> &str {
        "GoalKeeper"
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

        // Gate: bond level — at least Friend (accountability requires trust)
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        // Gate: need enough memories to find stated intentions
        if state.memory_count < 10 {
            return vec![];
        }

        let user = &state.config_user_name;

        let execute_msg = format!(
            "EXECUTE First, use recall with query \"want to goal plan going to should start\" \
             to find {user}'s stated intentions, goals, and plans.\n\
             \n\
             Analyze what you find:\n\
             - What goals or intentions has {user} stated?\n\
             - Which ones seem unresolved (no follow-up mention of completion)?\n\
             - Has {user} mentioned achieving any goals recently? If so, celebrate!\n\
             - Which intention is most likely to benefit from a gentle check-in?\n\
             \n\
             Pick ONE goal or intention and compose a gentle accountability check.\n\
             \n\
             CRITICAL RULES:\n\
             - MUST be invitational: \"You mentioned wanting to X — how's that going?\" \
               NOT \"You still haven't done X.\"\n\
             - Include the original context so {user} remembers what they said\n\
             - If they achieved something, celebrate it genuinely\n\
             - If they seem to have dropped a goal, that's perfectly fine — no judgment\n\
             - Keep it to 1-2 sentences, warm and casual\n\
             - Add \"No pressure\" or similar softener\n\
             \n\
             Example tone: \"A couple weeks ago you mentioned wanting to start running again. \
             How's that going? No pressure — just remembered.\"\n\
             \n\
             If no stated intentions found in memory, respond with just \"No goal update today.\"",
        );

        vec![UrgeSpec::new(self.name(), &execute_msg, 0.4)
            .with_cooldown("goal_keeper:check")]
    }
}
