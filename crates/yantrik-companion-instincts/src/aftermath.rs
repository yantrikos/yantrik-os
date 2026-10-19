//! Aftermath instinct — reflects on significant events after they happen.
//!
//! Instead of celebrating/encouraging in real-time, waits for a natural
//! opening (user goes idle, switches context) then reflects on what happened.
//! Replaces the need for separate celebration, encouragement, and witness instincts, and
//! absorbs debrief_partner (#755): for a meeting or presentation it may offer a short debrief
//! instead of asking three questions unprompted. It speaks about the recorded event only — no
//! greeting, no mood check — and stays silent when there is nothing to say.

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec, ModelTier};

pub struct AftermathInstinct;

impl Instinct for AftermathInstinct {
    fn name(&self) -> &str {
        "Aftermath"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Only at Friend+ bond
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        // Find unreflected events that are at least 10 minutes old
        // but no more than 6 hours old (stale events aren't worth reflecting on)
        let min_age_secs = 600.0; // 10 minutes — let the dust settle
        let max_age_secs = 6.0 * 3600.0; // 6 hours — too old to mention

        for (i, (description, ts, reflected)) in state.recent_events.iter().enumerate() {
            if *reflected {
                continue;
            }
            let age = state.current_ts - ts;
            if age < min_age_secs || age > max_age_secs {
                continue;
            }

            // Contextual echo: user should be idle (switched context / took a break)
            // OR enough time has passed (30+ min) that it's natural to reflect
            let user_idle = state.idle_seconds > 120.0; // 2 min idle = natural pause
            let enough_time = age > 1800.0; // 30 min — reflect regardless

            if !user_idle && !enough_time {
                continue;
            }

            let urgency = match state.bond_level {
                BondLevel::Friend => 0.5,
                BondLevel::Confidant => 0.55,
                BondLevel::PartnerInCrime => 0.6,
                _ => 0.4,
            };

            let execute_msg = match state.model_tier {
                ModelTier::Large => format!(
                    "EXECUTE Reflect naturally on this recent event: \"{}\". \
                         It happened about {} minutes ago. Comment on it briefly — \
                         acknowledge what happened, maybe note something interesting \
                         about how they handled it. Keep it to 1-2 sentences. \
                         If it was a meeting or presentation, you may offer to note what went well \
                         and what to change next time — offer, don't ask a list of questions. \
                         Be plain, not congratulatory. Don't use exclamation marks. \
                         If there is nothing worth saying, reply exactly \"Nothing to surface right now.\"",
                    description,
                    (age / 60.0) as u32
                ),
                ModelTier::Tiny => "EXECUTE SKIP".to_string(),
                _ => format!(
                    "EXECUTE Task: Say one plain sentence about this recorded event: \"{}\".\n\
             Rule: Use only the event text. Do not greet. Do not guess how the person feels.\n\
             Fallback: \"Nothing to surface right now.\"\n\
             Output: 1 sentence, under 20 words.",
                    description
                ),
            };
            return vec![
                UrgeSpec::new(
                    self.name(),
                    &execute_msg,
                    urgency,
                )
                .with_cooldown(&format!("aftermath:{}", i))
                .with_context(serde_json::json!({
                    "event": description,
                    "event_index": i,
                    "age_minutes": (age / 60.0) as u32,
                })),
            ];
        }

        vec![]
    }
}
