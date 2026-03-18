//! Humor instinct — at bond level 3+, occasionally suggests callbacks
//! to shared references or inside jokes.

use std::sync::Mutex;

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};

/// Minimum seconds between humor urges (30 minutes).
const HUMOR_COOLDOWN_SECS: f64 = 30.0 * 60.0;

pub struct HumorInstinct {
    last_fire_ts: Mutex<f64>,
}

impl HumorInstinct {
    pub fn new() -> Self {
        Self {
            last_fire_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for HumorInstinct {
    fn name(&self) -> &str {
        "Humor"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Only activate at Friend level or above
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        // Internal rate-limiting (cold-start guard + 30min cooldown)
        {
            let mut last = self.last_fire_ts.lock().unwrap();
            let now = state.current_ts;
            if *last == 0.0 {
                *last = now; // warm up
                return vec![];
            }
            if now - *last < HUMOR_COOLDOWN_SECS {
                return vec![];
            }
            *last = now;
        }

        let mut urges = Vec::new();

        // Interest-aware humor — make jokes relevant to what the user loves.
        // A fisher gets fishing jokes, a coder gets programming jokes, etc.
        let urgency = match state.bond_level {
            BondLevel::Friend => 0.3,
            BondLevel::Confidant => 0.4,
            BondLevel::PartnerInCrime => 0.5,
            _ => 0.2,
        };

        if !state.user_interests.is_empty() {
            // Pick a random interest to joke about (rotate via timestamp)
            let interests = &state.user_interests;
            let idx = (state.current_ts as usize / 60) % interests.len();
            let interest = &interests[idx];

            urges.push(
                UrgeSpec::new(
                    self.name(),
                    &format!(
                        "EXECUTE {}'s interests include {}. First try recall with query \
                         'shared references jokes {}' to find inside jokes. \
                         If you find shared references, make a callback about one in 1 sentence. \
                         If not, come up with a SHORT, actually funny joke or witty observation \
                         related to {}. It MUST be specific to the topic — not generic humor. \
                         Examples of good jokes: fishing puns, coding one-liners, cooking disasters. \
                         Keep it to 1 sentence. Be genuinely funny, not corny. \
                         If you can't think of anything good, say 'nothing to share right now' exactly.",
                        state.config_user_name, interest, interest, interest,
                    ),
                    urgency,
                )
                .with_cooldown("humor:interest"),
            );
        } else if state.shared_references_count > 0 {
            // Fallback: shared references humor if no interests known
            urges.push(
                UrgeSpec::new(
                    self.name(),
                    "EXECUTE You MUST call recall with query 'shared references jokes' first. \
                     Then pick ONE specific result and make a brief callback about it in 1 sentence. \
                     RULES: Do NOT mention rain, weather, loops, or abstract metaphors. \
                     Do NOT be philosophical. Reference a CONCRETE detail from the search results. \
                     If recall returns nothing useful, say 'nothing to share right now' exactly.",
                    urgency,
                )
                .with_cooldown("humor:callback"),
            );
        }

        // Self-deprecating humor opportunity at higher bond levels
        if state.bond_level >= BondLevel::Confidant {
            for pattern in &state.active_patterns {
                if let Some(desc) = pattern.get("description").and_then(|v| v.as_str()) {
                    let lower = desc.to_lowercase();
                    if lower.contains("repeat") || lower.contains("often") || lower.contains("always") {
                        urges.push(
                            UrgeSpec::new(
                                self.name(),
                                &format!(
                                    "EXECUTE Make a brief playful self-deprecating observation \
                                     about this pattern: \"{}\". Keep it to 1 witty sentence.",
                                    desc
                                ),
                                0.35,
                            )
                            .with_cooldown(&format!(
                                "humor:pattern:{}",
                                &desc[..desc.len().min(20)]
                            )),
                        );
                        break; // One pattern joke per cycle
                    }
                }
            }
        }

        urges.into_iter().take(1).collect()
    }
}
