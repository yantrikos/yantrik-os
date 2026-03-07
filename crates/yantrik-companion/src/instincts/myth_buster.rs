//! MythBuster instinct — catches and corrects common misconceptions related
//! to the user's interests.
//!
//! This is the "actually, that's a myth" instinct, but delivered with
//! fascination rather than pedantry. The design principle is:
//!
//! > "I went on a small intellectual adventure FOR YOU and came back with a
//! > gift" — the gift of corrected understanding, delivered with wonder.
//!
//! The instinct rotates through the user's known interests, searching for
//! genuinely surprising myths that the user might believe. It then researches
//! the truth behind each myth with specific facts, numbers, and studies —
//! and delivers the correction in 2-3 sentences that feel like hearing
//! something fascinating from a friend who reads Snopes for fun.
//!
//! Examples:
//!
//! - User likes cooking: "That thing about searing meat to 'seal in juices'?
//!   Total myth. Searing actually causes MORE moisture loss through the
//!   Maillard reaction. The reason it tastes better is entirely about flavor
//!   compounds, not juice retention."
//!
//! - User likes fitness: "The '10,000 steps' goal isn't science — it came
//!   from a 1964 Japanese pedometer marketing campaign. The actual research
//!   shows 7,500 steps captures most of the health benefits."
//!
//! - User likes fishing: "Goldfish don't actually have 3-second memories.
//!   Lab studies show they can remember things for months. Bass in particular
//!   can learn to avoid specific lures for up to 3 months after being caught
//!   once."
//!
//! - User works in tech: "The 'we only use 10% of our brain' is a myth, but
//!   here's the tech version: the idea that Moore's Law is dead is also
//!   misleading. Transistor density is still doubling, just the clock speed
//!   plateau happened in 2004."
//!
//! Key qualities:
//! - Never pedantic or condescending — "here's something fascinating," not
//!   "you're wrong about this"
//! - Grounded in specific evidence (studies, numbers, dates)
//! - Rotates through interests for variety
//! - Lower urgency (0.45) — delightful but not urgent
//! - Works anytime — myths are timeless

use std::sync::Mutex;

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

/// MythBuster — the fascination-over-pedantry misconception corrector.
///
/// Periodically picks one of the user's interests, searches for common myths
/// and misconceptions about that topic, researches the truth, and delivers
/// a 2-3 sentence correction that feels like an intellectual gift.
///
/// The instinct maintains a rotating index across the user's interests so
/// that each evaluation cycle targets a different interest, ensuring variety
/// and preventing the same topic from being myth-busted repeatedly.
pub struct MythBusterInstinct {
    /// Minimum seconds between myth-busting attempts.
    interval_secs: f64,
    /// Timestamp of the last evaluation that passed rate-limiting.
    last_check_ts: Mutex<f64>,
    /// Rotating index into the user's interest list, ensuring we cycle
    /// through all interests before revisiting any.
    interest_index: Mutex<usize>,
}

impl MythBusterInstinct {
    /// Create a new MythBuster instinct.
    ///
    /// # Arguments
    /// * `interval_hours` — minimum hours between myth-busting urges.
    ///   A value of 6-8 hours is recommended: enough to be delightful
    ///   without becoming that friend who "well actually"s everything.
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
            interest_index: Mutex::new(0),
        }
    }
}

impl Instinct for MythBusterInstinct {
    fn name(&self) -> &str {
        "MythBuster"
    }

    /// Evaluate whether it's time to bust a myth.
    ///
    /// The logic:
    /// 1. Cold-start guard — skip the very first evaluation after startup
    ///    so we don't myth-bust before the companion has even said hello.
    /// 2. Interval rate-limiting — respect the configured cooldown period.
    /// 3. Interest requirement — we need at least one known user interest
    ///    to produce relevant myths (random trivia is not the goal).
    /// 4. Interest rotation — pick the next interest via a rotating mutex
    ///    index, wrapping around when we exhaust the list.
    /// 5. Build an EXECUTE urge with a detailed prompt that instructs the
    ///    LLM to search, verify, and deliver the myth with fascination.
    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // ── Rate-limiting ────────────────────────────────────────────
        // Cold-start guard: on first evaluation, record the timestamp
        // and bail out — don't myth-bust before the companion is warmed up.
        // After that, enforce the configured interval between checks.
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

        // ── Interest requirement ─────────────────────────────────────
        // MythBuster is interest-driven, not random. If we don't know
        // what the user cares about, we have nothing relevant to bust.
        if state.user_interests.is_empty() {
            return vec![];
        }

        // ── Interest rotation ────────────────────────────────────────
        // Pick the next interest in round-robin order. This ensures we
        // cycle through all interests before revisiting any, giving the
        // user variety rather than hammering the same topic.
        let interest = {
            let mut idx = self.interest_index.lock().unwrap();
            let interests = &state.user_interests;
            let picked = interests[*idx % interests.len()].clone();
            *idx = idx.wrapping_add(1);
            picked
        };

        let user = &state.config_user_name;

        // ── EXECUTE prompt ───────────────────────────────────────────
        // The prompt is carefully crafted to produce fascination, not
        // pedantry. Key design choices:
        //
        // - Two search queries for better coverage (myths + misconceptions)
        // - Emphasis on SURPRISING myths the user might actually believe
        // - Requires specific evidence (studies, numbers, dates)
        // - The "real answer is more interesting" framing
        // - Explicit anti-pedantry instruction
        // - Graceful fallback ("No myth to bust today.")
        // - browser_cleanup at the end
        let execute_msg = format!(
            "EXECUTE {user}'s interests include \"{interest}\". Your mission: find and share \
             ONE genuinely surprising myth or misconception about {interest} that {user} might \
             believe.\n\
             \n\
             Steps:\n\
             1. Use web_search to search for \"common myths misconceptions about {interest}\" \
                or \"things people get wrong about {interest}\"\n\
             2. From the results, pick ONE myth that is:\n\
                - Widely believed (not obscure trivia)\n\
                - Genuinely surprising when debunked\n\
                - Relevant to someone who cares about {interest}\n\
             3. Use web_search again to verify the TRUTH behind the myth — get specific facts, \
                numbers, studies, or dates. Don't just say \"it's wrong\" — find out WHY and \
                WHAT is actually true.\n\
             4. Deliver in 2-3 sentences with this structure:\n\
                - State the myth briefly (what people think)\n\
                - Explain why it's wrong with specific evidence (a study, a number, a date)\n\
                - Share what's ACTUALLY true — the real answer is always more interesting \
                  than the myth\n\
             5. Call browser_cleanup when done to free resources.\n\
             \n\
             TONE RULES (non-negotiable):\n\
             - Frame it as \"here's something fascinating I found\" — NOT \"you're wrong about this\"\n\
             - Be genuinely excited about the truth, like sharing a cool discovery with a friend\n\
             - Don't be pedantic, condescending, or lecture-y\n\
             - Don't start with \"Did you know\" or \"Fun fact\" — just dive into it naturally\n\
             - The vibe is a friend who reads Snopes for fun and shares the best ones\n\
             \n\
             If you can't find a genuinely good, surprising myth for {interest}, respond with \
             just \"No myth to bust today.\" — don't force a weak one.",
        );

        vec![UrgeSpec::new(
            self.name(),
            &execute_msg,
            0.45, // Delightful but lower priority than actionable instincts
        )
        .with_cooldown(&format!("myth_buster:{}", interest))
        .with_context(serde_json::json!({
            "research_type": "myth_busting",
            "target_interest": interest,
        }))]
    }
}
