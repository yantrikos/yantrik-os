//! Activity Recommender instinct — "perfect day for..." suggestions.
//!
//! Cross-references the user's interests with current weather, time of day,
//! day of week, and location to suggest activities at the right moment.
//!
//! This is the instinct that says:
//!   "Beautiful Saturday morning — 72°F, light breeze. Perfect conditions for
//!    fishing at your usual spot. Bass should be active in this weather."
//!
//! Or:
//!   "Rainy evening ahead — great excuse to try that pasta recipe you bookmarked."
//!
//! Or:
//!   "Clear skies tonight with no moon — amazing for astrophotography if you're up for it."
//!
//! Unlike InterestIntelligence (which researches specific topics), this instinct
//! focuses on the MOMENT — matching conditions to activities.

use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec, ModelTier};

/// Outdoor activity categories that benefit from weather awareness.
const OUTDOOR_KEYWORDS: &[&str] = &[
    "fishing", "hiking", "cycling", "running", "golf", "kayaking", "surfing",
    "camping", "photography", "gardening", "bird watching", "picnic",
    "tennis", "soccer", "basketball", "baseball", "swimming", "sailing",
    "rock climbing", "skateboarding", "dog walking",
];

/// Indoor activity suggestions for bad weather.
const INDOOR_KEYWORDS: &[&str] = &[
    "cooking", "baking", "reading", "gaming", "movies", "music",
    "painting", "drawing", "puzzles", "board games", "yoga",
    "meditation", "coding", "writing", "crafts",
];

pub struct ActivityRecommenderInstinct {
    /// Seconds between recommendation checks.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl ActivityRecommenderInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for ActivityRecommenderInstinct {
    fn name(&self) -> &str {
        "ActivityRecommender"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // Rate-limit (cold-start guard)
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

        // Only fire if we know some interests
        if state.user_interests.is_empty() {
            return vec![];
        }

        // Classify user's interests into outdoor vs indoor
        let interests_lower: Vec<String> = state
            .user_interests
            .iter()
            .map(|i| i.to_lowercase())
            .collect();

        let has_outdoor = interests_lower.iter().any(|i| {
            OUTDOOR_KEYWORDS.iter().any(|kw| i.contains(kw))
        });

        let has_indoor = interests_lower.iter().any(|i| {
            INDOOR_KEYWORDS.iter().any(|kw| i.contains(kw))
        });

        if !has_outdoor && !has_indoor {
            return vec![];
        }

        let user = &state.config_user_name;
        let location = if state.user_location.is_empty() {
            "their area".to_string()
        } else {
            state.user_location.clone()
        };

        // Build a context-aware recommendation prompt
        let interests_str = state.user_interests.join(", ");

                let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE First use get_weather to check current and forecast conditions in {location}. \
             Then use date_calc to check what day of the week it is and the time. \
             \
             {user}'s interests include: {interests_str}. \
             \
             Based on CURRENT conditions (weather, day, time), suggest ONE perfect-for-right-now activity \
             from their interests. Be SPECIFIC: \
             - If it's a beautiful morning and they fish: suggest fishing with conditions details \
             - If it's raining and they cook: suggest a cozy recipe that fits the weather \
             - If it's a clear night and they photograph: mention astrophotography conditions \
             - If it's a weekend morning: suggest something fun, not work \
             - If it's a weekday evening: suggest something relaxing \
             \
             The magic is MATCHING THE MOMENT to the activity. Don't just say 'go fishing' — say \
             'Perfect morning for bass fishing — 68°F, overcast skies keep them near the surface.' \
             \
             Keep it to 2-3 natural sentences. If conditions don't particularly favor any activity, \
             respond with just 'Nothing special to suggest right now.' \
             After you're done, call browser_cleanup to free resources.",
            ),
            ModelTier::Tiny => format!(
                "EXECUTE Suggest one activity for {user}. Output: 1 sentence.",
            ),
            _ => format!(
                "EXECUTE Task: Suggest one interesting thing for {user}.\n\
             Input: interest={interests_str}.\n\
             Tool: You may use recall or web search once.\n\
             Rule: Do not invent facts. Do not repeat recent suggestions.\n\
             Fallback: \"No suggestion right now.\"\n\
             Output: 1 sentence.",
            ),
        };

        vec![UrgeSpec::new("ActivityRecommender", &execute_msg, 0.55)
            .with_cooldown("activity:recommend")
            .with_context(serde_json::json!({
                "has_outdoor_interests": has_outdoor,
                "has_indoor_interests": has_indoor,
                "location": location,
                "research_type": "activity_recommendation",
            }))]
    }
}
