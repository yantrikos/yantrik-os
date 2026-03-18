//! Future Self instinct — projects the user's current trajectories forward in time.
//!
//! Philosophy: "If you keep going at this rate, here's where you'll be."
//!
//! Looks at active trajectories — skill learning, projects, habits — and extrapolates
//! where they lead. Can be encouraging ("at this pace, you'll be fluent in 6 weeks")
//! or gently honest ("you've been saying you'll start this for 2 months — at this
//! trajectory, it won't happen without a trigger").
//!
//! The key is honesty with kindness. A good friend tells you the truth about
//! where your current path leads.

use std::sync::Mutex;

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec, ModelTier};

pub struct FutureSelfInstinct {
    /// Minimum seconds between projections.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl FutureSelfInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for FutureSelfInstinct {
    fn name(&self) -> &str {
        "FutureSelf"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Gate: bond must be Friend or above
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        // Gate: need substantial memory to project trajectories
        if state.memory_count < 25 {
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

                let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE Use recall with query \"progress learning goal started project building\" \
             to find active trajectories in {user}'s life — skills being learned, projects in progress, \
             habits being formed or abandoned. \
             Pick the ONE most interesting trajectory and project it forward. \
             Good projections: \
             - \"At the rate you've been learning Rust, you'll be comfortable with async/await in about 3 weeks.\" \
             - \"You've been saying you'll start exercising for 2 months — at this trajectory, it won't happen \
               without a concrete trigger. What if you picked just one day this week?\" \
             - \"Your side project has gotten 3 commits this week after a month of silence. If you keep this \
               momentum, you'll have an MVP in about 4 weeks.\" \
             Be honest but kind. Positive trajectories deserve encouragement. Stalled ones deserve \
             a gentle nudge, not guilt. If there's nothing to project, respond with \
             \"No future projection today.\" exactly.",
            ),
            ModelTier::Tiny => format!(
                "EXECUTE SKIP",
            ),
            _ => format!(
                "EXECUTE Task: Share one brief, grounded observation with {user}.\n\
             Tool: You may use recall once for an explicit past detail.\n\
             Rule: Use only details explicitly stated by the user or returned by recall. Prefer a concrete observation about a stated preference, repeated interest, or recent user-mentioned topic. Do not infer patterns, traits, emotions, or personal growth.\n\
             Fallback: \"No suggestion right now.\"\n\
             Output: 1 sentence.",
            ),
        };

        vec![UrgeSpec::new(
            self.name(),
            &execute_msg,
            0.4,
        )
        .with_cooldown("future_self:projection")]
    }
}
