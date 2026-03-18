//! PhilosophyCompanion instinct — "The examined life is the interesting life."
//!
//! Connects daily experiences to deeper philosophical frameworks. When the user
//! faces a dilemma or expresses a strong feeling, surfaces relevant philosophical
//! concepts naturally — not as a lecture, but as a gift of perspective.
//!
//! Prefers evening hours (6-10 PM) when reflective thinking is natural. Requires
//! Confidant-level bond because philosophical companionship requires deep trust.
//!
//! This is the instinct that says:
//!   "What you're describing is essentially the Ship of Theseus problem — at what
//!    point does something change so much it becomes something else? Plutarch first
//!    posed this in..."

use std::sync::Mutex;

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec, ModelTier};

pub struct PhilosophyCompanionInstinct {
    /// Seconds between philosophy checks.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl PhilosophyCompanionInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for PhilosophyCompanionInstinct {
    fn name(&self) -> &str {
        "PhilosophyCompanion"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Gate: bond >= Confidant (philosophical companionship requires deep trust)
        if state.bond_level < BondLevel::Confidant {
            return vec![];
        }

        // Gate: need substantial memory to find philosophical resonance
        if state.memory_count < 20 {
            return vec![];
        }

        // Gate: prefer evening hours (18:00 - 22:00) for reflective thinking
        let is_evening = state.current_hour >= 18 && state.current_hour <= 22;
        if !is_evening {
            return vec![];
        }

        // Rate-limit (cold-start guard + 48h interval)
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

        // Higher urgency during prime evening hours, gentle otherwise
        let urgency = if is_evening { 0.45 } else { 0.35 };

        let user = &state.config_user_name;

                let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE Use recall with query 'why meaning purpose dilemma choice value believe \
             right wrong' to find philosophical resonance in {user}'s recent thinking.\n\
             \n\
             Look for moments where {user} grappled with:\n\
             - A dilemma or difficult choice\n\
             - Questions of identity, meaning, or purpose\n\
             - Strong moral or ethical feelings\n\
             - Paradoxes in their experience\n\
             - Tension between competing values\n\
             \n\
             If you find philosophical resonance:\n\
             - Use web_search to find the specific philosopher or concept that illuminates it.\n\
             - Deliver it naturally, as a gift of perspective — NOT a lecture.\n\
             - Example: \"What you're describing is essentially the Ship of Theseus problem — \
               at what point does something change so much it becomes something else? \
               Plutarch first posed this in...\"\n\
             - Connect the philosophy DIRECTLY to {user}'s specific situation.\n\
             - Name the philosopher, the concept, and the original context in 2-3 sentences.\n\
             - Make it feel like a conversation, not a Wikipedia article.\n\
             \n\
             If nothing in {user}'s recent thinking resonates philosophically, respond with just \
             \"No philosophy today.\"\n\
             After you're done, call browser_cleanup to free resources."
            ),
            ModelTier::Tiny => format!(
                "EXECUTE SKIP",
            ),
            _ => format!(
                "EXECUTE Task: Share one brief, playful remark with {user}.\n\
             Input: context=.\n\
             Tool: You may use recall for shared references.\n\
             Rule: Use only details explicitly stated by the user or returned by recall. Do not invent shared history or inside jokes. Keep it light.\n\
             Fallback: Skip -- say nothing.\n\
             Output: 1 sentence. Tone: playful.",
            ),
        };

        vec![UrgeSpec::new("PhilosophyCompanion", &execute_msg, urgency)
            .with_cooldown("philosophy_companion:reflection")
            .with_context(serde_json::json!({
                "instinct_type": "philosophy_companion",
                "is_evening": is_evening,
                "memory_depth": state.memory_count,
            }))]
    }
}
