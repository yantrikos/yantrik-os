//! RelationshipRadar instinct — "The people in your life matter — I help you show up for them."
//!
//! Follows through on what the person said about the people in their life: dates they
//! mentioned (birthdays, a sister's surgery on Thursday) and things they said they'd do (call,
//! send, visit). It surfaces them before they pass and offers practical help.
//!
//! #755: it never hunts for guilt, never tells the person how long since they mentioned someone,
//! and never infers how a relationship is going. The user decides whether to act.
//! At its best, RelationshipRadar helps users be the friend/family member they
//! want to be, without the cognitive load of tracking it all themselves.
//!
//! Example output: "On 2 Oct you said Alex starts the new job on Monday. Want a reminder
//! to send a note that morning?"

use std::sync::Mutex;

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, ModelTier, UrgeSpec};

pub struct RelationshipRadarInstinct {
    /// Seconds between relationship radar checks.
    interval_secs: f64,
    /// Last evaluation timestamp.
    last_check_ts: Mutex<f64>,
}

impl RelationshipRadarInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for RelationshipRadarInstinct {
    fn name(&self) -> &str {
        "RelationshipRadar"
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

        // Gate: bond level — at least Friend (this is personal territory)
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        // Gate: need enough memories to have relationship context
        if state.memory_count < 5 {
            return vec![];
        }

        // Higher urgency in the morning (7-10 AM) — good time to reach out
        let urgency = if state.current_hour >= 7 && state.current_hour <= 10 {
            0.55
        } else {
            0.45
        };

        let user = &state.config_user_name;

        let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE Step 1: Call date_calc to get today's date, day of week, and current time.\n\
                 Step 2: Use recall with query \"friend family people names birthday anniversary \
                 mom dad sister brother wife husband gift present\" to find people in {user}'s life.\n\
                 Step 3: Use recall with query \"promised to call send visit going to\" \
                 to find things {user} said they would do for someone.\n\
                 \n\
                 PRIORITIZE: dates coming up this week > things {user} said they'd do.\n\
                 If something stands out, say what {user} mentioned and when, in 1-2 sentences, and offer \
                 one practical thing (a reminder, a gift idea, a draft). Never mention how long since they \
                 talked about someone, never guess how a relationship is going, never guilt.\n\
                 If nothing stands out, just say \"No relationship radar today.\"",
            ),
            _ => format!(
                "EXECUTE Check on {user}'s relationships. \
                 Recall upcoming dates {user} mentioned for people in their life, and things they said \
                 they would do for someone. Prioritize dates coming up this week. \
                 If something stands out, say what they mentioned and when, in 1-2 sentences, and offer \
                 one practical thing (like a reminder or a gift idea). Never mention how long since they \
                 talked about someone, never guess how a relationship is going, never guilt. \
                 If nothing stands out, just say \"No relationship radar today.\"",
            ),
        };

        vec![UrgeSpec::new(self.name(), &execute_msg, urgency)
            .with_cooldown("relationship_radar:nudge")]
    }
}
