//! Cultural Radar instinct — discovers new content and creators matching the user's taste.
//!
//! Philosophy: "There's something new in the world that's exactly your taste."
//!
//! Unlike generic recommendation engines, Cultural Radar understands the intersection
//! of the user's interests and finds the ONE thing that sits right at that crossroads.
//! Not "here are 10 popular podcasts" but "based on your love of dry humor + history,
//! this new podcast by X is exactly your vibe."

use std::sync::Mutex;

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};

pub struct CulturalRadarInstinct {
    /// Minimum seconds between discoveries.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl CulturalRadarInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for CulturalRadarInstinct {
    fn name(&self) -> &str {
        "CulturalRadar"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Gate: need known interests to match against
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

        // Pick one interest axis to explore (rotate via timestamp)
        let interests = &state.user_interests;
        let idx = (now as usize / 3600) % interests.len();
        let interest = &interests[idx];

        let execute_msg = format!(
            "EXECUTE Use recall with query \"{interest} preferences taste style\" to understand \
             what specifically {user} likes about {interest} — their particular taste within it. \
             Then use web_search for \"new {interest} releases trending 2026\" or similar to find \
             new releases, trending content, emerging creators, or hidden gems in that space. \
             Find the ONE thing that matches their taste perfectly — a new album, book, show, \
             creator, tool, game, or experience. \
             Include WHY it matches: \"Based on your love of X + Y, this new Z by W is exactly your vibe.\" \
             The recommendation must feel personal, not algorithmic. Connect at least two things \
             you know about them to explain the match. \
             If nothing genuinely good surfaces, respond with \"No cultural radar today.\" exactly. \
             After you're done, call browser_cleanup to free resources.",
        );

        vec![UrgeSpec::new(
            self.name(),
            &execute_msg,
            0.35,
        )
        .with_cooldown("cultural_radar:discovery")]
    }
}
