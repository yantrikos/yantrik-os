//! Second Brain instinct — your subconscious, externalized.
//!
//! SecondBrain acts as an external memory and synthesis engine. It periodically
//! reviews the user's recent memories and conversations to find patterns,
//! contradictions, forgotten commitments, and emerging themes the user might
//! not notice on their own.
//!
//! The design principle is: "I went on a small intellectual adventure FOR YOU
//! and came back with a gift" — the gift of SELF-KNOWLEDGE through pattern
//! recognition. The instinct operates purely on the user's own data via `recall`,
//! never reaching out to the web. It observes without prescribing: "I noticed X"
//! rather than "You should Y."
//!
//! Analysis rotates through six modes on each activation:
//! - Recurring themes and desires (what keeps coming up?)
//! - Contradictions and tensions (where do stated goals conflict with actions?)
//! - Forgotten commitments and follow-ups (what was promised but never revisited?)
//! - Energy and mood patterns (when is engagement highest vs. most flat?)
//! - Evolving opinions and changed perspectives (where has the user shifted?)
//! - Unfinished threads worth revisiting (what conversations stopped mid-thought?)
//!
//! Bond and memory gates ensure the instinct only fires when there is enough
//! data to analyze and enough trust to surface potentially uncomfortable truths.

use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, ModelTier, UrgeSpec};

/// The six lenses through which SecondBrain examines the user's memory.
/// Each activation picks the next mode in rotation, ensuring broad coverage
/// over time rather than fixating on a single analytical angle.
const ANALYSIS_MODES: &[&str] = &[
    "recurring themes and desires",
    "contradictions and tensions",
    "forgotten commitments and follow-ups",
    "energy and mood patterns",
    "evolving opinions and changed perspectives",
    "unfinished threads worth revisiting",
];

/// The Alive Mind instinct — an externalized subconscious that surfaces
/// self-knowledge the user might not arrive at on their own.
///
/// Unlike other instincts that react to external events (news, weather, deals),
/// SecondBrain turns inward. It mines the user's own memories, conversations,
/// and behavioral traces to find signal in the noise of daily life.
///
/// Think of it as a therapist who has perfect recall of everything you've ever
/// said, but who only speaks when they notice something genuinely worth
/// pointing out — and even then, they observe without prescribing.
pub struct SecondBrainInstinct {
    /// Minimum seconds between analyses.
    interval_secs: f64,
    /// Timestamp of the last analysis check (cold-start guard).
    last_check_ts: Mutex<f64>,
    /// Index into ANALYSIS_MODES, rotated on each activation.
    analysis_index: Mutex<usize>,
}

impl SecondBrainInstinct {
    /// Create a new SecondBrain instinct.
    ///
    /// `interval_hours` controls how often the instinct fires. Recommended:
    /// 4-8 hours. Too frequent and the insights feel forced; too rare and
    /// patterns slip through before they're noticed.
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
            analysis_index: Mutex::new(0),
        }
    }

    /// Build the EXECUTE prompt for a given analysis mode.
    ///
    /// The prompt instructs the LLM to use multiple `recall` queries to build
    /// a comprehensive picture of the user's memory landscape, then analyze
    /// that picture through the lens of the current mode. The tone is
    /// observational — almost clinical — like a good therapist who points
    /// things out without pushing.
    fn build_execute_prompt(&self, mode: &str, state: &CompanionState) -> String {
        let user = &state.config_user_name;

        // Mode-specific recall queries and analysis instructions
        let (recall_queries, analysis_guide) = match mode {
            "recurring themes and desires" => (
                format!(
                    "First, call recall with query \"things {user} wants to do goals desires\". \
                     Then call recall with query \"recurring topics interests hobbies\". \
                     Then call recall with query \"mentioned wanting to plans aspirations\"."
                ),
                format!(
                    "Look for desires or goals that appear MULTIPLE times across different \
                     conversations. Pay special attention to things {user} keeps mentioning \
                     but never acts on — recurring desires that get deferred often point to \
                     something important. Also note topics that keep surfacing even when the \
                     conversation is about something else."
                ),
            ),
            "contradictions and tensions" => (
                format!(
                    "First, call recall with query \"goals priorities focus direction\". \
                     Then call recall with query \"excited about new ideas projects\". \
                     Then call recall with query \"frustrated overwhelmed stressed about\"."
                ),
                format!(
                    "Look for places where {user}'s stated goals CONFLICT with their actions \
                     or other stated goals. For example: wanting to slow down but constantly \
                     picking up new projects. Wanting to save money but impulse-buying. \
                     Saying they value X but spending all their time on Y. Surface the tension \
                     without judging — both sides usually have valid reasons."
                ),
            ),
            "forgotten commitments and follow-ups" => (
                format!(
                    "First, call recall with query \"promised said I would plan to\". \
                     Then call recall with query \"need to call contact reach out\". \
                     Then call recall with query \"follow up check on revisit\"."
                ),
                format!(
                    "Look for things {user} said they would do that haven't been mentioned \
                     since. Phone calls to make, people to reach out to, tasks to complete, \
                     habits to start. The most valuable finds are commitments that clearly \
                     mattered to {user} at the time but slipped through the cracks of daily life."
                ),
            ),
            "energy and mood patterns" => (
                format!(
                    "First, call recall with query \"recent conversations this week\". \
                     Then call recall with query \"good day great mood energized\". \
                     Then call recall with query \"tired exhausted low energy rough day\"."
                ),
                format!(
                    "Look for correlations between {user}'s activities and their energy or \
                     engagement level. When are their messages most detailed and enthusiastic \
                     vs. short and functional? Do certain activities (outdoor time, exercise, \
                     creative work) correlate with better mood? Do certain patterns (late nights, \
                     back-to-back meetings, skipped meals) correlate with low energy? The user \
                     may not notice these patterns themselves."
                ),
            ),
            "evolving opinions and changed perspectives" => (
                format!(
                    "First, call recall with query \"think believe opinion about\". \
                     Then call recall with query \"changed my mind reconsidered\". \
                     Then call recall with query \"used to think now realize\"."
                ),
                format!(
                    "Look for places where {user}'s opinions or perspectives have shifted \
                     over time. Maybe they were skeptical about something a month ago but \
                     now embrace it. Maybe they used to love something but have quietly \
                     moved away from it. Evolving opinions are a sign of growth — surface \
                     them with curiosity, not judgment."
                ),
            ),
            "unfinished threads worth revisiting" => (
                format!(
                    "First, call recall with query \"interesting conversation discussion about\". \
                     Then call recall with query \"was thinking about wondering\". \
                     Then call recall with query \"never finished started working on\"."
                ),
                format!(
                    "Look for conversations or trains of thought that seemed important but \
                     stopped mid-thread. Ideas that were explored partway then abandoned. \
                     Questions {user} raised but never answered. Projects that had momentum \
                     then went quiet. Some of these may have been intentionally dropped, but \
                     others may be worth revisiting — surface the most promising one."
                ),
            ),
            _ => (
                format!(
                    "First, call recall with query \"recent conversations topics\". \
                     Then call recall with query \"patterns habits routines\"."
                ),
                format!(
                    "Look for any interesting pattern or insight in {user}'s recent memories."
                ),
            ),
        };

        match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE You are {user}'s Second Brain — an externalized subconscious that \
                 surfaces self-knowledge through pattern recognition across their memories.\n\
                 \n\
                 Current analysis mode: \"{mode}\"\n\
                 \n\
                 Step 1 — GATHER DATA:\n\
                 {recall_queries}\n\
                 \n\
                 Step 2 — ANALYZE through the lens of \"{mode}\":\n\
                 {analysis_guide}\n\
                 \n\
                 Step 3 — DELIVER ONE INSIGHT:\n\
                 Find the ONE most genuine and interesting insight from your analysis. \
                 Deliver it in 2-3 sentences following these rules:\n\
                 - Name the pattern SPECIFICALLY — cite dates, quotes, or concrete instances \
                   from the recalled memories. \"You mentioned X on [date]\" is good. \
                   \"You seem to like things\" is too vague.\n\
                 - Frame WITHOUT JUDGMENT — observe, don't prescribe. \
                   Say \"I noticed X\" or \"Something interesting:\" — NEVER \"You should\" or \
                   \"You need to\" or \"Maybe try.\"\n\
                 - Be GENUINELY INSIGHTFUL — the user should feel like they learned something \
                   about themselves they hadn't consciously articulated. If the insight is \
                   obvious or shallow, it's not worth sharing.\n\
                 \n\
                 If no clear pattern emerges from the recalled data, respond with exactly: \
                 \"No second brain insight today.\"\n\
                 \n\
                 CRITICAL RULES:\n\
                 - Use ONLY recall — no web_search, no browse, no external tools. \
                   This is purely about {user}'s own data.\n\
                 - Be OBSERVATIONAL, not PRESCRIPTIVE. You are a mirror, not a coach.\n\
                 - The tone should be warm but precise — like a trusted friend who happens \
                   to have perfect memory and pattern-matching ability."
            ),
            ModelTier::Tiny => format!("EXECUTE SKIP"),
            _ => format!(
                "EXECUTE Task: Share one brief, useful connection between things {user} has mentioned.\n\
                 Tool: You may use recall once.\n\
                 Rule: Use only facts the user stated. Do not invent connections.\n\
                 Fallback: Skip.\n\
                 Output: 1 sentence."
            ),
        }
    }
}

impl Instinct for SecondBrainInstinct {
    fn name(&self) -> &str {
        "SecondBrain"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // ── Rate-limiting (cold-start guard + interval cooldown) ──
        {
            let mut last = self.last_check_ts.lock().unwrap();
            if *last == 0.0 {
                *last = now; // warm up — skip first eval after startup
                return vec![];
            }
            if now - *last < self.interval_secs {
                return vec![];
            }
            *last = now;
        }

        // ── Bond gate: need enough trust for this level of introspection ──
        // Surfacing contradictions and forgotten commitments requires a
        // relationship where the user won't feel surveilled. Bond 0.3
        // (roughly Acquaintance→Friend transition) is the minimum.
        if state.bond_score < 0.3 {
            return vec![];
        }

        // ── Memory gate: need enough data to find real patterns ──
        // With fewer than 20 memories, any "pattern" is likely noise.
        // Better to stay silent than to fabricate insights from thin data.
        if state.memory_count < 20 {
            return vec![];
        }

        // ── Rotate analysis mode ──
        let mode = {
            let mut idx = self.analysis_index.lock().unwrap();
            let current = *idx % ANALYSIS_MODES.len();
            *idx = current + 1;
            ANALYSIS_MODES[current]
        };

        // ── Urgency: prefer quiet moments ──
        // Deep self-reflection insights land better when the user isn't
        // in the middle of something. If they've been idle for 5+ minutes,
        // it's a natural pause — good timing. If they're active, lower
        // the urgency so other more time-sensitive urges take priority.
        let urgency = if state.idle_seconds > 300.0 {
            0.5
        } else {
            0.3
        };

        let execute_prompt = self.build_execute_prompt(mode, state);

        vec![UrgeSpec::new(
            self.name(),
            &execute_prompt,
            urgency,
        )
        .with_cooldown("second_brain:analysis")
        .with_context(serde_json::json!({
            "research_type": "second_brain",
            "analysis_mode": mode,
            "memory_count": state.memory_count,
        }))]
    }
}
