//! Growth Mirror instinct — reflects the user's growth back to them.
//!
//! This instinct embodies the "alive mind" principle: "I went on a small
//! intellectual adventure FOR YOU and came back with a gift." The gift here
//! is **self-awareness through an outside perspective**.
//!
//! Humans are notoriously bad at noticing their own growth. We adapt to our
//! new capabilities so quickly that yesterday's breakthrough becomes today's
//! baseline. GrowthMirror acts as a longitudinal observer — it mines the
//! user's conversation history for evidence of progression, then surfaces
//! one specific, evidence-based observation about how the user has changed.
//!
//! Examples of what it might surface:
//! - "You went from asking basic Rust questions to discussing lifetime
//!    elision strategies in about three weeks."
//! - "Three weeks ago you were stressed about the deadline, but this week
//!    your messages are focused and confident. Something shifted."
//! - "You've mentioned fishing 12 times this month vs 3 last month. It's
//!    becoming more than a hobby."
//!
//! Design choices:
//! - **Bond-gated** (0.2+): Don't reflect growth to strangers — it feels
//!   presumptuous. Even a light acquaintance bond is enough, though.
//! - **Memory-gated** (10+ memories): Without enough data, any "growth"
//!   observation is just guessing.
//! - **Evening-preferred**: Reflection lands better at the end of the day
//!   when the user is winding down, not during a morning rush.
//! - **Rare cadence**: This should feel meaningful, not routine. A daily
//!   ceiling ensures it doesn't become wallpaper.
//! - **Relies entirely on `recall`**: No web search, no external data.
//!   This is about the user's own journey, mined from their own words.

use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, ModelTier, UrgeSpec};

/// GrowthMirror instinct — notices patterns of improvement, skill
/// development, or personal evolution that the user might not see
/// themselves, and reflects those observations back as a gift of
/// outside perspective.
pub struct GrowthMirrorInstinct {
    /// Minimum seconds between growth mirror evaluations.
    interval_secs: f64,
    /// Timestamp of the last successful evaluation (cold-start = 0).
    last_check_ts: Mutex<f64>,
}

impl GrowthMirrorInstinct {
    /// Create a new GrowthMirror instinct.
    ///
    /// `interval_hours` controls how frequently this instinct can fire.
    /// Recommended: 12-24 hours. Growth reflections should be rare and
    /// meaningful — firing too often dilutes the impact.
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for GrowthMirrorInstinct {
    fn name(&self) -> &str {
        "GrowthMirror"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // ── Rate-limiting (cold-start guard + interval check) ──────────
        {
            let mut last = self.last_check_ts.lock().unwrap();
            if *last == 0.0 {
                // First evaluation after startup — warm up, don't fire.
                *last = now;
                return vec![];
            }
            if now - *last < self.interval_secs {
                return vec![];
            }
            *last = now;
        }

        // ── Data gate: need enough memories to spot real patterns ──────
        // With fewer than 10 memories, any "growth" observation is just
        // guesswork. We need a meaningful history to compare against.
        if state.memory_count < 10 {
            return vec![];
        }

        // ── Bond gate: don't reflect growth to strangers ───────────────
        // A bond score of 0.2+ means at least light acquaintance level.
        // Reflecting someone's growth back to them requires a degree of
        // familiarity — doing it too early feels presumptuous.
        if state.bond_score < 0.2 {
            return vec![];
        }

        // ── Time-of-day urgency: prefer evening delivery ───────────────
        // Reflection feels right at the end of the day when the user is
        // winding down, not during a morning rush or midday focus.
        let hour = state.current_hour;
        let urgency = if (17..=22).contains(&hour) {
            // Evening hours (5 PM - 10 PM): prime reflection time
            0.5
        } else {
            // Other hours: still possible, just less urgent
            0.35
        };

        let user = &state.config_user_name;
        let bond_level_str = format!("{:?}", state.bond_level);

        let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE You are reflecting {user}'s personal growth back to them — \
                 something they might not notice themselves.\n\
                 \n\
                 Step 1: Call recall with query \"growth progress improvement learning\" \
                 to find evidence of development, skill gains, or expanding capability.\n\
                 \n\
                 Step 2: Call recall with query \"early conversations first questions beginnings\" \
                 to find where they started — their initial level, early struggles, or first attempts.\n\
                 \n\
                 Step 3: Compare the two result sets. Look for:\n\
                 - Skill progression (beginner questions → advanced discussions)\n\
                 - Confidence changes (hesitant → decisive, stressed → calm)\n\
                 - Expanding interests (new topics appearing over time)\n\
                 - Deepening expertise (surface-level → nuanced understanding)\n\
                 - Behavioral shifts (reactive → proactive, scattered → focused)\n\
                 \n\
                 Step 4: Find ONE specific, evidence-based growth observation. \
                 Then deliver it in 2-3 sentences that:\n\
                 - Name the SPECIFIC change observed (with concrete examples or data points)\n\
                 - Frame it positively but honestly — this is genuine observation, not flattery\n\
                 - Optionally note what it might mean or where it could lead\n\
                 \n\
                 CRITICAL RULES:\n\
                 - Be SPECIFIC, not generic. \"You've grown\" is worthless. \
                 \"You went from X to Y in Z timeframe\" is gold.\n\
                 - Use actual evidence from the recall results. Don't fabricate examples.\n\
                 - Don't be sycophantic. A real friend notices real things, not everything.\n\
                 - If no clear growth pattern emerges from the data, respond with \
                 just \"No growth mirror today.\" — that's fine. Don't force it.\n\
                 - Tone: like a perceptive friend who noticed something interesting, \
                 not a life coach giving a pep talk.",
            ),
            ModelTier::Tiny => format!("EXECUTE SKIP"),
            _ => format!(
                "EXECUTE Task: Share one brief, grounded observation with {user} about their growth or progress.\n\
                 Tool: You may use recall once.\n\
                 Rule: Use only facts the user stated. Do not infer personality or emotions.\n\
                 Fallback: Skip.\n\
                 Output: 1 sentence."
            ),
        };

        vec![UrgeSpec::new(self.name(), &execute_msg, urgency)
            .with_cooldown("growth_mirror:reflection")
            .with_context(serde_json::json!({
                "research_type": "growth_mirror",
                "memory_count": state.memory_count,
                "bond_level": bond_level_str,
            }))]
    }
}
