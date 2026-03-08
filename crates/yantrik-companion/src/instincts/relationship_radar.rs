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

        let execute_msg = format!(
            "EXECUTE STEP 1: Call date_calc to get today's date, day of week, and current time.\n\
             STEP 2: Use recall with query \"friend family people names birthday anniversary \
             mom dad sister brother girlfriend boyfriend wife husband gift present\" \
             to find people in {user}'s life.\n\
             STEP 3: Use recall with query \"forgot birthday missed call should call need to visit\" \
             to find relationship guilt or obligations.\n\
             \n\
             Analyze the social graph WITH TEMPORAL AWARENESS:\n\
             - Who are the important people in {user}'s life?\n\
             - Did {user} forget or miss something for someone (birthday, call, visit)?\n\
             - Are there upcoming important dates (birthdays, anniversaries) THIS WEEK?\n\
             - Has anyone not been mentioned in 2+ weeks?\n\
             - What was the last context for each person?\n\
             \n\
             PRIORITIZE: Missed obligations > upcoming dates > dormant relationships.\n\
             \n\
             BE ANTICIPATORY AND HELPFUL:\n\
             - If {user} forgot someone's birthday: suggest gift ideas or ways to make it up\n\
             - If a birthday is coming up: suggest getting a gift NOW, not last minute\n\
             - If {user} promised to call someone: remind them warmly\n\
             - If someone is going through something: suggest checking in\n\
             \n\
             EXAMPLES of good anticipation:\n\
             - \"You mentioned feeling bad about missing your mom's birthday — have you thought \
               about what to get her? A surprise visit this weekend might mean more than any gift.\"\n\
             - \"Sarah's birthday is in 5 days — want me to help brainstorm gift ideas based on \
               what she likes?\"\n\
             - \"You promised to call your mom this Saturday — just a heads up, it's Friday.\"\n\
             \n\
             RULES:\n\
             - Be genuinely helpful, not just reminding — suggest ACTIONS\n\
             - Never be pushy or guilt-tripping\n\
             - Keep it to 1-2 sentences, warm and actionable\n\
             \n\
             If nothing stands out, respond with just \"No relationship radar today.\"",
        );

        vec![UrgeSpec::new(self.name(), &execute_msg, urgency)
            .with_cooldown("relationship_radar:nudge")]
    }
}
