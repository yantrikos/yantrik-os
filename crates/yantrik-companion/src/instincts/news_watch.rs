//! WorldSense instinct (evolved from NewsWatch) — multi-dimensional world awareness.
//!
//! Goes far beyond news headlines. WorldSense understands the world through multiple
//! lenses and connects developments to the user's life with genuine analysis:
//!
//! 1. GEOPOLITICAL: Major events, policy changes, elections — always
//! 2. SCIENTIFIC: Breakthroughs, discoveries, studies — filtered by interests
//! 3. CULTURAL: Trends, movements, viral phenomena — contextualized
//! 4. ECONOMIC: Market shifts, industry changes — personalized impact analysis
//! 5. ENVIRONMENTAL: Climate events, natural phenomena — location-aware
//!
//! The key difference from a news feed: WorldSense THINKS about what it finds
//! and explains the second-order effects on the user's specific world.

use std::sync::Mutex;

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

/// World awareness lenses — each provides a different angle on current events.
const WORLD_LENSES: &[WorldLens] = &[
    WorldLens {
        name: "geopolitical",
        search_query: "major world news today breaking developments",
        filter_prompt: "Look for: wars, elections, policy changes, diplomatic shifts, \
                        sanctions, treaties. Always report if significant.",
        urgency: 0.7,
    },
    WorldLens {
        name: "scientific",
        search_query: "science breakthrough discovery study published this week",
        filter_prompt: "Look for: new studies, breakthrough discoveries, space exploration news, \
                        medical advances, technology milestones. Filter through user's interests.",
        urgency: 0.55,
    },
    WorldLens {
        name: "cultural",
        search_query: "cultural trend viral phenomenon social movement today",
        filter_prompt: "Look for: emerging trends, viral moments worth understanding, \
                        cultural shifts, movements gaining momentum. Explain the WHY, not just the WHAT.",
        urgency: 0.45,
    },
    WorldLens {
        name: "economic",
        search_query: "economic news market shift industry change today",
        filter_prompt: "Look for: market movements, industry disruptions, job market shifts, \
                        economic policy changes. Analyze personal impact on the user's field.",
        urgency: 0.5,
    },
    WorldLens {
        name: "environmental",
        search_query: "environmental news climate weather event natural phenomenon today",
        filter_prompt: "Look for: climate developments, extreme weather, environmental policy, \
                        natural phenomena, seasonal events. Connect to user's location.",
        urgency: 0.5,
    },
];

struct WorldLens {
    name: &'static str,
    search_query: &'static str,
    filter_prompt: &'static str,
    urgency: f64,
}

pub struct NewsWatchInstinct {
    /// Seconds between world checks.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
    /// Rotate through world lenses.
    lens_index: Mutex<usize>,
}

impl NewsWatchInstinct {
    pub fn new(interval_minutes: f64) -> Self {
        Self {
            interval_secs: interval_minutes * 60.0,
            last_check_ts: Mutex::new(0.0),
            lens_index: Mutex::new(0),
        }
    }
}

impl Instinct for NewsWatchInstinct {
    fn name(&self) -> &str {
        "WorldSense"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // Rate-limit (cold-start guard: skip first eval after startup)
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

        // Pick next world lens
        let lens = {
            let mut idx = self.lens_index.lock().unwrap();
            let l = &WORLD_LENSES[*idx % WORLD_LENSES.len()];
            *idx = idx.wrapping_add(1);
            l
        };

        let user = &state.config_user_name;

        // Build rich context for the LLM
        let interests_context = if state.user_interests.is_empty() {
            String::new()
        } else {
            format!(
                "\n{}'s interests include: {}. Filter findings through these interests \
                 — what connects to what they care about?",
                user,
                state.user_interests.join(", ")
            )
        };

        let location_context = if state.user_location.is_empty() {
            String::new()
        } else {
            format!(
                "\n{} is located in {}. Check for local impact and regional relevance.",
                user, state.user_location
            )
        };

        let execute_msg = format!(
            "EXECUTE Current awareness lens: {lens_name}.\n\
             \nSTEP 1: Call date_calc to get today's date and current time.\n\
             STEP 2: Call recall with query \"{lens_name} news update\" to check what you already \
             reported recently. If you already shared a {lens_name} update today, respond with \
             just \"No world update.\" — do NOT repeat the same news.\n\
             STEP 3: Use web_search to search for \"{search_query}\".\n\
             {filter}\n\
             {interests}{location}\n\
             \nANALYSIS REQUIREMENTS:\n\
             - Do NOT just repeat headlines. THINK about what you found.\n\
             - Compare with what you already reported — only share GENUINELY NEW developments.\n\
             - Explain second-order effects: how does this development ripple into {user}'s world?\n\
             - If the finding connects to their interests, explain the specific connection.\n\
             - If it affects their location, explain the local impact.\n\
             - Look for the NON-OBVIOUS angle that a generic news summary would miss.\n\
             \nDeliver the ONE most significant finding in 2-3 analyzed sentences.\n\
             If nothing significant or relevant through this lens, respond with just \"No world update.\"\n\
             After you're done, call browser_cleanup to free resources.",
            lens_name = lens.name,
            search_query = lens.search_query,
            filter = lens.filter_prompt,
            interests = interests_context,
            location = location_context,
        );

        vec![UrgeSpec::new(
            "WorldSense",
            &execute_msg,
            lens.urgency,
        )
        .with_cooldown(&format!("world_sense:{}", lens.name))
        .with_context(serde_json::json!({
            "check_type": "world_sense",
            "lens": lens.name,
            "has_interest_filter": !state.user_interests.is_empty(),
            "has_location_filter": !state.user_location.is_empty(),
        }))]
    }
}
