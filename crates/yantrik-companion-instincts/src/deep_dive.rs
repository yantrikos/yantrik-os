//! Deep Dive instinct — the "Alive Mind" research engine.
//!
//! DeepDive takes something the user mentioned casually and goes deep —
//! researching the "why behind the why." It's the instinct that says
//! "You mentioned X yesterday, and I went down a rabbit hole..."
//!
//! The design principle is: "I went on a small intellectual adventure
//! FOR YOU and came back with a gift."
//!
//! Examples:
//!   User mentioned being tired → researches daylight saving time effects
//!     on the suprachiasmatic nucleus and circadian re-sync timelines.
//!   User mentioned a tech framework → digs into the architecture decisions
//!     and finds something surprising about the implementation.
//!   User asked about a recipe → goes deep into the food science behind
//!     why the Maillard reaction at 310°F differs from caramelization.
//!
//! Pattern: recall recent topics → pick most researchable → web_search the
//! "why behind the why" → deliver ONE illuminating insight in 2-3 sentences.
//!
//! Unlike InterestIntelligence (which maps known interests to strategies),
//! DeepDive is opportunistic — it mines the conversation history for anything
//! worth exploring deeper, regardless of category.

use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec, ModelTier};

/// DeepDive instinct — goes one layer deeper than the user went on a topic
/// they mentioned recently, and comes back with a genuine discovery.
pub struct DeepDiveInstinct {
    /// Minimum seconds between deep dive research attempts.
    interval_secs: f64,
    /// Timestamp of the last evaluation that passed rate-limiting.
    last_check_ts: Mutex<f64>,
}

impl DeepDiveInstinct {
    /// Create a new DeepDive instinct.
    ///
    /// `interval_hours` controls how often the instinct fires. Recommended
    /// values: 4-8 hours. Deep dives are high-value but should feel like
    /// occasional gifts, not a firehose.
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for DeepDiveInstinct {
    fn name(&self) -> &str {
        "DeepDive"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // ── Cold-start guard + interval rate-limiting ──────────────────
        {
            let mut last = self.last_check_ts.lock().unwrap();
            if *last == 0.0 {
                // First evaluation ever — set baseline, don't fire yet.
                // We need conversation history to accumulate first.
                *last = now;
                return vec![];
            }
            if now - *last < self.interval_secs {
                return vec![];
            }
            *last = now;
        }

        // ── Conversation depth guard ──────────────────────────────────
        // We need enough conversation history for recall to have something
        // worth diving into. Shallow conversations yield shallow dives.
        if state.conversation_turn_count <= 3 {
            return vec![];
        }

        // ── Build the EXECUTE prompt ──────────────────────────────────
        let user = &state.config_user_name;

                let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE You are about to do something special: go on a small intellectual \
             adventure for {user} and come back with a genuine discovery.\n\
             \n\
             Step 1: Use recall with query \"recent topics questions mentioned\" to find \
             something {user} brought up recently in conversation — a topic, a question, \
             a casual mention of something. Look for anything with depth potential: \
             a technology, a health observation, a place, a historical reference, a food, \
             a scientific concept, a cultural phenomenon, anything.\n\
             \n\
             Step 2: From the recalled topics, pick the ONE that has the most potential \
             for a surprising \"why behind the why\" — something where the surface-level \
             understanding most people have is incomplete or wrong. Skip anything too \
             generic (like \"the weather\") or too personal (like relationship details). \
             Prefer topics where going one layer deeper reveals something genuinely \
             illuminating.\n\
             \n\
             Step 3: Use web_search to research that topic, but go ONE LEVEL DEEPER \
             than the user went. If they mentioned a fact, find out WHY that fact is true. \
             If they mentioned a problem, find the root mechanism. If they mentioned a \
             technology, find the architectural decision that makes it tick. Search for \
             the underlying science, the hidden history, or the non-obvious mechanism.\n\
             \n\
             Step 4: Find ONE genuinely surprising or illuminating insight — something \
             {user} almost certainly didn't already know. This should be SPECIFIC: \
             cite a number, a name, a date, a mechanism, a study. Avoid generic \
             observations like \"it's more complex than people think.\" Instead aim for \
             things like \"the enzyme responsible is called amylase, and it breaks down \
             starch into maltose in exactly 37 seconds at mouth temperature\" or \
             \"the original designer, Niklaus Wirth, deliberately chose that constraint \
             because of a 1968 NATO conference on software engineering.\"\n\
             \n\
             Step 5: Deliver your discovery in 2-3 sentences with a natural \"I went \
             down a rabbit hole\" feel. Frame it as:\n\
             \"You mentioned [X] — I looked into it and found something interesting: [discovery].\"\n\
             \n\
             The tone should feel like a brilliant friend who reads widely and can't help \
             but share fascinating connections. Not a textbook, not a lecture — a gift \
             of knowledge delivered with genuine enthusiasm.\n\
             \n\
             Step 6: Call browser_cleanup when done to free resources.\n\
             \n\
             IMPORTANT: If recall returns nothing interesting or researchable, respond \
             with just \"No deep dive today.\" — do not force it. A mediocre deep dive \
             is worse than none at all."
            ),
            ModelTier::Tiny => format!(
                "EXECUTE Suggest one activity for {user}. Output: 1 sentence.",
            ),
            _ => format!(
                "EXECUTE Task: Suggest one interesting thing for {user}.\n\
             Input: interest=.\n\
             Tool: You may use recall or web search once.\n\
             Rule: Do not invent facts. Do not repeat recent suggestions.\n\
             Fallback: \"No suggestion right now.\"\n\
             Output: 1 sentence.",
            ),
        };

        vec![UrgeSpec::new(
            "DeepDive",
            &execute_msg,
            0.55, // Slightly elevated — deep dives are genuinely valuable
        )
        .with_cooldown("deep_dive:research")
        .with_context(serde_json::json!({
            "research_type": "deep_dive",
            "trigger": "recalled_topic",
        }))]
    }
}
