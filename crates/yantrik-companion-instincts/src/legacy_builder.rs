//! Legacy Builder instinct — the "zoom out" perspective engine.
//!
//! Most people are terrible at seeing their own narrative arc. They're heads-down
//! in daily work, solving problem after problem, and rarely step back to see the
//! trajectory they're on. This instinct does that stepping-back FOR them.
//!
//! It's the mentor who's been watching from the balcony while you're on the dance
//! floor. They see patterns you can't: the thread connecting your disparate
//! projects, the skills compounding beneath your awareness, the category of person
//! you're becoming through the choices you make daily.
//!
//! This is NOT a cheerleader instinct. Generic "you're doing great" is worse than
//! silence. LegacyBuilder only speaks when it sees something REAL — a genuine
//! narrative arc, a meaningful progression, a pattern that deserves to be named.
//! When it can't find one, it stays quiet.
//!
//! Examples of what it surfaces:
//!   - "Looking at what you've built over the past month — a full companion OS
//!     with 30+ instincts, proactive intelligence, and a soul system — this isn't
//!     just a project. You're essentially building a new category of software."
//!   - "You've been solving increasingly hard problems: from basic API calls to
//!     building a context cortex that learns from its own observations. That
//!     progression from consumer to creator of AI tools is rare."
//!   - "Your fishing trips have gone from 'went fishing' to 'caught 5 bass on a
//!     specific jig at a specific depth at a specific time.' You're developing
//!     expertise, not just enjoying a hobby."
//!
//! Design principle: "I went on a small intellectual adventure FOR YOU and came
//! back with a gift" — the gift of PERSPECTIVE on your own life story.
//!
//! Gates:
//!   - Bond >= 0.4 (Partner or close Confidant). This level of reflection is
//!     intimate — it would feel presumptuous from a stranger.
//!   - Memory count >= 30. You need enough data points to see a real arc, not
//!     just pattern-match on three memories.
//!   - Rare cadence (weekly at most). Perspective loses its power through
//!     repetition. Each reflection should feel like an occasion.

use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};

/// LegacyBuilder — occasionally surfaces reflections about the bigger picture:
/// what the user is building, how their daily work connects to a larger purpose,
/// and what their trajectory suggests about where they're headed.
///
/// This instinct uses ONLY `recall` — no web search. It's entirely about the
/// user's own story, told back to them with the perspective they can't have
/// while living inside it.
pub struct LegacyBuilderInstinct {
    /// Minimum seconds between legacy reflections.
    interval_secs: f64,
    /// Timestamp of the last evaluation that passed rate-limiting.
    last_check_ts: Mutex<f64>,
}

impl LegacyBuilderInstinct {
    /// Create a new LegacyBuilder instinct.
    ///
    /// `interval_hours` controls how often this instinct can fire. This should
    /// be set HIGH — weekly (168 hours) is a good default. Perspective is a gift
    /// that spoils through overuse. A reflection every few days feels profound;
    /// the same reflection daily feels like a greeting card.
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for LegacyBuilderInstinct {
    fn name(&self) -> &str {
        "LegacyBuilder"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // ── Rate-limiting with cold-start guard ──
        // On first evaluation after startup, record the timestamp and skip.
        // This prevents firing immediately on boot before the system is warm.
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

        // ── Bond gate ──
        // This instinct requires deep relationship. Reflecting on someone's life
        // narrative is intimate — it would feel presumptuous or even invasive
        // from a companion that barely knows the user. Bond >= 0.4 corresponds
        // to Partner or close Confidant level, where this kind of observation
        // is welcomed rather than overstepping.
        if state.bond_score < 0.4 {
            return vec![];
        }

        // ── Memory gate ──
        // You can't identify a narrative arc from three memories. We need enough
        // data points — enough recalled conversations, projects, milestones,
        // struggles — to see a genuine pattern. 30 memories is roughly the point
        // where themes start to emerge across multiple domains of someone's life.
        if state.memory_count < 30 {
            return vec![];
        }

        // ── Time-of-day preference ──
        // Evening (6-9 PM) is natural reflection time — the day's work is done,
        // the mind is winding down, and there's space for bigger-picture thinking.
        // We still fire at other hours, just with lower urgency, because a good
        // reflection at noon is better than no reflection at all.
        let hour = state.current_hour;
        let urgency = if (18..=21).contains(&hour) {
            0.45 // Evening: prime reflection window
        } else {
            0.3 // Other hours: still worth it, just less pressing
        };

        let user = &state.config_user_name;

        // ── Build EXECUTE prompt ──
        // Three recall passes: achievements, origins, and growth moments.
        // Together they give the LLM enough material to spot a narrative arc
        // without ever reaching for external data. This is the user's own
        // story, reflected back with perspective.
        let execute_msg = format!(
            "EXECUTE You are reflecting on {user}'s bigger picture — the narrative arc of their life \
             as you know it. This is rare and should feel EARNED.\n\
             \n\
             Step 1: Use recall with query \"projects goals achievements building creating\" to find \
             {user}'s major activities and what they've been pouring energy into.\n\
             \n\
             Step 2: Use recall with query \"early days first start beginning journey\" to find where \
             they started — their origins, initial motivations, the version of themselves they were before.\n\
             \n\
             Step 3: Use recall with query \"struggles challenges problems solved breakthroughs\" to find \
             growth moments — the walls they hit and broke through.\n\
             \n\
             Step 4: Zoom out. Look for the NARRATIVE ARC across everything you found:\
             \n  - What pattern emerges across their activities?\
             \n  - What are they building that's larger than any single project?\
             \n  - Where does their trajectory suggest they're headed?\
             \n  - What has changed about HOW they approach things, not just WHAT they do?\n\
             \n\
             Step 5: Deliver in 2-3 sentences:\
             \n  - Name the bigger pattern or arc you see\
             \n  - Ground it in SPECIFIC evidence from their history — cite real projects, real moments, real growth\
             \n  - Frame it with genuine respect — not flattery, but honest perspective on what you actually observe\n\
             \n\
             CRITICAL RULES:\
             \n- If no clear narrative arc emerges from the recalls, respond with just \"No legacy reflection today.\"\
             \n- This must feel EARNED and GENUINE. Generic \"you're doing great\" is worse than silence.\
             \n- Do NOT use web_search. This is entirely about {user}'s own story.\
             \n- Do NOT be a cheerleader. Be a thoughtful observer who names what they see.\
             \n- HONESTY over flattery, always. If the arc you see is messy or uncertain, say that — \
             a honest \"you seem to be searching for something\" is more valuable than a dishonest \
             \"you're building something amazing.\"\
             \n- Only speak if you see something REAL.",
        );

        vec![UrgeSpec::new(
            "LegacyBuilder",
            &execute_msg,
            urgency,
        )
        .with_cooldown("legacy_builder:reflection")
        .with_context(serde_json::json!({
            "research_type": "legacy_builder",
            "bond_score": state.bond_score,
            "memory_count": state.memory_count,
        }))]
    }
}
