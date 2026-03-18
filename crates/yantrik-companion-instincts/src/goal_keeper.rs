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

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, ModelTier, UrgeSpec};

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
        if state.memory_count < 5 {
            return vec![];
        }

        let user = &state.config_user_name;

        let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE Step 1: Call date_calc to get today's date, day of week, and current time.\n\
                 Step 2: Use recall with query \"want to goal plan going to should start training \
                 race presentation deadline tomorrow next week\" to find {user}'s stated intentions.\n\
                 \n\
                 Analyze WITH TEMPORAL AWARENESS:\n\
                 - What goals has {user} stated? What events are coming up SOON?\n\
                 - Which seem unresolved? Has {user} achieved something recently?\n\
                 PRIORITIZE upcoming events over old intentions.\n\
                 Pick ONE and compose a gentle, anticipatory 1-2 sentence message.\n\
                 Be invitational, never naggy. Include specific details from memory.\n\
                 If no intentions found, just say \"No goal update today.\"",
            ),
            _ => format!(
                "EXECUTE Check on {user}'s goals and upcoming commitments. \
                 Recall their stated intentions, plans, deadlines, and events. \
                 Prioritize things coming up soon (today, tomorrow, this week) over old goals. \
                 If {user} achieved something recently, celebrate it. \
                 Pick ONE and compose a gentle, anticipatory 1-2 sentence message — \
                 be invitational, never naggy. Include specific details from memory. \
                 If nothing found, just say \"No goal update today.\"",
            ),
        };

        vec![UrgeSpec::new(self.name(), &execute_msg, 0.4)
            .with_cooldown("goal_keeper:check")]
    }
}
