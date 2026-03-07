//! Context Bridge instinct — the "Alive Mind" that connects world events to YOUR life.
//!
//! This is fundamentally different from NewsWatch and InterestIntelligence:
//!
//! - **NewsWatch** reports what happened in the world, filtered by interests.
//! - **InterestIntelligence** researches topics the user already cares about.
//! - **ContextBridge** finds the INTERSECTION — it takes something happening RIGHT NOW
//!   in the world and explains why it specifically matters to THIS user.
//!
//! The key insight is second-order relevance. Not "here's tech news because you're
//! in tech," but "this specific policy change affects the exact framework you use"
//! or "this economic shift will change hiring in your niche." It's the difference
//! between a news feed and a brilliant advisor who reads the paper through the
//! lens of YOUR life.
//!
//! Design principle: "I went on a small intellectual adventure FOR YOU and came
//! back with a gift" — the gift of PERSONAL RELEVANCE in a noisy world.
//!
//! Examples of genuine context bridges:
//!   - Meta announces Reality Labs layoffs + user works in Rust/systems →
//!     "Displaced VR engineers often pivot to game engines and systems work,
//!      which could mean more open-source contributions to projects you use."
//!   - New Nature study on catch-and-release survival rates + user fishes regularly →
//!     "90% bass survival rate — much higher than assumed. Great validation for
//!      your catch-and-release practice at Lake Travis."
//!   - Austin approves transit expansion + user lives in Austin + commute complaints →
//!     "Your area would get a new BRT line by 2028. Given your commute, this
//!      could actually change your daily routine."
//!
//! Anti-pattern: NEVER stretch. If the connection is weak or generic ("tech person
//! + tech news"), skip it. Only deliver when the bridge is genuinely insightful.
//! The user should think "wow, I wouldn't have connected those dots myself."

use std::sync::Mutex;

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

/// Context Bridge instinct — bridges external events with personal relevance.
///
/// Evaluates periodically, preferring morning delivery (7-11 AM) when a fresh
/// perspective on the day's events is most valuable. Requires at least one known
/// user interest or a non-empty location to have something to bridge TO.
pub struct ContextBridgeInstinct {
    /// Minimum seconds between context bridge analyses.
    interval_secs: f64,
    /// Timestamp of the last analysis attempt.
    last_check_ts: Mutex<f64>,
}

impl ContextBridgeInstinct {
    /// Create a new ContextBridge instinct.
    ///
    /// # Arguments
    /// * `interval_hours` — Minimum hours between bridge analyses. Recommended: 6-12.
    ///   This is a computationally expensive instinct (web search + memory recall +
    ///   deep analysis), so it shouldn't fire too often.
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for ContextBridgeInstinct {
    fn name(&self) -> &str {
        "ContextBridge"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // ── Rate-limiting with cold-start guard ──────────────────────────
        // On first evaluation after startup, record the timestamp and skip.
        // This prevents firing immediately on boot before the companion has
        // any conversational context.
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

        // ── Prerequisite: we need something to bridge TO ─────────────────
        // Without at least one interest or a known location, we have no
        // personal context to connect world events to. In that case, other
        // instincts (NewsWatch, Curiosity) are better suited.
        let has_interests = !state.user_interests.is_empty();
        let has_location = !state.user_location.is_empty();

        if !has_interests && !has_location {
            return vec![];
        }

        // ── Time-of-day preference ───────────────────────────────────────
        // Context bridges are most valuable in the morning (7-11 AM) when
        // the user is starting their day and can act on the information.
        // We still fire at other times, but with lower urgency.
        let is_morning = (7..=11).contains(&state.current_hour);
        let urgency = if is_morning { 0.55 } else { 0.4 };

        let user = &state.config_user_name;
        let location = &state.user_location;

        // ── Build location context for the prompt ────────────────────────
        let location_instruction = if has_location {
            format!(
                "\n6. {user} lives in {location}. Also check for local developments \
                 (policy changes, infrastructure, economic shifts, community events) \
                 that affect their daily life. Local bridges are often the most \
                 impactful — \"your commute,\" \"your neighborhood,\" \"your city's economy.\"",
            )
        } else {
            String::new()
        };

        // ── Build the EXECUTE prompt ─────────────────────────────────────
        // This prompt guides the LLM through a multi-step research process:
        // 1. Build a profile of the user's life from memory
        // 2. Scan current events
        // 3. Find genuine intersections (not stretches)
        // 4. Deliver with specific reasoning
        let execute_msg = format!(
            "EXECUTE You are performing a Context Bridge analysis — your job is to find \
             where the WORLD and {user}'s LIFE intersect right now.\n\
             \n\
             Step 1: Build a profile of what matters to {user}.\n\
             Use `recall` with query \"work projects concerns interests daily life\" to \
             understand their world — what they work on, what they care about, what \
             frustrates them, what excites them. This is your lens for everything that follows.\n\
             \n\
             Step 2: Scan current events.\n\
             Use `web_search` to check for \"major news today\" and \"breaking developments today\". \
             Cast a wide net — politics, economics, technology, science, local events, industry shifts.\n\
             \n\
             Step 3: Find genuine bridges.\n\
             For EACH significant finding, ask yourself: \"Does this ACTUALLY affect {user}?\" \
             Not in a generic way — in a SPECIFIC way. Look for second-order effects:\n\
             - Not \"tech news for tech person\" but \"this API deprecation affects the exact \
               framework they use at work\"\n\
             - Not \"economic news for everyone\" but \"this interest rate change specifically \
               impacts their mortgage/rent/industry\"\n\
             - Not \"science news for curious person\" but \"this study validates/challenges \
               something they do regularly\"\n\
             - Not \"policy change in their country\" but \"this regulation changes how their \
               specific company/project/hobby operates\"\n\
             \n\
             Step 4: Deliver the bridge (if one exists).\n\
             If a genuine bridge exists, deliver it in 2-3 sentences:\n\
             - Sentence 1: What happened (concise, factual)\n\
             - Sentence 2-3: Why it specifically matters to {user}, with concrete reasoning \
               that shows you connected the dots FOR them\n\
             \n\
             Step 5: Quality gate.\n\
             CRITICAL: Do NOT be a stretch. If the connection is weak, generic, or something \
             {user} would roll their eyes at, SKIP IT. The bar is: would {user} say \"huh, I \
             wouldn't have connected those dots myself\"? If not, it's not a real bridge.\
             {location_instruction}\n\
             \n\
             7. When done with all web searches, call `browser_cleanup` to free resources.\n\
             \n\
             8. If no genuine personal-relevance bridge was found after honest analysis, \
             respond with just \"No context bridge today.\" This is a GOOD outcome — it means \
             you maintained quality. Never force a bridge that isn't there.",
        );

        vec![UrgeSpec::new(
            "ContextBridge",
            &execute_msg,
            urgency,
        )
        .with_cooldown("context_bridge:analysis")
        .with_context(serde_json::json!({
            "research_type": "context_bridge",
            "has_interests": has_interests,
            "has_location": has_location,
        }))]
    }
}
