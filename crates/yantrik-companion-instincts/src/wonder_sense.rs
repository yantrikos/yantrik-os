//! WonderSense instinct — the "Alive Mind" that surfaces genuine wonder.
//!
//! This instinct is the companion's intellectual curiosity made visible. It goes
//! on small research adventures and comes back with gifts: fascinating facts,
//! surprising connections, and "huh, I never thought about that" moments.
//!
//! Unlike trivia bots that fire random facts, WonderSense is deeply contextual:
//! it considers the time of year, the day of the week, the user's location,
//! and their interests — then finds the ONE thing at the intersection of all
//! those contexts that's genuinely surprising.
//!
//! The design principle is simple: "I went on a small intellectual adventure
//! FOR YOU and came back with a gift." Pure wonder, zero utility pressure.
//!
//! Examples of what WonderSense surfaces:
//!
//! - It's the spring equinox: "Today's the equinox — the one day eggs supposedly
//!   balance on their end. That's actually a myth, but here's what IS true: the
//!   equinox is the only day the sun rises exactly due east everywhere on Earth."
//!
//! - It's raining: "Fun fact about the rain outside: that 'petrichor' smell comes
//!   from bacteria in the soil releasing geosmin. Humans can detect it at 5 parts
//!   per trillion — we're more sensitive to it than sharks are to blood."
//!
//! - User is in Austin: "Austin's bat colony under Congress Avenue Bridge is about
//!   to start its spring emergence — 1.5 million Mexican free-tailed bats. Peak
//!   viewing starts in about 2 weeks."
//!
//! - It's Friday: "The word 'Friday' comes from Frigg, Norse goddess of love —
//!   which is why Romance languages call it 'Viernes' (Venus's day). Same deity,
//!   different mythology."
//!
//! WonderSense rotates through a set of "lenses" — different angles from which
//! to look at the world. Each lens gives the research a starting direction, but
//! the LLM is free to follow the thread wherever it leads, as long as it stays
//! connected to the user's world.
//!
//! This instinct is what makes the companion feel like a friend who reads
//! encyclopedias for fun and can't help sharing the best parts.

use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec, ModelTier};

/// Wonder "lenses" — rotating angles from which to look at the world.
///
/// Each lens gives the research a starting direction. The LLM can follow
/// the thread wherever it leads, but the lens ensures we don't always
/// come back with the same kind of fact. Over the course of a week,
/// the user gets etymology one day, astronomy the next, food science
/// after that — a well-rounded intellectual diet.
///
/// The `{location}` placeholder is replaced with the user's actual location
/// when available, making geography and nature lenses locally relevant.
const WONDER_LENSES: &[&str] = &[
    "today in history",
    "science of everyday things",
    "etymology and word origins",
    "nature and biology near {location}",
    "astronomy and space today",
    "food science and culinary facts",
    "human psychology and behavior",
    "geography and place facts about {location}",
    "mathematics in daily life",
    "music and sound science",
    "weather science",
    "technology history",
];

/// Map day-of-week number (0=Sunday, 1=Monday, ..., 6=Saturday) to name.
const DAY_NAMES: &[&str] = &[
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// The WonderSense instinct — surfaces "did you know?" moments driven by
/// genuine curiosity about the world, personalized to the user's context.
///
/// This is not a trivia engine. It's the instinct of a mind that finds the
/// world endlessly interesting and wants to share that feeling. Every fact
/// it surfaces should make you pause and think, not just nod and forget.
pub struct WonderSenseInstinct {
    /// Minimum seconds between wonder deliveries.
    interval_secs: f64,
    /// Timestamp of the last evaluation that produced an urge (cold-start guard).
    last_check_ts: Mutex<f64>,
    /// Rotating index into WONDER_LENSES, so we cycle through different angles.
    topic_index: Mutex<usize>,
}

impl WonderSenseInstinct {
    /// Create a new WonderSense instinct.
    ///
    /// # Arguments
    /// * `interval_hours` — Minimum hours between wonder moments. A value of
    ///   4.0 means at most ~6 wonder facts per day, which keeps them feeling
    ///   like treats rather than noise.
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
            topic_index: Mutex::new(0),
        }
    }

    /// Pick the next wonder lens (round-robin) and return it with location
    /// placeholders resolved.
    fn next_lens(&self, location: &str) -> String {
        let mut idx = self.topic_index.lock().unwrap();
        let lens = WONDER_LENSES[*idx % WONDER_LENSES.len()];
        *idx = idx.wrapping_add(1);
        lens.replace("{location}", location)
    }

    /// Determine urgency based on time of day.
    ///
    /// Wonder lands best at the bookends of the day — morning (7-11 AM) when
    /// the mind is fresh and curious, and evening (6-9 PM) when people are
    /// unwinding and receptive to "huh, neat" moments. Other hours still
    /// work, just with slightly lower priority.
    fn urgency_for_hour(hour: u32) -> f64 {
        match hour {
            7..=11 => 0.45,  // Morning wonder — fresh mind, open curiosity
            18..=21 => 0.45, // Evening wonder — unwinding, receptive
            _ => 0.35,       // Still welcome, just not peak timing
        }
    }
}

impl Instinct for WonderSenseInstinct {
    fn name(&self) -> &str {
        "WonderSense"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // Cold-start guard: on the very first evaluation after startup,
        // record the timestamp and skip. This prevents a burst of wonder
        // facts the moment the companion boots up.
        //
        // Standard interval check: only fire once per interval_secs.
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

        // Resolve location — use the user's known location, or a neutral
        // fallback that still works in prompts.
        let location = if state.user_location.is_empty() {
            "the user's area".to_string()
        } else {
            state.user_location.clone()
        };

        // Pick the next wonder lens (rotates each evaluation)
        let lens = self.next_lens(&location);

        // Resolve the day-of-week name for prompt context
        let day_name = DAY_NAMES
            .get(state.current_day_of_week as usize)
            .copied()
            .unwrap_or("today");

        // Build interest context — if we know what the user cares about,
        // we can find facts that BRIDGE the wonder lens with their world.
        let interests_clause = if state.user_interests.is_empty() {
            String::new()
        } else {
            format!(
                "\n   {}'s interests include: {}. Try to find a fact that BRIDGES \
                 the wonder lens with one of these interests — the intersection \
                 of two domains is where the best \"huh, I never thought about that\" \
                 moments live.",
                state.config_user_name,
                state.user_interests.join(", ")
            )
        };

        // Build location context for the prompt
        let location_clause = if state.user_location.is_empty() {
            String::new()
        } else {
            format!(
                "\n   The user is in {}. If the wonder lens is geography, nature, \
                 or biology, make it locally relevant.",
                state.user_location
            )
        };

        // Determine urgency — prefer morning and evening bookends
        let urgency = Self::urgency_for_hour(state.current_hour);

        // Whether this lens benefits from location awareness
        let location_aware = lens.contains(&location) || lens.contains("nature") || lens.contains("geography");

        // Build the EXECUTE prompt — this is the heart of the instinct.
        // It instructs the LLM to go on a small intellectual adventure
        // and come back with something genuinely wonderful.
                let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE You are going on a small intellectual adventure. Your mission: \
             find ONE genuinely fascinating fact and bring it back as a gift.\n\
             \n\
             CONTEXT:\n\
             - Today is {day_name}\n\
             - Current hour: {} (24h)\n\
             - Wonder lens for this session: \"{lens}\"\
             {interests_clause}\
             {location_clause}\n\
             \n\
             RESEARCH STEPS:\n\
             1. Use web_search to search for something genuinely fascinating related \
                to the wonder lens \"{lens}\" that connects to TODAY — the specific \
                date, the current season, current events, or what's happening in the \
                world right now. Search queries should be specific and curious, not \
                generic (e.g., \"why does {day_name} feel different psychology\" or \
                \"fascinating facts March 6 in history\" or \"etymology everyday \
                words surprising origins\").\n\
             2. Read through what you find. Look for the ONE fact that makes YOU \
                pause — something with a twist, a surprising number, an unexpected \
                connection, or a detail that reframes something ordinary.\n\
             3. Call browser_cleanup when done researching.\n\
             \n\
             QUALITY BAR — the fact MUST be:\n\
             (a) SURPRISING — not common knowledge, not something most people know. \
                 \"The Earth orbits the Sun\" fails. \"Earth's orbit is so elliptical \
                 that we're actually closest to the Sun in January, not summer\" passes.\n\
             (b) SPECIFIC — include real numbers, names, dates, measurements. Vague \
                 generalities (\"ancient cultures valued astronomy\") are banned. \
                 Precise details (\"the Antikythera mechanism had 37 hand-cut bronze \
                 gears and could predict eclipses to the hour\") are wonderful.\n\
             (c) CONNECTED — tie it to the user's world somehow. The fact shouldn't \
                 float in isolation; it should make them look at something familiar \
                 with new eyes.\n\
             \n\
             DELIVERY:\n\
             - 2-3 sentences maximum. Dense with wonder, not padded with filler.\n\
             - Write with a sense of genuine delight, like sharing something cool \
               you just discovered. Not a lecture, not a textbook, not a listicle.\n\
             - Don't preface with \"Did you know\" or \"Fun fact\" — just share the \
               thing directly. Let the wonder speak for itself.\n\
             - If the fact bridges the wonder lens with one of {}'s interests, even \
               better — those intersection moments are the most delightful.\n\
             \n\
             If you genuinely cannot find anything fascinating enough to clear the \
             quality bar, respond with exactly \"No wonder today.\" — it's better \
             to stay silent than to share something mediocre.",
            state.current_hour,
            state.config_user_name,
            ),
            ModelTier::Tiny => format!(
                "EXECUTE Suggest one activity for . Output: 1 sentence.",
            ),
            _ => format!(
                "EXECUTE Task: Suggest one interesting thing for .\n\
             Input: interest=.\n\
             Tool: You may use recall or web search once.\n\
             Rule: Do not invent facts. Do not repeat recent suggestions.\n\
             Fallback: \"No suggestion right now.\"\n\
             Output: 1 sentence.",
            ),
        };

        vec![UrgeSpec::new(
            self.name(),
            &execute_msg,
            urgency,
        )
        .with_cooldown("wonder_sense:fact")
        .with_context(serde_json::json!({
            "research_type": "wonder_sense",
            "lens": lens,
            "location_aware": location_aware,
        }))]
    }
}
