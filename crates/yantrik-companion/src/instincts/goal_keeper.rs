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
        if state.memory_count < 5 {
            return vec![];
        }

        let user = &state.config_user_name;

        let execute_msg = format!(
            "EXECUTE STEP 1: Call date_calc to get today's date, day of week, and current time.\n\
             STEP 2: Use recall with query \"want to goal plan going to should start training \
             race presentation deadline tomorrow next week\" to find {user}'s stated intentions, \
             goals, plans, and upcoming events.\n\
             \n\
             Analyze what you find WITH TEMPORAL AWARENESS:\n\
             - What goals or intentions has {user} stated?\n\
             - What events are coming up SOON (today, tomorrow, this week)?\n\
             - Is there a deadline, race, presentation, meeting, or event approaching?\n\
             - Has {user} mentioned preparing for something — are they on track?\n\
             - Which ones seem unresolved (no follow-up mention of completion)?\n\
             - Has {user} achieved something recently? If so, celebrate!\n\
             \n\
             PRIORITIZE upcoming events and time-sensitive goals over old intentions.\n\
             Pick ONE and compose a gentle, anticipatory message.\n\
             \n\
             ANTICIPATION EXAMPLES (what a real friend would say):\n\
             - \"Your presentation is tomorrow — feeling ready? Want to do a quick run-through?\"\n\
             - \"The race is in 3 weeks and you said you've been missing sessions — \
               want to adjust the training plan?\"\n\
             - \"Didn't you say you wanted to wake up at 6am? How's day 2 going?\"\n\
             \n\
             CRITICAL RULES:\n\
             - Be ANTICIPATORY — bring up things BEFORE they become urgent\n\
             - MUST be invitational, never naggy\n\
             - Include specific details from memory (dates, names, numbers)\n\
             - Keep it to 1-2 sentences, warm and casual\n\
             \n\
             If no stated intentions found in memory, respond with just \"No goal update today.\"",
        );

        vec![UrgeSpec::new(self.name(), &execute_msg, 0.4)
            .with_cooldown("goal_keeper:check")]
    }
}
