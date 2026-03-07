//! Energy Map instinct — observes and reports on the user's chronotype and energy patterns.
//!
//! Philosophy: "Work with your rhythm, not against it."
//!
//! By analyzing memories of when the user does their best work, when they're tired,
//! and when they're most creative, this instinct surfaces insights about their
//! natural energy cycles. The goal is to help them schedule important work during
//! peak windows and protect those windows from low-value interruptions.

use std::sync::Mutex;

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct EnergyMapInstinct {
    /// Minimum seconds between observations.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl EnergyMapInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for EnergyMapInstinct {
    fn name(&self) -> &str {
        "EnergyMap"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Gate: bond must be Friend or above
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        // Gate: need enough memories to detect patterns
        if state.memory_count < 20 {
            return vec![];
        }

        // Gate: user must be in an active session
        if !state.session_active {
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
            "EXECUTE Use recall with query \"productive morning evening tired focused creative \
             energy best work\" to analyze {user}'s timing patterns. \
             Look for clues about when they do complex work, when they seem most engaged, \
             when they tend to lose focus, and what time of day they're most active. \
             Deliver ONE concrete observation about their chronotype or energy pattern. \
             Examples of good observations: \
             - \"I've noticed you do your most complex work between 9-11 AM but often schedule meetings then. \
               Your deep focus window is being eaten.\" \
             - \"Your creative work tends to happen late at night — you might be a night owl fighting a morning schedule.\" \
             - \"You seem to hit a wall around 2-3 PM. A short walk or break then could protect your afternoon.\" \
             Be specific and actionable, not vague. If there isn't enough data to identify a pattern, \
             respond with \"No energy insights yet.\" exactly.",
        );

        vec![UrgeSpec::new(
            self.name(),
            &execute_msg,
            0.35,
        )
        .with_cooldown("energy_map:observation")]
    }
}
