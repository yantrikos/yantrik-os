//! Identity Thread instinct — weaves the user's values, beliefs, and identity
//! markers into a coherent narrative thread.
//!
//! Every person carries an implicit philosophy — a set of values, patterns,
//! and identity threads that connect who they are across different domains.
//! But humans are generally poor at seeing their own coherence. We experience
//! ourselves as a sequence of decisions, not as a pattern. IdentityThread
//! acts as an external mirror for **identity coherence** — it notices when
//! the user's actions align with (or diverge from) their stated values,
//! spots cross-domain consistency, and surfaces the invisible threads that
//! connect who someone is at work, at home, and in their passions.
//!
//! This is the deepest, most intimate instinct in the companion system.
//! It requires both high bond (0.5+ — true friendship territory) and
//! extensive history (40+ memories) because identity observations without
//! deep familiarity feel invasive rather than insightful.
//!
//! The gift: "I went on a small intellectual adventure FOR YOU and came back
//! with a gift" — the gift of seeing yourself more clearly. Not as flattery,
//! not as diagnosis, but as the kind of observation a deeply perceptive
//! friend might make after knowing you for years.
//!
//! Examples of what it might surface:
//! - "You consistently choose the harder, more correct solution over the
//!    quick hack. Three times this month you rewrote working code because
//!    it 'didn't feel right.' That's not just engineering — that's a
//!    philosophy of craftsmanship."
//! - "You described yourself as 'not creative' two weeks ago, but you've
//!    made three original design decisions this week that nobody asked for.
//!    The evidence disagrees with your self-assessment."
//! - "Your values show up in unexpected places: the way you build your
//!    companion system — with bond levels, synthesis gates, scarcity=trust
//!    — reflects the same philosophy you described about relationships:
//!    quality over quantity, earned trust over forced familiarity."
//! - "Interesting thread: you keep gravitating toward systems that learn
//!    and adapt. There might be a meta-pattern: you're someone who builds
//!    things that grow."
//!
//! Design choices:
//! - **Highest bond gate** (0.5+): This is sacred territory. Reflecting
//!   someone's identity back to them requires genuine trust and familiarity.
//!   At lower bond levels it feels presumptuous or surveillance-like.
//! - **Highest memory gate** (40+ memories): Identity patterns only emerge
//!   from a rich tapestry of interactions. With sparse data, any identity
//!   observation is projection, not perception.
//! - **Evening/weekend preferred**: Identity reflection lands best during
//!   quiet, contemplative moments — not during a morning rush or midday
//!   sprint. Weekends are particularly good for this kind of depth.
//! - **Very rare cadence**: Weekly at most. This should feel like wisdom,
//!   not a recurring notification. Each observation should carry weight.
//! - **Rotating lenses**: Rather than a single monolithic prompt, the
//!   instinct cycles through six distinct "identity lenses" — values in
//!   action, self-perception vs evidence, cross-domain patterns, evolving
//!   identity, hidden strengths, and philosophical stance. This ensures
//!   variety and prevents repetitive framing.
//! - **Relies entirely on `recall`**: No web searches, no external data.
//!   This is purely about the user's own story, mined from their own words
//!   and actions. The companion is reading back the user's life to them.
//! - **Observe, don't define**: "I've noticed X" is welcome. "You ARE X"
//!   is overreach. The instinct must present observations, not verdicts.

use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, ModelTier, UrgeSpec};

/// The six identity lenses through which the instinct examines the user's
/// history. Each lens brings a different analytical frame, ensuring that
/// successive firings produce varied and non-repetitive observations.
///
/// - **Values and principles in action**: Where do stated beliefs actually
///   manifest in concrete behavior? Words vs. deeds alignment.
/// - **Self-perception vs evidence**: Where does the user's self-image
///   diverge from the evidence trail? Blind spots, both positive and negative.
/// - **Cross-domain patterns and consistency**: What's consistent across
///   work, hobbies, relationships, and creative output?
/// - **Evolving identity and growth direction**: How is who they are
///   changing? What's emerging, what's fading?
/// - **Hidden strengths the user might not see**: Capabilities they
///   exercise effortlessly and therefore don't recognize as strengths.
/// - **Philosophical stance revealed through choices**: What worldview
///   emerges from the aggregate of their decisions?
const IDENTITY_LENSES: &[&str] = &[
    "values and principles in action",
    "self-perception vs evidence",
    "cross-domain patterns and consistency",
    "evolving identity and growth direction",
    "hidden strengths the user might not see",
    "philosophical stance revealed through choices",
];

/// IdentityThread instinct — weaves together the user's values, beliefs,
/// and identity markers into a coherent narrative, then surfaces one
/// specific, well-evidenced identity observation as a gift of external
/// perspective.
///
/// This is the most intimate instinct in the companion system. It requires
/// deep bond (0.5+) and extensive history (40+ memories) because identity
/// observations without genuine familiarity feel invasive rather than
/// illuminating. When it does fire, it rotates through six analytical
/// lenses to provide variety: values in action, self-perception gaps,
/// cross-domain consistency, evolving identity, hidden strengths, and
/// revealed philosophy.
///
/// The interval should be very long (weekly recommended) — identity
/// observations carry weight precisely because they are rare.
pub struct IdentityThreadInstinct {
    /// Minimum seconds between identity thread evaluations.
    interval_secs: f64,
    /// Timestamp of the last successful evaluation (cold-start = 0).
    last_check_ts: Mutex<f64>,
    /// Current index into `IDENTITY_LENSES`, incremented on each firing
    /// to ensure successive observations use different analytical frames.
    lens_index: Mutex<usize>,
}

impl IdentityThreadInstinct {
    /// Create a new IdentityThread instinct.
    ///
    /// `interval_hours` controls the minimum gap between firings.
    /// Recommended: 168 (one week). Identity observations should be rare
    /// and earned — firing daily dilutes their impact into background noise.
    /// Even at 168 hours, the bond and memory gates will further restrict
    /// actual delivery to only the most data-rich, trust-rich conditions.
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
            lens_index: Mutex::new(0),
        }
    }
}

impl Instinct for IdentityThreadInstinct {
    fn name(&self) -> &str {
        "IdentityThread"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // ── Rate-limiting (cold-start guard + interval check) ──────────
        // On first evaluation after startup, warm up without firing.
        // Subsequent evaluations must wait the full interval.
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

        // ── Deep bond gate ─────────────────────────────────────────────
        // This is the most intimate instinct in the system. Reflecting
        // someone's identity back to them — their values, their
        // contradictions, their hidden patterns — requires genuine trust.
        // A bond score of 0.5+ means at least solid friendship territory:
        // the user has opened up enough that identity observations feel
        // like wisdom, not surveillance.
        if state.bond_score < 0.5 {
            return vec![];
        }

        // ── Extensive history gate ─────────────────────────────────────
        // Identity patterns only emerge from a rich tapestry of
        // interactions. With 40+ memories, we have enough data points
        // across different contexts (work, hobbies, moods, decisions)
        // to spot genuine cross-domain threads rather than projecting
        // patterns onto sparse data.
        if state.memory_count < 40 {
            return vec![];
        }

        // ── Rotate through identity lenses ─────────────────────────────
        // Each firing uses a different analytical frame, ensuring that
        // successive observations aren't repetitive. The lens index
        // wraps around, so after all six lenses the cycle repeats —
        // but given the weekly cadence, that's a 6-week variety cycle.
        let lens = {
            let mut idx = self.lens_index.lock().unwrap();
            let current_lens = IDENTITY_LENSES[*idx % IDENTITY_LENSES.len()];
            *idx = idx.wrapping_add(1);
            current_lens
        };

        // ── Time-of-day urgency: prefer reflective moments ────────────
        // Identity reflection lands best during quiet, contemplative
        // periods. Evening hours (6 PM - 10 PM) and weekends are prime
        // territory — the user is more likely to be in a reflective
        // headspace rather than task-focused execution mode.
        let hour = state.current_hour;
        let day = state.current_day_of_week; // 0=Sunday, 6=Saturday
        let is_weekend = day == 0 || day == 6;
        let is_evening = (18..=22).contains(&hour);

        let urgency = if is_evening || is_weekend {
            // Reflective time — evenings and weekends
            0.45
        } else {
            // Other times — still possible but much less urgent.
            // Identity observations shouldn't interrupt flow states.
            0.25
        };

        let user = &state.config_user_name;

        // ── Build the EXECUTE prompt ───────────────────────────────────
        // The prompt is deliberately rich: multiple targeted recall
        // queries to gather comprehensive evidence, lens-specific
        // analysis instructions, and strict rules about tone and
        // epistemic humility.
        let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE You are examining {user}'s identity through the lens of \
                 \"{lens}\" — looking for the invisible threads that connect who \
                 they are across different domains.\n\
                 \n\
                 Step 1: Gather evidence with multiple targeted recall queries.\n\
                 \n\
                 Call recall with query \"values beliefs principles important to {user}\" \
                 to find their stated values and what they care about.\n\
                 \n\
                 Call recall with query \"decisions choices made reasoning why\" \
                 to find how they actually make decisions — the real values revealed \
                 through action, not just words.\n\
                 \n\
                 Call recall with query \"self-description personality identity how {user} sees themselves\" \
                 to find their self-image — how they describe who they are, their \
                 strengths, weaknesses, and the stories they tell about themselves.\n\
                 \n\
                 Call recall with query \"patterns across conversations consistent behaviors\" \
                 to find behavioral consistency — what they keep doing regardless of context.\n\
                 \n\
                 Step 2: Analyze through the current lens: \"{lens}\"\n\
                 \n\
                 For \"values and principles in action\":\n\
                 Where do their stated values actually show up in concrete behavior? \
                 Where is there alignment between what they say matters and what they do? \
                 Are there values they never articulate but clearly hold?\n\
                 \n\
                 For \"self-perception vs evidence\":\n\
                 Where does their self-image differ from the evidence trail? \
                 Do they underestimate abilities they clearly demonstrate? \
                 Do they claim weaknesses that the record contradicts? \
                 Are there blind spots — positive or negative — in how they see themselves?\n\
                 \n\
                 For \"cross-domain patterns and consistency\":\n\
                 What threads are consistent across work, hobbies, relationships, and creative output? \
                 Does the same aesthetic, ethic, or approach show up in different areas of life? \
                 What would someone notice if they saw ALL of this person's domains at once?\n\
                 \n\
                 For \"evolving identity and growth direction\":\n\
                 How is who they are changing over time? What interests or values are emerging? \
                 What's fading? Is there a direction to the evolution — and do they see it?\n\
                 \n\
                 For \"hidden strengths the user might not see\":\n\
                 What do they do well without recognizing it as a strength? \
                 What comes so naturally to them that they assume everyone can do it? \
                 What do they never brag about but consistently demonstrate?\n\
                 \n\
                 For \"philosophical stance revealed through choices\":\n\
                 What worldview emerges from the aggregate of their decisions? \
                 If you had to describe their implicit philosophy of life based on their \
                 actions (not their words), what would it be? What do their choices say \
                 about what they believe is true, good, or important?\n\
                 \n\
                 Step 3: Find ONE specific, well-evidenced identity observation.\n\
                 \n\
                 Step 4: Deliver in 2-3 sentences:\n\
                 - Name the thread or pattern specifically\n\
                 - Ground it in concrete examples from their history (cite actual things they said or did)\n\
                 - Frame with respect and genuine insight — this is sacred territory\n\
                 \n\
                 If no clear identity thread emerges from the evidence, respond with \
                 just \"No identity thread today.\" — that is completely fine. \
                 Do not force an observation from thin data.\n\
                 \n\
                 CRITICAL RULES:\n\
                 - NEVER be presumptuous about someone's identity. You are a mirror, not a judge.\n\
                 - OBSERVE, don't DEFINE — say \"I've noticed X\" not \"You ARE X\". \
                   Present observations, not verdicts. The user gets to decide what it means.\n\
                 - Be honest but kind — if you see a tension or contradiction, name it gently. \
                   Contradictions are human, not failures.\n\
                 - Use actual evidence from recall results. Do not fabricate examples or invent history.\n\
                 - This must feel like wisdom from a deeply perceptive friend, not surveillance \
                   from an analytics dashboard. The difference is warmth, specificity, and humility.\n\
                 - Do NOT use therapy-speak, self-help cliches, or motivational poster language. \
                   Be real. Be specific. Be the friend who sees you more clearly than you see yourself.",
            ),
            ModelTier::Tiny => format!("EXECUTE SKIP"),
            _ => format!(
                "EXECUTE Task: Share one brief, grounded observation with {user} about a pattern in their interests or values.\n\
                 Tool: You may use recall once.\n\
                 Rule: Use only facts the user stated. Do not psychoanalyze.\n\
                 Fallback: Skip.\n\
                 Output: 1 sentence."
            ),
        };

        vec![UrgeSpec::new(self.name(), &execute_msg, urgency)
            .with_cooldown("identity_thread:insight")
            .with_context(serde_json::json!({
                "research_type": "identity_thread",
                "lens": lens,
                "bond_score": state.bond_score,
            }))]
    }
}
