//! GoldenFind instinct — discovers hidden gems the user would love but would
//! never find on their own.
//!
//! This is the "I went on a small intellectual adventure FOR YOU and came back
//! with a gift" instinct. It researches the user's known interests and digs
//! for genuinely obscure, high-quality resources: tiny YouTube channels,
//! niche blogs, unknown tools, local spots, cult-favorite products, and
//! forgotten techniques.
//!
//! The gold is in the obscurity. Mainstream, first-page-of-Google results are
//! explicitly rejected. Every find must pass two filters:
//!   1. **Genuinely obscure** — low view counts, few reviews, niche communities.
//!   2. **Genuinely good**   — not obscure because it's bad.
//!
//! Examples:
//!   - User likes cooking Italian → "I found this tiny YouTube channel
//!     (Chef Ferrara, 2K subs) where an 80-year-old Neapolitan grandmother
//!     teaches pasta shapes I've never seen. Her corzetti technique is incredible."
//!   - User works in Rust → "Found a Rust crate called `compact_str` that stores
//!     strings ≤24 bytes inline without heap allocation. Could shave 40% off
//!     your string-heavy code's allocations."
//!   - User likes hiking near Austin → "There's a trail called Turkey Creek that
//!     almost nobody knows about — 2.5 miles through a limestone canyon with a
//!     swimming hole at the end. Only 12 reviews on AllTrails."
//!   - User likes fishing → "A local bait shop owner posted a blog about a hidden
//!     catfish honey hole on Lake Travis — coordinates included. Posted 3 days
//!     ago, only 8 views."
//!
//! Interest rotation ensures variety across evaluations. Location-aware for
//! local discoveries when `user_location` is set.

use std::sync::Mutex;

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

/// GoldenFind — discovers hidden gems tailored to the user's interests.
///
/// On each evaluation cycle it picks the next interest from the user's list
/// (round-robin via `interest_index`), builds a targeted EXECUTE prompt, and
/// hands it to the agent loop for web research.
pub struct GoldenFindInstinct {
    /// Seconds between golden-find research attempts.
    interval_secs: f64,
    /// Timestamp of the last evaluation that passed the rate limiter.
    last_check_ts: Mutex<f64>,
    /// Round-robin index into `state.user_interests`.
    interest_index: Mutex<usize>,
}

impl GoldenFindInstinct {
    /// Create a new GoldenFind instinct.
    ///
    /// # Arguments
    /// * `interval_hours` — minimum hours between research attempts.
    ///   Recommended: 4–8 hours (these are deep-research operations).
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
            interest_index: Mutex::new(0),
        }
    }
}

impl Instinct for GoldenFindInstinct {
    fn name(&self) -> &str {
        "GoldenFind"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // ── Rate-limit with cold-start guard ──────────────────────────
        {
            let mut last = self.last_check_ts.lock().unwrap();
            if *last == 0.0 {
                // First tick after startup — warm up, don't fire immediately.
                *last = now;
                return vec![];
            }
            if now - *last < self.interval_secs {
                return vec![];
            }
            *last = now;
        }

        // ── Need at least one interest to research ────────────────────
        if state.user_interests.is_empty() {
            return vec![];
        }

        // ── Pick next interest (round-robin) ──────────────────────────
        let interest = {
            let mut idx = self.interest_index.lock().unwrap();
            let picked = state.user_interests[*idx % state.user_interests.len()].clone();
            *idx = idx.wrapping_add(1);
            picked
        };

        let user = &state.config_user_name;
        let has_location = !state.user_location.is_empty();

        // ── Build location context for local hidden gems ──────────────
        let location_clause = if has_location {
            format!(
                "\n{user} is based in {}. Include location-specific searches like \
                 \"{interest} hidden gem {}\" or \"{interest} best kept secret {}\". \
                 Local discoveries (unknown shops, trails, spots, events) are especially valuable.",
                state.user_location, state.user_location, state.user_location,
            )
        } else {
            String::new()
        };

        // ── EXECUTE prompt ────────────────────────────────────────────
        //
        // The prompt guides the LLM through a multi-step research flow:
        //   1. Recall what the user already knows/uses for this interest
        //   2. Run multiple targeted web searches for OBSCURE content
        //   3. Filter: must be genuinely obscure AND genuinely good
        //   4. Deliver ONE find with context on why it's special
        let execute_msg = format!(
            "EXECUTE You are going on a small research adventure to find a HIDDEN GEM \
             for {user} related to their interest: \"{interest}\".\
             \n\
             \nStep 1: Use recall with query \"{interest} preferences tools resources favorites\" \
             to understand what {user} already knows, uses, or has mentioned about {interest}. \
             This is critical — do NOT recommend something they already know.\
             \n\
             \nStep 2: Use web_search to run at LEAST two of these searches (pick the most relevant):\
             \n  - \"{interest} hidden gem underrated\"\
             \n  - \"{interest} best kept secret reddit\"\
             \n  - \"{interest} lesser known tips\"\
             \n  - \"{interest} underappreciated tool\"\
             \n  - \"{interest} small youtube channel\"\
             \n  - \"{interest} niche blog worth reading\"\
             \n  - \"{interest} cult favorite unknown\"\
             \n  - \"{interest} obscure but amazing\"\
             {location_clause}\
             \n\
             \nStep 3: FILTER your results ruthlessly. Every find MUST pass BOTH tests:\
             \n  A) GENUINELY OBSCURE — not first-page-of-Google obvious. Look for signals:\
             \n     - YouTube channels with <50K subscribers\
             \n     - Blog posts with few comments/shares\
             \n     - Tools/apps without major press coverage\
             \n     - Places with <100 reviews\
             \n     - Reddit threads with <200 upvotes in niche subreddits\
             \n     - Techniques or resources that require digging to find\
             \n  B) GENUINELY GOOD — not obscure because it's bad. The find should be \
             \n     something that makes you think \"how is this not more well-known?\"\
             \n\
             \nStep 4: Deliver ONE golden find in 2-3 sentences. Explain:\
             \n  - WHAT it is (be specific — name, creator, link context)\
             \n  - WHY it's special (what makes it stand out)\
             \n  - WHY {user} specifically would love it (connect to their known preferences)\
             \n\
             \nTone: Excited but not breathless. Like a friend who found something cool and \
             can't wait to share it. Example: \"I found this tiny YouTube channel (Chef Ferrara, \
             2K subs) where an 80-year-old Neapolitan grandmother teaches pasta shapes I've never \
             seen. Her corzetti technique is incredible.\"\
             \n\
             \nIMPORTANT: Do NOT recommend mainstream, well-known resources. No top-10 lists, \
             no major publications, no tools everyone already knows. The gold is in the obscurity. \
             If you search and everything you find is mainstream, dig deeper with more specific \
             search queries before giving up.\
             \n\
             \nIf after thorough searching you genuinely cannot find anything that passes both \
             the obscurity AND quality filters, respond with just \"No golden find today.\"\
             \n\
             \nAfter you're done, call browser_cleanup to free resources.",
        );

        let cooldown_key = format!("golden_find:{}", interest);

        vec![UrgeSpec::new(
            "GoldenFind",
            &execute_msg,
            0.5,
        )
        .with_cooldown(&cooldown_key)
        .with_context(serde_json::json!({
            "research_type": "golden_find",
            "target_interest": interest,
            "location_aware": has_location,
        }))]
    }
}
