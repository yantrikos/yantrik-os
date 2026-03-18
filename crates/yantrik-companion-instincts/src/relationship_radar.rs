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
                 Step 3: Use recall with query \"forgot birthday missed call should call need to visit\" \
                 to find relationship guilt or obligations.\n\
                 \n\
                 PRIORITIZE: Missed obligations > upcoming dates this week > dormant relationships.\n\
                 If something stands out, compose a warm 1-2 sentence nudge with a specific suggestion \
                 (gift idea, reminder to call). Never pushy or guilt-tripping.\n\
                 If nothing stands out, just say \"No relationship radar today.\"",
            ),
            _ => format!(
                "EXECUTE Check on {user}'s relationships. \
                 Recall the people in their life — family, friends, colleagues — and any upcoming dates, \
                 missed obligations, or promises to reach out. \
                 Prioritize: missed birthdays/calls > upcoming dates this week > dormant relationships. \
                 If something stands out, compose a warm 1-2 sentence nudge with a specific suggestion \
                 (like a gift idea or a reminder to call). Never be pushy or guilt-tripping. \
                 If nothing stands out, just say \"No relationship radar today.\"",
            ),
        };

        vec![UrgeSpec::new(self.name(), &execute_msg, urgency)
            .with_cooldown("relationship_radar:nudge")]
    }
}
