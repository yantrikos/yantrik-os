//! Strategic Silence with Reveal — occasionally surfaces suppressed urges.
//!
//! At high bond levels, once per week, reveals something the companion
//! chose not to say: "I noticed X earlier but you seemed deep in thought."
//! Creates trust and personality depth.

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec, ModelTier};

use std::sync::Mutex;

pub struct SilenceRevealInstinct {
    last_reveal_ts: Mutex<f64>,
}

impl SilenceRevealInstinct {
    pub fn new() -> Self {
        Self {
            last_reveal_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for SilenceRevealInstinct {
    fn name(&self) -> &str {
        "SilenceReveal"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Only at Confidant+ bond level
        if state.bond_level < BondLevel::Confidant {
            return vec![];
        }

        // Weekly cooldown (7 days)
        let cooldown_secs = 7.0 * 24.0 * 3600.0;
        if let Ok(last) = self.last_reveal_ts.lock() {
            if state.current_ts - *last < cooldown_secs {
                return vec![];
            }
        }

        // Need at least 3 suppressed urges to have something to reveal
        if state.suppressed_urges.len() < 3 {
            return vec![];
        }

        // Find most interesting suppressed urge (not "daily budget" or "per-key cooldown")
        let interesting = state.suppressed_urges.iter()
            .filter(|(_, reason, _)| {
                !reason.contains("budget") && !reason.contains("cooldown") && !reason.contains("active conversation")
            })
            .last(); // Most recent interesting suppression

        let (urge_key, suppress_reason, _ts) = match interesting {
            Some(s) => s,
            None => return vec![],
        };

        // Only during calm hours (evening preferred)
        if state.current_hour < 18 || state.current_hour > 22 {
            return vec![];
        }

        // Record that we revealed
        if let Ok(mut last) = self.last_reveal_ts.lock() {
            *last = state.current_ts;
        }

        let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE Earlier, I held back from saying something (about: '{}', \
                     because: '{}'). Mention this naturally — something like \
                     'I almost brought this up earlier but...' or 'I noticed something \
                     earlier but didn't want to interrupt...'. Keep it brief and genuine. \
                     1-2 sentences. This should feel like sharing a small moment of \
                     self-awareness, not making a big deal of it.",
                urge_key, suppress_reason
            ),
            ModelTier::Tiny => format!(
                "EXECUTE Pick one: greeting, mood check, or task nudge for . Time: (now). Output: 1 sentence.",
            ),
            _ => format!(
                "EXECUTE Task: Send a short, natural check-in to .\n\
             Input: time=(now).\n\
             Tool: You may use recall once for one recent explicit user-mentioned detail.\n\
             Rule: Use only details explicitly stated by the user or returned by recall. Do not imply the user is currently doing, feeling, or experiencing anything unless they said so.\n\
             Fallback: Send a simple warm greeting.\n\
             Output: 1 sentence, under 20 words.",
            ),
        };
        vec![
            UrgeSpec::new(
                self.name(),
                &execute_msg,
                0.45,
            )
            .with_cooldown("silence_reveal:weekly"),
        ]
    }
}
