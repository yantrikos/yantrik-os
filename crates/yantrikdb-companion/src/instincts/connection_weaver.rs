//! ConnectionWeaver instinct — finds surprising connections between the user's
//! different interests, memories, and current events.
//!
//! This is the "I just realized something interesting" instinct. It picks 2-3
//! of the user's known interests, researches them, and looks for non-obvious
//! bridges — an unexpected parallel, a current event that ties them together,
//! or an insight from one domain that illuminates another.
//!
//! Design principle: "I went on a small intellectual adventure FOR YOU and came
//! back with a gift." The instinct should THINK and RESEARCH, not just state
//! facts.
//!
//! Examples:
//!   User likes fishing AND cooking → "I was reading about seasonal fish runs
//!     near Austin and realized the redfish are running right now — perfect
//!     timing since you mentioned wanting to try that blackened redfish recipe"
//!   User works in Rust AND likes music → "Interesting parallel — the Rust
//!     compiler's borrow checker works like a music conductor ensuring no two
//!     instruments play the same note simultaneously"
//!   User mentioned a problem at work AND has a hobby → draws an unexpected
//!     insight from the hobby that applies to the work problem
//!
//! Pattern: pick 2-3 interests → recall memories → find bridge → web_search
//!   for current connection → craft genuine "aha moment" insight
//!
//! Time-aware: only fires during daytime/evening (8 AM - 10 PM).
//! Rotation: deterministic cycling through interest combinations for variety.

use std::sync::Mutex;

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

/// ConnectionWeaver — discovers surprising bridges between the user's interests.
pub struct ConnectionWeaverInstinct {
    /// Minimum seconds between connection-weaving attempts.
    interval_secs: f64,
    /// Timestamp of the last check.
    last_check_ts: Mutex<f64>,
    /// Rotation index for deterministic cycling through interest combinations.
    combination_index: Mutex<usize>,
}

impl ConnectionWeaverInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
            combination_index: Mutex::new(0),
        }
    }

    /// Pick 2-3 interests from the user's list using deterministic rotation.
    ///
    /// Uses a combination index that wraps around, producing different pairs
    /// and triples each time so the user gets variety over time.
    fn pick_interests<'a>(&self, interests: &'a [String]) -> Vec<&'a String> {
        let n = interests.len();
        if n < 2 {
            return vec![];
        }

        let mut idx = self.combination_index.lock().unwrap();
        let current = *idx;
        *idx = idx.wrapping_add(1);

        // Total possible pairs: n*(n-1)/2. We cycle through them.
        // For triples (when n >= 3), we interleave: even indices pick pairs,
        // odd indices pick triples.
        if n >= 3 && current % 2 == 1 {
            // Pick a triple using modular arithmetic
            let base = current / 2;
            let i = base % n;
            let j = (i + 1 + (base / n) % (n - 1)) % n;
            let mut k = (j + 1 + (base / (n * n)) % (n - 2)) % n;
            // Ensure k doesn't collide with i or j
            while k == i || k == j {
                k = (k + 1) % n;
            }
            vec![&interests[i], &interests[j], &interests[k]]
        } else {
            // Pick a pair
            let total_pairs = n * (n - 1) / 2;
            let pair_idx = (current / 2) % total_pairs;

            // Map linear index to (i, j) pair
            let mut count = 0;
            for i in 0..n {
                for j in (i + 1)..n {
                    if count == pair_idx {
                        return vec![&interests[i], &interests[j]];
                    }
                    count += 1;
                }
            }
            // Fallback: first two
            vec![&interests[0], &interests[1]]
        }
    }
}

impl Instinct for ConnectionWeaverInstinct {
    fn name(&self) -> &str {
        "ConnectionWeaver"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // ── Rate-limit with cold-start guard ──
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

        // ── Time window: 8 AM - 10 PM only ──
        let hour = state.current_hour;
        if !(8..=22).contains(&hour) {
            return vec![];
        }

        // ── Need at least 2 interests to find connections ──
        if state.user_interests.len() < 2 {
            return vec![];
        }

        // ── Pick 2-3 interests via deterministic rotation ──
        let picked = self.pick_interests(&state.user_interests);
        if picked.len() < 2 {
            return vec![];
        }

        let interests_list = picked
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        let interests_quoted = picked
            .iter()
            .map(|s| format!("\"{}\"", s))
            .collect::<Vec<_>>()
            .join(" and ");

        let user = &state.config_user_name;
        let location = if state.user_location.is_empty() {
            "their area".to_string()
        } else {
            state.user_location.clone()
        };

        // ── Build the EXECUTE prompt ──
        let execute_msg = format!(
            "EXECUTE You are going on a small intellectual adventure for {user}. \
             Your mission: find a SURPRISING, NON-OBVIOUS connection between these \
             interests of theirs: {interests_quoted}.\n\n\
             Step 1: Use recall with query \"{interests_list} preferences experiences\" \
             to pull up what you know about {user}'s relationship with each of these \
             interests. Look for specific details — projects, preferences, past \
             conversations, problems they mentioned.\n\n\
             Step 2: THINK deeply. What connects {interests_quoted} in a way that \
             isn't immediately obvious? Look for:\n\
             - A principle from one domain that illuminates the other\n\
             - A current event or development that bridges them\n\
             - A technique or concept that transfers between them\n\
             - A historical or cultural thread that ties them together\n\
             - Something happening right now near {location} that involves both\n\n\
             Step 3: Use web_search to verify or enrich your connection. Search for \
             something specific that bridges these interests — a recent article, \
             a new development, a local event, a scientific finding. Do NOT search \
             for generic \"connection between X and Y\". Instead, search for the \
             SPECIFIC bridge you identified (e.g., if you noticed a biomechanics \
             parallel between their sport and their engineering work, search for \
             the specific concept).\n\n\
             Step 4: Craft your insight in 2-3 sentences. It should feel like a \
             genuine \"I just realized something interesting\" moment — a discovery \
             story, not a Wikipedia summary. Be SPECIFIC: reference concrete details \
             from {user}'s memories and from your research. If you found a current \
             event or fact that creates the bridge, lead with that.\n\n\
             IMPORTANT RULES:\n\
             - Do NOT be generic. \"Both X and Y require patience\" is BORING. \
             Find something that would make {user} say \"huh, I never thought of it that way.\"\n\
             - Do NOT lecture. Share the insight like you're excitedly telling a friend \
             something cool you discovered.\n\
             - If you can't find a genuinely interesting connection, say nothing rather \
             than forcing a weak one. Simply call browser_cleanup and stop.\n\
             - Keep it to 2-3 sentences. The insight should be dense, not padded.\n\n\
             After you're done, call browser_cleanup to free resources.",
            user = user,
            interests_quoted = interests_quoted,
            interests_list = interests_list,
            location = location,
        );

        let interests_used: Vec<&str> = picked.iter().map(|s| s.as_str()).collect();

        vec![UrgeSpec::new(
            "ConnectionWeaver",
            &execute_msg,
            0.5,
        )
        .with_cooldown("connection_weaver:insight")
        .with_context(serde_json::json!({
            "interests_used": interests_used,
            "research_type": "connection_weaving",
        }))]
    }
}
