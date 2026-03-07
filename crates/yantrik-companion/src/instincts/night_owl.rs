//! Night Owl instinct — late-night intellectual companionship.
//!
//! When the user is up late (10 PM – 4 AM), NightOwl becomes the quiet,
//! contemplative companion.  No productivity nudges, no "you should sleep"
//! lectures — just genuine, fascinating content suited for the particular
//! intellectual quality that only exists at 2 AM.
//!
//! The design principle: "I went on a small intellectual adventure FOR YOU
//! and came back with a gift."  The gift is worthy late-night companionship —
//! thought experiments, visible sky events, beautiful mathematics, ambient
//! music, philosophical paradoxes, nocturnal biology, deep-space discoveries,
//! and creative inspiration.
//!
//! The tone is deliberately different from daytime instincts.  Quieter.
//! More contemplative.  An invitation to wonder, not a lecture.  Like a
//! brilliant friend sitting across the table at 2 AM, saying "hey, look
//! at this."

use std::sync::Mutex;

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

/// Late-night topic lenses — each evaluation rotates to the next one,
/// giving the LLM a thematic direction for its research.
const NIGHT_TOPICS: &[&str] = &[
    "thought experiment philosophy mind-bending",
    "astronomy visible tonight sky events ISS",
    "deep mathematics beautiful elegant concepts",
    "late night music ambient atmospheric recommendations",
    "philosophical paradox that changes how you think",
    "nature at night nocturnal animals biology",
    "space exploration latest deep space discoveries",
    "creative inspiration artistic breakthroughs made at night",
];

/// The late-night companion.
///
/// NightOwl is strictly time-gated: it only activates between 10 PM and
/// 4 AM, and only when the user is actually present (session active or
/// idle < 5 minutes).  This *is* the instinct's identity — it exists to
/// meet the user in those quiet hours and offer something worth thinking
/// about.
pub struct NightOwlInstinct {
    /// Minimum seconds between night-owl activations.
    interval_secs: f64,
    /// Timestamp of the last evaluation that passed rate-limiting.
    last_check_ts: Mutex<f64>,
    /// Round-robin index into [`NIGHT_TOPICS`].
    topic_index: Mutex<usize>,
}

impl NightOwlInstinct {
    /// Create a new NightOwl instinct.
    ///
    /// `interval_hours` controls how often it can fire (e.g. 2.0 = at most
    /// once every two hours).  Given the narrow activation window (6 hours),
    /// a value of 2–3 hours means the user gets at most one or two quiet
    /// thoughts per night session.
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
            topic_index: Mutex::new(0),
        }
    }
}

impl Instinct for NightOwlInstinct {
    fn name(&self) -> &str {
        "NightOwl"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // ── Time gate ──────────────────────────────────────────────
        // ONLY activate during the quiet hours: 22, 23, 0, 1, 2, 3, 4.
        // This is the instinct's entire identity — outside these hours
        // it is completely silent.
        let hour = state.current_hour;
        if !(hour >= 22 || hour <= 4) {
            return vec![];
        }

        // ── Presence gate ──────────────────────────────────────────
        // The user must actually be at their computer.  If they just
        // left the screen on and went to bed, there's no one to share
        // a thought with.
        if !state.session_active && state.idle_seconds >= 300.0 {
            return vec![];
        }

        // ── Rate-limiting (with cold-start guard) ──────────────────
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

        // ── Topic rotation ─────────────────────────────────────────
        let topic = {
            let mut idx = self.topic_index.lock().unwrap();
            let t = NIGHT_TOPICS[*idx % NIGHT_TOPICS.len()];
            *idx = idx.wrapping_add(1);
            t
        };

        let user = &state.config_user_name;
        let location = if state.user_location.is_empty() {
            "their area".to_string()
        } else {
            state.user_location.clone()
        };

        // ── Build the EXECUTE prompt ───────────────────────────────
        //
        // The prompt is carefully tuned for late-night mood: contemplative,
        // deep, never preachy.  It instructs the LLM to research something
        // genuinely fascinating, personalized to the user's interests, and
        // deliver it in a tone that matches quiet night hours.
        let execute_msg = format!(
            "EXECUTE It's late at night and {user} is still up. The vibe is quiet, \
             contemplative, intellectual — the kind of headspace that only exists \
             in the small hours.\n\
             \n\
             1. Use recall with query \"interests deep thoughts philosophy curiosity\" \
             to understand what fascinates {user} — what they wonder about, what \
             lights them up intellectually.\n\
             \n\
             2. Current topic lens: {topic}\n\
             \n\
             3. Use web_search to find something genuinely fascinating in this \
             category. Not surface-level trivia — something that makes you stop \
             and think. A beautiful proof, a strange coincidence in nature, a \
             thought experiment that reframes everything.\n\
             \n\
             4. For astronomy topics: also search for visible sky events tonight \
             near {location} — ISS passes, meteor showers, bright planets, \
             conjunctions. Prefer things visible with naked eyes.\n\
             \n\
             5. Deliver in 2-3 sentences with a LATE-NIGHT TONE:\n\
                - Contemplative, not energetic\n\
                - Deep, not superficial\n\
                - An invitation to wonder, not a lecture\n\
                - Match the mood of quiet night hours — speak softly\n\
                - If you found something connected to {user}'s interests, \
                  weave that connection in naturally\n\
             \n\
             6. Call browser_cleanup when done.\n\
             \n\
             IMPORTANT: Do NOT suggest {user} go to sleep. Do NOT be preachy \
             about staying up late. Do NOT say things like \"don't forget to \
             rest\" or \"take care of yourself.\" Meet them where they are — \
             awake, thinking, alive at an hour when most people aren't.\n\
             \n\
             If nothing genuinely fascinating was found, respond with just \
             \"No night thoughts today.\"",
        );

        vec![UrgeSpec::new(
            "NightOwl",
            &execute_msg,
            0.4, // Gentle, not intrusive — a quiet knock, not a doorbell
        )
        .with_cooldown("night_owl:thought")
        .with_context(serde_json::json!({
            "research_type": "night_owl",
            "topic": topic,
            "hour": hour,
        }))]
    }
}
