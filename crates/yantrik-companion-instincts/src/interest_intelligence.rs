//! Interest Intelligence instinct — the core human-first proactive engine.
//!
//! Unlike generic curiosity (which cycles through categories), this instinct
//! understands WHAT the user loves and maps each interest to specific,
//! actionable research strategies.
//!
//! Examples:
//!   "likes fishing" → check fishing reports near area, weather windows, best spots
//!   "likes cooking" → seasonal recipes, grocery deals, "it's soup weather today"
//!   "likes tech news" → analyzed tech summaries filtered by their stack/interests
//!   "likes shopping" → deal alerts on items they mentioned wanting
//!   "likes hiking" → trail conditions, weather windows, sunrise times
//!   "likes sports/NFL" → game schedules, scores, trade news
//!
//! Pattern: recall_preferences → build targeted search → web_search → analyze → deliver
//!
//! Location-aware: uses user's location for local relevance.
//! Weather-aware: outdoor interests check weather first.
//! Time-aware: doesn't suggest fishing at midnight.

use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, ModelTier, UrgeSpec};

/// Interest-to-search strategy mappings.
/// Each strategy defines HOW to research a particular interest category.
const STRATEGIES: &[InterestStrategy] = &[
    InterestStrategy {
        category: "fishing",
        keywords: &["fishing", "angling", "bass", "fly fishing", "deep sea", "trout", "catfish"],
        search_template: "fishing report {location} this week conditions",
        weather_relevant: true,
        time_window: TimeWindow::DaytimeOnly,
        research_prompt: "EXECUTE First use get_weather to check current conditions. \
             Then use recall with query \"fishing preferences location\" to find where {user} fishes. \
             Then use web_search to search for \"fishing report {location} this week\". \
             Analyze the results: water temperature, what's biting, best times. \
             If conditions look good for fishing soon, share a brief 2-3 sentence recommendation \
             with specific details (what species, what time, weather window). \
             If conditions are poor, say so briefly with when it might improve. \
             After you're done, call browser_cleanup to free resources.",
        urgency: 0.5,
    },
    InterestStrategy {
        category: "hiking",
        keywords: &["hiking", "trails", "backpacking", "trekking", "outdoor", "nature walks"],
        search_template: "hiking trail conditions {location} this weekend",
        weather_relevant: true,
        time_window: TimeWindow::DaytimeOnly,
        research_prompt: "EXECUTE First use get_weather to check conditions for the next few days. \
             Then use recall with query \"hiking preferences trails\" to find {user}'s favorite trails or areas. \
             Then use web_search for \"best hiking trails {location} current conditions\". \
             If weather looks good for hiking, suggest a specific trail with conditions, distance, \
             and best time to go. Include sunrise/sunset awareness. 2-3 sentences max. \
             After you're done, call browser_cleanup to free resources.",
        urgency: 0.5,
    },
    InterestStrategy {
        category: "cooking",
        keywords: &["cooking", "recipes", "baking", "cuisine", "chef", "food", "meal prep"],
        search_template: "seasonal recipes {season} easy dinner",
        weather_relevant: true,
        time_window: TimeWindow::EveningPreferred,
        research_prompt: "EXECUTE Use get_weather to check today's conditions. \
             Then use recall with query \"cooking food preferences cuisine\" to find what {user} likes to cook. \
             Then use web_search for a recipe idea that matches: their cuisine preferences + current weather \
             (comfort food if cold/rainy, light meals if hot, grilling if nice out). \
             Share ONE specific recipe suggestion in 2-3 sentences — name the dish, why it fits today, \
             and one interesting detail. Keep it natural, not like a recipe card. \
             After you're done, call browser_cleanup to free resources.",
        urgency: 0.4,
    },
    InterestStrategy {
        category: "tech_news",
        keywords: &["tech", "programming", "software", "AI", "machine learning", "coding", "developer", "startup"],
        search_template: "tech news today {interest_detail}",
        weather_relevant: false,
        time_window: TimeWindow::MorningPreferred,
        research_prompt: "EXECUTE Use recall with query \"tech interests programming languages frameworks\" \
             to find what tech {user} is into. \
             Then use web_search for recent tech news filtered to their specific interests \
             (e.g., if they like Rust, search \"rust programming news this week\"). \
             Find ONE genuinely interesting development and ANALYZE it — don't just report the headline. \
             Explain why it matters, what it means for their work/interests. 2-3 sentences. \
             After you're done, call browser_cleanup to free resources.",
        urgency: 0.5,
    },
    InterestStrategy {
        category: "sports",
        keywords: &["sports", "football", "NFL", "NBA", "soccer", "baseball", "hockey", "cricket", "tennis", "F1", "racing"],
        search_template: "{interest_detail} scores schedule today",
        weather_relevant: false,
        time_window: TimeWindow::Anytime,
        research_prompt: "EXECUTE Use recall with query \"sports teams preferences\" to find what teams/sports {user} follows. \
             Then use web_search for their team's latest: upcoming games, recent scores, trade news, injuries. \
             Share the most relevant update in 2-3 sentences. If there's a game today, lead with that. \
             After you're done, call browser_cleanup to free resources.",
        urgency: 0.6,
    },
    InterestStrategy {
        category: "fitness",
        keywords: &["fitness", "gym", "workout", "running", "yoga", "CrossFit", "weightlifting", "exercise"],
        search_template: "workout suggestion {fitness_type} today",
        weather_relevant: true,
        time_window: TimeWindow::MorningPreferred,
        research_prompt: "EXECUTE Use get_weather to check today's conditions. \
             Use recall with query \"fitness workout preferences routine\" to find {user}'s fitness habits. \
             If they run/cycle outdoors, factor in weather. Suggest a workout idea that fits today — \
             outdoor if nice, indoor alternative if not. Or note a rest day if they've been active. \
             Keep it to 1-2 encouraging sentences. \
             After you're done, call browser_cleanup to free resources.",
        urgency: 0.4,
    },
    InterestStrategy {
        category: "gardening",
        keywords: &["gardening", "plants", "garden", "landscaping", "growing", "vegetables", "flowers"],
        search_template: "gardening tips {location} {season} what to plant",
        weather_relevant: true,
        time_window: TimeWindow::MorningPreferred,
        research_prompt: "EXECUTE Use get_weather to check conditions and forecast. \
             Use recall with query \"gardening plants zone location\" to find {user}'s garden details. \
             Then use web_search for timely gardening advice for their zone/season. \
             Share ONE actionable tip: what to plant now, frost warnings, watering advice based on forecast, \
             or pest alerts for the season. 1-2 sentences. \
             After you're done, call browser_cleanup to free resources.",
        urgency: 0.4,
    },
    InterestStrategy {
        category: "movies_tv",
        keywords: &["movies", "film", "TV", "shows", "Netflix", "streaming", "cinema", "series", "anime"],
        search_template: "new {interest_detail} releases this week",
        weather_relevant: false,
        time_window: TimeWindow::EveningPreferred,
        research_prompt: "EXECUTE Use recall with query \"movie TV show preferences genres\" to find what {user} watches. \
             Then use web_search for new releases or highly-rated content matching their taste. \
             Suggest ONE specific thing to watch with a brief, spoiler-free reason why they'd like it. \
             2 sentences max. Don't be a generic recommendation engine — connect to what you know about them. \
             After you're done, call browser_cleanup to free resources.",
        urgency: 0.35,
    },
    InterestStrategy {
        category: "music",
        keywords: &["music", "concerts", "albums", "playlist", "guitar", "piano", "band", "DJ"],
        search_template: "new music releases {interest_detail} this week",
        weather_relevant: false,
        time_window: TimeWindow::Anytime,
        research_prompt: "EXECUTE Use recall with query \"music preferences artists genres\" to find {user}'s music taste. \
             Then use web_search for new releases, upcoming concerts near {location}, or music news \
             in their preferred genres. Share ONE interesting find in 1-2 sentences. \
             After you're done, call browser_cleanup to free resources.",
        urgency: 0.35,
    },
    InterestStrategy {
        category: "photography",
        keywords: &["photography", "camera", "photo", "landscape photography", "portrait", "drone"],
        search_template: "golden hour {location} today photography conditions",
        weather_relevant: true,
        time_window: TimeWindow::DaytimeOnly,
        research_prompt: "EXECUTE Use get_weather to check sky conditions (clouds, visibility, precipitation). \
             Use recall with query \"photography preferences style camera\" to find what {user} shoots. \
             If conditions are good for photography today (golden hour, dramatic clouds, clear night for astro), \
             mention it with specific timing. If there are interesting local events or seasonal opportunities, \
             note those. 1-2 sentences. \
             After you're done, call browser_cleanup to free resources.",
        urgency: 0.4,
    },
    InterestStrategy {
        category: "travel",
        keywords: &["travel", "vacation", "trip", "flights", "hotels", "destinations", "road trip"],
        search_template: "travel deals {interest_detail} cheap flights",
        weather_relevant: false,
        time_window: TimeWindow::Anytime,
        research_prompt: "EXECUTE Use recall with query \"travel preferences destinations wishlist\" to find \
             where {user} wants to go or has been. \
             Then use web_search for travel deals, flight sales, or interesting travel news for those destinations. \
             If there's a genuinely good deal or interesting opportunity, share it in 2-3 sentences. \
             After you're done, call browser_cleanup to free resources.",
        urgency: 0.4,
    },
    InterestStrategy {
        category: "gaming",
        keywords: &["gaming", "video games", "PC gaming", "console", "Steam", "PlayStation", "Xbox", "Nintendo"],
        search_template: "new game releases {interest_detail} this week",
        weather_relevant: false,
        time_window: TimeWindow::EveningPreferred,
        research_prompt: "EXECUTE Use recall with query \"gaming preferences games platforms\" to find what {user} plays. \
             Then use web_search for new releases, sales, or updates for their platform/genre preferences. \
             Share ONE relevant gaming update in 1-2 sentences. \
             After you're done, call browser_cleanup to free resources.",
        urgency: 0.35,
    },
    InterestStrategy {
        category: "finance",
        keywords: &["stocks", "investing", "crypto", "market", "finance", "trading", "portfolio"],
        search_template: "stock market today {interest_detail} analysis",
        weather_relevant: false,
        time_window: TimeWindow::MorningPreferred,
        research_prompt: "EXECUTE Use recall with query \"investment stocks portfolio interests\" to find \
             what {user} tracks financially. \
             Then use web_search for market analysis relevant to their holdings or interests. \
             Share ONE market insight in 2-3 sentences — what moved, why, and what it might mean. \
             Be analytical, not alarmist. \
             After you're done, call browser_cleanup to free resources.",
        urgency: 0.5,
    },
    InterestStrategy {
        category: "pets",
        keywords: &["dog", "cat", "pets", "puppy", "kitten", "aquarium", "reptile", "bird"],
        search_template: "{interest_detail} care tips seasonal advice",
        weather_relevant: true,
        time_window: TimeWindow::Anytime,
        research_prompt: "EXECUTE Use recall with query \"pet preferences animals\" to find what pets {user} has. \
             Use get_weather to check conditions (heat warnings for dogs, cold for outdoor cats, etc.). \
             Then use web_search for timely pet care advice relevant to their pet + current conditions. \
             Share ONE useful tip in 1-2 sentences. \
             After you're done, call browser_cleanup to free resources.",
        urgency: 0.4,
    },
];

/// Time window when this interest's research is most relevant.
#[derive(Clone, Copy)]
enum TimeWindow {
    /// Only suggest during daytime hours (7 AM - 7 PM).
    DaytimeOnly,
    /// Prefer morning delivery (6 AM - 11 AM).
    MorningPreferred,
    /// Prefer evening delivery (5 PM - 10 PM).
    EveningPreferred,
    /// Any time is fine.
    Anytime,
}

struct InterestStrategy {
    category: &'static str,
    keywords: &'static [&'static str],
    #[allow(dead_code)]
    search_template: &'static str,
    weather_relevant: bool,
    time_window: TimeWindow,
    research_prompt: &'static str,
    urgency: f64,
}

pub struct InterestIntelligenceInstinct {
    /// Minimum seconds between any interest research.
    interval_secs: f64,
    /// Last research timestamp.
    last_research_ts: Mutex<f64>,
    /// Index to rotate through matched strategies.
    strategy_index: Mutex<usize>,
}

impl InterestIntelligenceInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_research_ts: Mutex::new(0.0),
            strategy_index: Mutex::new(0),
        }
    }

    /// Match user interests (from CompanionState) to research strategies.
    fn match_strategies(&self, state: &CompanionState) -> Vec<&'static InterestStrategy> {
        let interests_lower: Vec<String> = state
            .user_interests
            .iter()
            .map(|i| i.to_lowercase())
            .collect();

        if interests_lower.is_empty() {
            // No known interests yet — fall back to generic curiosity behavior
            return vec![];
        }

        let mut matched: Vec<&InterestStrategy> = Vec::new();

        for strategy in STRATEGIES {
            for keyword in strategy.keywords {
                let kw_lower = keyword.to_lowercase();
                if interests_lower.iter().any(|interest| {
                    interest.contains(&kw_lower) || kw_lower.contains(interest.as_str())
                }) {
                    matched.push(strategy);
                    break; // Don't match same strategy twice
                }
            }
        }

        matched
    }

    /// Check if current hour is appropriate for the time window.
    fn is_good_time(window: TimeWindow, hour: u32) -> bool {
        match window {
            TimeWindow::DaytimeOnly => (7..=19).contains(&hour),
            TimeWindow::MorningPreferred => (6..=11).contains(&hour),
            TimeWindow::EveningPreferred => (17..=22).contains(&hour),
            TimeWindow::Anytime => true,
        }
    }
}

impl Instinct for InterestIntelligenceInstinct {
    fn name(&self) -> &str {
        "InterestIntelligence"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // Rate-limit (cold-start guard)
        {
            let mut last = self.last_research_ts.lock().unwrap();
            if *last == 0.0 {
                *last = now;
                return vec![];
            }
            if now - *last < self.interval_secs {
                return vec![];
            }
            *last = now;
        }

        // Find strategies that match user's interests
        let matched = self.match_strategies(state);
        if matched.is_empty() {
            return vec![];
        }

        // Pick next strategy (round-robin through matched ones)
        let strategy = {
            let mut idx = self.strategy_index.lock().unwrap();
            let s = matched[*idx % matched.len()];
            *idx = idx.wrapping_add(1);
            s
        };

        // Estimate current hour from timestamp (UTC-based, rough)
        // The LLM will use actual local time from date_calc if needed.
        let approx_hour = ((now as u64 / 3600) % 24) as u32;

        // Check time window — if not ideal time, skip this cycle
        // (but Anytime strategies always pass)
        if !Self::is_good_time(strategy.time_window, approx_hour) {
            return vec![];
        }

        let user = &state.config_user_name;
        let location = if state.user_location.is_empty() {
            "nearby".to_string()
        } else {
            state.user_location.clone()
        };

        // Build the EXECUTE prompt with user context
        let execute_msg = match state.model_tier {
            ModelTier::Large => strategy
                .research_prompt
                .replace("{user}", user)
                .replace("{location}", &location),
            ModelTier::Tiny => format!("EXECUTE SKIP"),
            _ => format!(
                "EXECUTE Task: Share one update for {user}'s interest.\n\
                 Tool: Use web search once.\n\
                 Rule: Only share verified facts from search results. Do not invent news.\n\
                 Fallback: \"Nothing new.\"\n\
                 Output: 1 sentence."
            ),
        };

        let mut urgency = strategy.urgency;

        // Boost urgency for weather-relevant activities on nice days
        // (The LLM will determine actual weather — this is a heuristic boost)
        if strategy.weather_relevant {
            urgency += 0.05; // Slight boost — outdoor activities are time-sensitive
        }

        vec![UrgeSpec::new(
            "InterestIntelligence",
            &execute_msg,
            urgency,
        )
        .with_cooldown(&format!("interest:{}", strategy.category))
        .with_context(serde_json::json!({
            "category": strategy.category,
            "weather_relevant": strategy.weather_relevant,
            "location": location,
            "research_type": "interest_intelligence",
        }))]
    }
}
