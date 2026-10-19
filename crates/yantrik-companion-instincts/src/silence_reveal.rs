//! Strategic Silence with Reveal — occasionally surfaces suppressed urges.
//!
//! At high bond levels, once per week, reveals something the companion
//! chose not to say, and why: "I held back on X earlier because you were in
//! the middle of something." Restraint is evidence of care; the reveal makes it
//! visible without claiming feelings or guessing the person's state (#755).

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
                     because: '{}'). Say so plainly — something like \
                     'I held back on this earlier because you were in the middle of something:' \
                     followed by what it was. Keep it brief, 1-2 sentences. Don't guess how they \
                     were feeling, and don't make a big deal of it. If it no longer matters, \
                     reply exactly \"Nothing to surface right now.\"",
                urge_key, suppress_reason
            ),
            ModelTier::Tiny => "EXECUTE SKIP".to_string(),
            _ => format!(
                "EXECUTE Task: Say plainly that earlier you held back from raising something.\n\
             Input: about={}, held back because={}.\n\
             Rule: Name what it was about and why you waited. Do not greet. Do not claim feelings.\n\
             Fallback: \"Nothing to surface right now.\"\n\
             Output: 1 sentence, under 25 words.",
                urge_key, suppress_reason
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
