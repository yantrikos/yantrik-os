//! Local Pulse instinct — hyperlocal intelligence for the user's neighborhood.
//!
//! LocalPulse keeps a finger on the pulse of the user's local area: new openings,
//! road closures, community events, seasonal happenings, farmers market updates,
//! and neighborhood changes. It's the "did you hear about..." instinct for your city.
//!
//! The design principle is "I went on a small intellectual adventure FOR YOU and came
//! back with a gift" — delivering hyperlocal intelligence that makes the user feel
//! genuinely connected to their neighborhood.
//!
//! Each evaluation rotates through a set of "local lenses" — search angles that cover
//! different facets of local life. This prevents the instinct from always checking the
//! same thing and ensures broad coverage across restaurants, events, traffic, outdoor
//! activities, deals, and seasonal happenings.
//!
//! # Examples
//!
//! - "That new ramen place on South Congress finally opened — the owner trained at
//!   Ippudo in Tokyo for 8 years. Early reviews mention a 45-minute wait but say the
//!   tonkotsu is the best in Austin."
//!
//! - "Heads up: they're closing Lamar Blvd between 5th and 7th for construction
//!   starting Monday. Your usual route to work will be affected for about 3 weeks."
//!
//! - "The Zilker Park trail of lights opens next Friday. Pro tip from last year's data:
//!   Tuesday nights have 60% shorter lines than weekends."
//!
//! - "Austin's farmers market at Mueller just added a new vendor — a family from Oaxaca
//!   making fresh mole paste from scratch. That's rare outside Mexico."
//!
//! # Location Requirement
//!
//! This instinct is completely useless without a user location. If
//! `state.user_location` is empty, `evaluate()` returns no urges.
//!
//! # Timing
//!
//! Mornings (7-10 AM) are preferred for day-planning relevance. Thursday through
//! Saturday get a slight urgency boost because people plan weekend activities during
//! those days.

use std::sync::Mutex;

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

/// Search angles that rotate each evaluation, covering different facets of local life.
///
/// The `{location}` placeholder is substituted with the user's actual location at
/// prompt-generation time. Each lens targets a distinct category of hyperlocal
/// intelligence to ensure broad, non-repetitive coverage.
const LOCAL_LENSES: &[&str] = &[
    "new restaurant opening {location} this week",
    "local events this weekend {location}",
    "road construction traffic changes {location}",
    "new business opening {location}",
    "community events festivals {location} this month",
    "local deals specials {location} today",
    "farmers market seasonal {location}",
    "outdoor activities events {location} weather",
];

/// Hyperlocal intelligence instinct — surfaces neighborhood happenings, new openings,
/// road changes, events, and seasonal phenomena for the user's city.
///
/// Acts like a well-connected local friend who always knows what's going on and shares
/// only the genuinely interesting, current, and actionable findings.
pub struct LocalPulseInstinct {
    /// Seconds between local pulse checks.
    interval_secs: f64,
    /// Timestamp of the last evaluation (cold-start guard: starts at 0.0).
    last_check_ts: Mutex<f64>,
    /// Index into `LOCAL_LENSES` — incremented each evaluation for rotation.
    lens_index: Mutex<usize>,
}

impl LocalPulseInstinct {
    /// Create a new LocalPulse instinct with the given check interval.
    ///
    /// # Arguments
    ///
    /// * `interval_hours` — Minimum hours between pulse checks. Recommended: 4-8 hours
    ///   to avoid overwhelming the user with local updates.
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
            lens_index: Mutex::new(0),
        }
    }
}

impl Instinct for LocalPulseInstinct {
    fn name(&self) -> &str {
        "LocalPulse"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // ── Rate-limit with cold-start guard ──────────────────────────────
        // On the very first evaluation after startup we record the timestamp
        // and return empty so we don't fire immediately on boot.
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

        // ── Location gate ─────────────────────────────────────────────────
        // This instinct is entirely useless without a location. A generic
        // "local events near you" with no city is just noise.
        if state.user_location.is_empty() {
            return vec![];
        }

        let location = &state.user_location;
        let user = &state.config_user_name;

        // ── Rotate through local lenses ───────────────────────────────────
        let current_lens = {
            let mut idx = self.lens_index.lock().unwrap();
            let lens = LOCAL_LENSES[*idx % LOCAL_LENSES.len()];
            *idx = (*idx + 1) % LOCAL_LENSES.len();
            lens.replace("{location}", location)
        };

        // ── Build interest context for cross-referencing ──────────────────
        let interests_hint = if state.user_interests.is_empty() {
            String::new()
        } else {
            format!(
                "\n{user}'s interests include: {}. If any local finding connects to these \
                 interests, prioritize it and explain the connection.",
                state.user_interests.join(", ")
            )
        };

        // ── EXECUTE prompt ────────────────────────────────────────────────
        let execute_msg = format!(
            "EXECUTE You are {user}'s hyperlocal intelligence — a well-connected friend \
             who always knows what's happening in {location}.\
             \n\n\
             Step 1: Use recall with query \"local favorites restaurants places {location}\" \
             to understand {user}'s local preferences, frequented spots, and neighborhood habits.\
             {interests_hint}\
             \n\n\
             Step 2: Use web_search with query \"{current_lens}\" to find CURRENT local happenings. \
             Focus on results from the last 7 days — stale news is not local pulse.\
             \n\n\
             Step 3: Filter results through three quality gates:\
             \n  - RECENCY: Must be current or upcoming (this week / this weekend / opening soon). \
             Anything older than a week is stale.\
             \n  - RELEVANCE: Connected to {user}'s interests if possible, or universally useful \
             (road closures, major events, weather-affected activities).\
             \n  - ACTIONABILITY: Something {user} can actually do, visit, avoid, or plan around. \
             Pure FYI without action is low value.\
             \n\n\
             Step 4: Deliver ONE genuinely interesting local finding in 2-3 sentences:\
             \n  - WHAT it is and WHERE (be specific — street names, neighborhoods, venues)\
             \n  - WHY it's noteworthy (not just \"there's an event\" — what makes it special?)\
             \n  - A SPECIFIC DETAIL that shows genuine research: opening hours, insider tips, \
             expected wait times, parking advice, the story behind it, comparisons to alternatives\
             \n\n\
             If {user} has known interests, look for the intersection between local happenings \
             and those interests. A fishing enthusiast hearing about a new bait shop is more \
             valuable than a generic festival announcement.\
             \n\n\
             After research, call browser_cleanup to free resources.\
             \n\n\
             If nothing genuinely interesting, current, or actionable was found for {location}, \
             respond with just \"No local pulse today.\" — do NOT fabricate or stretch a weak finding."
        );

        // ── Urgency calculation ───────────────────────────────────────────
        // Morning hours (7-10 AM) are best for day-planning local intel.
        // Thursday (4), Friday (5), Saturday (6) get a boost because people
        // actively plan weekend activities during those days.
        let is_morning = (7..=10).contains(&state.current_hour);
        let is_weekend_planning_day = matches!(state.current_day_of_week, 4 | 5 | 6);

        let urgency = if is_morning || is_weekend_planning_day {
            0.5
        } else {
            0.35
        };

        vec![UrgeSpec::new("LocalPulse", &execute_msg, urgency)
            .with_cooldown("local_pulse:update")
            .with_context(serde_json::json!({
                "research_type": "local_pulse",
                "location": location,
                "lens": current_lens,
            }))]
    }
}
