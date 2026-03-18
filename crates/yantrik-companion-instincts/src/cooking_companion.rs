//! Cooking Companion instinct — contextual food intelligence for the user's
//! cooking journey.
//!
//! This goes far beyond recipe suggestions. CookingCompanion understands
//! seasonality, food science, technique, and food-culture connections to
//! deliver genuinely useful food knowledge at the right moment.
//!
//! What makes it tick:
//! - **Seasonal awareness**: Ingredients have peak windows measured in weeks,
//!   not months. Ramps are 3 weeks. Hatch chiles are 6 weeks. This instinct
//!   knows when to sound the alarm.
//! - **Food science**: The *why* behind cooking techniques — why resting meat
//!   matters, how Maillard reactions work, why cold butter makes flakier pastry.
//! - **Weather-aware cravings**: Cold evening → soup science. Scorching day →
//!   ceviche technique. Rainy weekend → bread baking tips.
//! - **Location-aware markets**: Farmer's market finds, regional specialties,
//!   ethnic grocery store gems specific to the user's area.
//! - **Cultural depth**: The stories behind dishes — why ramen broth is simmered
//!   for 18 hours, why French onion soup was peasant food, why Nashville hot
//!   chicken was invented as revenge.
//!
//! Design principle: "I went on a small intellectual adventure FOR YOU and came
//! back with a gift" — food knowledge that makes the user a better, more
//! adventurous cook.
//!
//! This instinct is **opt-in via interests**: it only fires if the user has
//! cooking-related interests in their profile (cooking, food, recipe, baking,
//! chef, cuisine, kitchen, BBQ, grilling, culinary, meal).
//!
//! Preferred delivery window: late afternoon to evening (4 PM – 8 PM), when
//! people are thinking about what to cook for dinner.

use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, ModelTier, UrgeSpec};

/// Rotating lenses for food intelligence research.
///
/// Each lens represents a different angle of food knowledge. The instinct
/// cycles through them so the user gets variety — seasonal alerts one day,
/// food science the next, a cultural deep-dive after that.
const FOOD_LENSES: &[&str] = &[
    "seasonal ingredients peak flavor this month",
    "food science cooking technique tips",
    "farmer's market seasonal finds near {location}",
    "cuisine deep dive cultural food facts",
    "ingredient substitution hacks and upgrades",
    "kitchen tool or technique that changes everything",
    "food history and origin stories",
    "fermentation and preservation seasonal timing",
];

/// Keywords that indicate a user has cooking-related interests.
/// Matched case-insensitively against `state.user_interests`.
const COOKING_KEYWORDS: &[&str] = &[
    "cooking",
    "food",
    "recipe",
    "baking",
    "chef",
    "cuisine",
    "kitchen",
    "bbq",
    "grilling",
    "culinary",
    "meal",
];

/// Cooking Companion instinct — contextual food intelligence delivered at the
/// moment when it matters most.
///
/// Unlike a recipe app that waits to be asked, this instinct proactively
/// researches seasonal ingredients, food science insights, technique tips,
/// and food-culture connections tailored to the user's location, weather,
/// and cooking preferences.
pub struct CookingCompanionInstinct {
    /// Minimum seconds between food intelligence deliveries.
    interval_secs: f64,
    /// Timestamp of the last research cycle.
    last_check_ts: Mutex<f64>,
    /// Index into `FOOD_LENSES` — rotates to provide variety.
    topic_index: Mutex<usize>,
}

impl CookingCompanionInstinct {
    /// Create a new CookingCompanion instinct.
    ///
    /// # Arguments
    /// * `interval_hours` — Minimum hours between food intelligence deliveries.
    ///   Recommended: 6–12 hours (once or twice daily).
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
            topic_index: Mutex::new(0),
        }
    }

    /// Check whether the user has any cooking-related interests.
    ///
    /// Returns `true` if at least one entry in `state.user_interests` contains
    /// (or is contained by) a keyword from `COOKING_KEYWORDS`. Matching is
    /// case-insensitive.
    fn has_cooking_interest(state: &CompanionState) -> bool {
        if state.user_interests.is_empty() {
            return false;
        }

        let interests_lower: Vec<String> = state
            .user_interests
            .iter()
            .map(|i| i.to_lowercase())
            .collect();

        for keyword in COOKING_KEYWORDS {
            for interest in &interests_lower {
                if interest.contains(keyword) || keyword.contains(interest.as_str()) {
                    return true;
                }
            }
        }

        false
    }

    /// Advance the lens index and return the current food lens string.
    fn next_lens(&self) -> &'static str {
        let mut idx = self.topic_index.lock().unwrap();
        let lens = FOOD_LENSES[*idx % FOOD_LENSES.len()];
        *idx = idx.wrapping_add(1);
        lens
    }
}

impl Instinct for CookingCompanionInstinct {
    fn name(&self) -> &str {
        "CookingCompanion"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // ── Rate-limiting with cold-start guard ──────────────────────
        // On first evaluation after startup, record the timestamp but
        // don't fire — avoids a burst of messages on boot.
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

        // ── Interest gate ────────────────────────────────────────────
        // This instinct is opt-in: only fire if the user has expressed
        // cooking-related interests. No interests → silent.
        if !Self::has_cooking_interest(state) {
            return vec![];
        }

        // ── Time-of-day preference ──────────────────────────────────
        // Preferred window: 4 PM – 8 PM (16:00–20:00), when people are
        // thinking about dinner. Outside this window the instinct still
        // fires but at lower urgency — food knowledge is welcome anytime,
        // just more relevant in the evening.
        let hour = state.current_hour;
        let in_preferred_window = (16..=20).contains(&hour);
        let urgency = if in_preferred_window { 0.5 } else { 0.35 };

        // ── Rotate through food lenses ──────────────────────────────
        let lens = self.next_lens();

        // ── Build context ───────────────────────────────────────────
        let user = &state.config_user_name;
        let location = if state.user_location.is_empty() {
            "their area".to_string()
        } else {
            state.user_location.clone()
        };

        let interests_str = state.user_interests.join(", ");

        // ── EXECUTE prompt ──────────────────────────────────────────
        // The prompt guides the LLM through a research sequence:
        //   1. Recall the user's food preferences and cooking level
        //   2. Check current weather (temperature drives cravings)
        //   3. Web search with the current food lens
        //   4. Synthesize into one actionable piece of food intelligence
        let lens_with_location = lens.replace("{location}", &location);

        let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE You are {user}'s food-obsessed friend who also happens to be a \
                 scientist. Your job: go on a small intellectual adventure and come back \
                 with a gift — one piece of food knowledge that's genuinely useful TODAY.\
                 \n\n\
                 Step 1: Use recall with query \"cooking food preferences cuisine kitchen\" \
                 to find {user}'s food preferences, cooking level, favorite cuisines, and \
                 dietary restrictions. Their known interests include: {interests_str}.\
                 \n\n\
                 Step 2: Use get_weather to check current conditions in {location}. \
                 Temperature and weather shape food cravings:\
                 \n  - Cold/rainy → comfort food, soups, baking, braises, stews\
                 \n  - Hot/sunny → grilling, salads, ceviche, cold noodles, light dishes\
                 \n  - Mild/pleasant → anything goes, great for farmer's market trips\
                 \n\n\
                 Step 3: Use web_search to search for \"{lens_with_location}\". \
                 Look for ONE genuinely useful piece of food intelligence from this lens. \
                 Prioritize:\
                 \n  - Seasonal ingredient alerts with SPECIFIC timing (\"available for \
                 3 weeks starting now\", \"peak flavor in March\")\
                 \n  - Food science tips that improve a technique they likely use \
                 (\"why you should salt pasta water to 1% concentration\", \"the Maillard \
                 reaction starts at 280F — that's why you need a screaming hot pan\")\
                 \n  - Local market finds or food events near {location}\
                 \n  - Cultural food facts that deepen appreciation of a cuisine they love\
                 \n  - Ingredient upgrades: the $2 swap that transforms a dish\
                 \n\n\
                 Step 4: Deliver your find in 2-3 sentences. Be practical, specific, and \
                 enthusiastic without being cheesy. Write like a knowledgeable friend \
                 sharing something exciting over a beer, not a food blogger. Include:\
                 \n  - WHAT the insight is (specific ingredient, technique, or fact)\
                 \n  - WHY it matters right now (season, weather, availability window)\
                 \n  - HOW to act on it (where to get it, how to use it, what to pair it with)\
                 \n\n\
                 After you're done, call browser_cleanup to free resources.\
                 \n\n\
                 If nothing genuinely interesting or timely is found, respond with just \
                 \"No food intel today.\" — never force a mediocre recommendation.",
            ),
            ModelTier::Tiny => format!("EXECUTE SKIP"),
            _ => format!(
                "EXECUTE Task: Share one brief food or recipe idea with {user}.\n\
                 Tool: You may use recall once.\n\
                 Rule: Use only facts the user stated. Do not invent dietary preferences.\n\
                 Fallback: Skip.\n\
                 Output: 1 sentence."
            ),
        };

        vec![UrgeSpec::new(
            "CookingCompanion",
            &execute_msg,
            urgency,
        )
        .with_cooldown("cooking_companion:intel")
        .with_context(serde_json::json!({
            "research_type": "cooking_companion",
            "lens": lens,
            "weather_aware": true,
            "location": location,
            "preferred_window": in_preferred_window,
        }))]
    }
}
