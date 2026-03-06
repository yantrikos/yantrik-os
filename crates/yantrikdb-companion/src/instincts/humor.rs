//! Humor instinct — at bond level 3+, occasionally suggests callbacks
//! to shared references or inside jokes.

use std::sync::Mutex;

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

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

        // Surface shared references as callbacks — uses EXECUTE so the LLM
        // actually recalls inside jokes and crafts a real joke/callback.
        if state.shared_references_count > 0 {
            let urgency = match state.bond_level {
                BondLevel::Friend => 0.3,
                BondLevel::Confidant => 0.4,
                BondLevel::PartnerInCrime => 0.5,
                _ => 0.2,
            };

            urges.push(
                UrgeSpec::new(
                    self.name(),
                    "EXECUTE You MUST call memory_search with query 'shared references jokes' first. \
                     Then pick ONE specific result and make a brief callback about it in 1 sentence. \
                     RULES: Do NOT mention rain, weather, loops, or abstract metaphors. \
                     Do NOT be philosophical. Reference a CONCRETE detail from the search results. \
                     If memory_search returns nothing useful, say 'nothing to share right now' exactly.",
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
