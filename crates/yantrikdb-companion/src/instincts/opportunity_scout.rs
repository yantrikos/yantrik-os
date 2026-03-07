//! Opportunity Scout instinct — finds real-world opportunities matching the user's profile.
//!
//! Philosophy: "The world has something for you — let me find it."
//!
//! Combines knowledge of the user's skills and interests with web research to find
//! opportunities they'd never search for themselves: hackathons, grants, competitions,
//! conferences, open source projects needing contributors, job openings, fellowships,
//! or community events. Each opportunity includes a clear explanation of why it fits.

use std::sync::Mutex;

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct OpportunityScoutInstinct {
    /// Minimum seconds between opportunity searches.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl OpportunityScoutInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for OpportunityScoutInstinct {
    fn name(&self) -> &str {
        "OpportunityScout"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Gate: need known interests to match opportunities against
        if state.user_interests.is_empty() {
            return vec![];
        }

        // Gate: bond must be Acquaintance or above
        if state.bond_level < BondLevel::Acquaintance {
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
        let interests_str = state.user_interests.join(", ");

        let location_context = if state.user_location.is_empty() {
            String::new()
        } else {
            format!(
                " Also check for local opportunities near {}.",
                state.user_location
            )
        };

        let execute_msg = format!(
            "EXECUTE Use recall with query \"skills experience work projects interests\" \
             to build a profile of {user}'s skills and interests. \
             Their known interests include: {interests_str}. \
             Then use web_search for opportunities matching their profile — search for things like: \
             hackathons, grants, competitions, conferences, open source projects seeking contributors, \
             community events, workshops, or fellowships related to their skills.{location_context} \
             Find ONE opportunity with high fit confidence. Include why it matches: \
             - \"There's a Rust hackathon in your city next month — they specifically want OS-level \
               projects. Fits your Yantrik work perfectly.\" \
             - \"The Mozilla Foundation has an open grant for privacy-focused tools. Your background \
               in X + Y makes you a strong candidate. Deadline is in 3 weeks.\" \
             - \"This open source project needs help with exactly the kind of systems programming \
               you do. They have 2K stars and active mentorship.\" \
             Be specific: include dates, deadlines, links context, and a clear reason for fit. \
             If nothing with genuine high fit is found, respond with \
             \"No opportunities spotted today.\" exactly. \
             After you're done, call browser_cleanup to free resources.",
        );

        vec![UrgeSpec::new(
            self.name(),
            &execute_msg,
            0.4,
        )
        .with_cooldown("opportunity_scout:find")]
    }
}
