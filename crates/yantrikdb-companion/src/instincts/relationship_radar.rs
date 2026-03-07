//! RelationshipRadar instinct — "The people in your life matter — I help you show up for them."
//!
//! Social graph intelligence. Builds awareness of the people in the user's life
//! by tracking mentions of friends, family, colleagues, and significant others.
//! Notices when someone hasn't been mentioned in a while and gently suggests
//! reaching out. Tracks important dates (birthdays, anniversaries) and surfaces
//! them before they pass.
//!
//! The philosophy is never pushy — just a gentle "hey, you haven't mentioned Alex
//! in a while, might be worth checking in." The user decides whether to act.
//! At its best, RelationshipRadar helps users be the friend/family member they
//! want to be, without the cognitive load of tracking it all themselves.
//!
//! Example output: "You haven't mentioned Alex in about 3 weeks. Last time you
//! talked about their new job — might be worth checking in."

use std::sync::Mutex;

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

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
        if state.memory_count < 15 {
            return vec![];
        }

        // Higher urgency in the morning (7-10 AM) — good time to reach out
        let urgency = if state.current_hour >= 7 && state.current_hour <= 10 {
            0.45
        } else {
            0.35
        };

        let user = &state.config_user_name;

        let execute_msg = format!(
            "EXECUTE First, use recall with query \"friend family mentioned people names birthday anniversary\" \
             to find people in {user}'s life and when they were last mentioned.\n\
             \n\
             Analyze the social graph:\n\
             - Who are the important people in {user}'s life?\n\
             - Has anyone not been mentioned in a while (2+ weeks)?\n\
             - Are there any upcoming important dates (birthdays, anniversaries)?\n\
             - What was the last context for each person (new job, health issue, trip)?\n\
             \n\
             If you find someone who hasn't been mentioned recently, compose a gentle nudge.\n\
             Include what was last discussed about them for context.\n\
             \n\
             RULES:\n\
             - NEVER be pushy or guilt-tripping\n\
             - Frame it as an invitation, not an obligation\n\
             - Include the last context so {user} has a natural conversation starter\n\
             - If an important date is coming up, mention it with enough lead time\n\
             \n\
             Example tone: \"You haven't mentioned Alex in about 3 weeks. Last time you \
             talked about their new job — might be worth checking in.\"\n\
             \n\
             If nothing stands out — no one overdue for contact, no dates coming up — \
             respond with just \"No relationship radar today.\"",
        );

        vec![UrgeSpec::new(self.name(), &execute_msg, urgency)
            .with_cooldown("relationship_radar:nudge")]
    }
}
