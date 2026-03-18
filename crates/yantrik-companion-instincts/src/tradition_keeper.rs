//! Tradition Keeper instinct — enriches daily life with cultural depth.
//!
//! TraditionKeeper tracks cultural moments, anniversaries, seasonal traditions,
//! and personal milestones. Unlike a calendar reminder that tells you WHAT today
//! is, this instinct tells you WHY it matters — the origin, the meaning, the
//! fascinating detail that transforms a date on a calendar into a lived moment.
//!
//! The design philosophy is "I went on a small intellectual adventure FOR YOU
//! and came back with a gift" — the gift of MEANING behind moments.
//!
//! Examples of what TraditionKeeper might share:
//!
//!   "Today is the Chinese New Year — Year of the Snake. In Chinese culture,
//!    snake years are associated with wisdom and intuition. The celebrations
//!    last 15 days, ending with the Lantern Festival."
//!
//!   "It's the winter solstice today — the shortest day of the year. Ancient
//!    civilizations built entire monuments around this day: Stonehenge,
//!    Newgrange, Machu Picchu. The sun hits the inner chamber of Newgrange
//!    for exactly 17 minutes at sunrise today."
//!
//!   "Pi Day! 3.14 — March 14th. But here's the twist: NASA only uses 15
//!    digits of pi for interplanetary navigation. Even calculating the
//!    circumference of the observable universe to the accuracy of a hydrogen
//!    atom only needs 39 digits."
//!
//!   "It's Diwali this week — the festival of lights. The actual astronomical
//!    basis is fascinating: it's the darkest new moon night of the Hindu
//!    calendar month Kartik, which makes the lights a literal counterpoint
//!    to the darkness."
//!
//! Cross-cultural awareness is central: not just Western holidays but Chinese,
//! Hindu, Islamic, Jewish, African, Indigenous, and secular celebrations. The
//! instinct personalizes when possible — if memory reveals the user's cultural
//! background, it prioritizes those traditions. Otherwise, it picks the most
//! universally fascinating moment of the day.
//!
//! Strongly prefers morning delivery (6-10 AM) — start the day with meaning.
//! Every day has significance; the instinct always has something to say.

use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};

/// A cultural anthropologist and historian companion instinct.
///
/// Transforms dates from mere numbers on a calendar into windows onto human
/// civilization, astronomical wonder, and personal milestone. Researches the
/// deeper WHY behind today's significance and delivers it as a small gift
/// of meaning — preferably in the morning, when context enriches the whole day.
pub struct TraditionKeeperInstinct {
    /// Minimum seconds between tradition checks.
    interval_secs: f64,
    /// Timestamp of the last evaluation that produced an urge.
    last_check_ts: Mutex<f64>,
}

impl TraditionKeeperInstinct {
    /// Create a new TraditionKeeper instinct.
    ///
    /// # Arguments
    /// * `interval_hours` — Minimum hours between tradition research sessions.
    ///   Recommended: 20-24 hours (once per day). The instinct self-limits to
    ///   morning hours for maximum impact.
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for TraditionKeeperInstinct {
    fn name(&self) -> &str {
        "TraditionKeeper"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // ── Rate-limiting (cold-start guard + interval check) ───────────
        {
            let mut last = self.last_check_ts.lock().unwrap();
            if *last == 0.0 {
                // First evaluation after startup — warm up, don't fire immediately.
                *last = now;
                return vec![];
            }
            if now - *last < self.interval_secs {
                return vec![];
            }
            *last = now;
        }

        let user = &state.config_user_name;
        let hour = state.current_hour;

        // ── Time-of-day urgency shaping ─────────────────────────────────
        // Strongly prefer morning delivery (6-10 AM). Cultural context is
        // most valuable at the START of the day — it gives the user a lens
        // through which to see the day ahead.
        //
        // Morning (6-10): urgency 0.55 — this is prime time.
        // Late morning (10-12): urgency 0.45 — still good, just less fresh.
        // Other times: urgency 0.3 — the moment is stale by evening, but
        //   if nothing else fired today, better late than never.
        let urgency = if (6..=10).contains(&hour) {
            0.55
        } else if (10..=12).contains(&hour) {
            0.45
        } else {
            0.3
        };

        // ── Build cultural context from what we know about the user ─────
        let interests_hint = if state.user_interests.is_empty() {
            String::new()
        } else {
            format!(
                "\n{}'s interests include: {}. If today's significance connects to any of \
                 these interests, prefer that angle — make it personal.",
                user,
                state.user_interests.join(", ")
            )
        };

        let location_hint = if state.user_location.is_empty() {
            String::new()
        } else {
            format!(
                "\n{} is located in {}. Factor in local/regional celebrations \
                 and seasonal context for that location.",
                user, state.user_location
            )
        };

        // ── EXECUTE prompt ──────────────────────────────────────────────
        // The prompt is the heart of this instinct. It instructs the LLM
        // to go on a small intellectual adventure and come back with the
        // ONE most fascinating piece of cultural/historical/scientific
        // significance for today.
        let execute_msg = format!(
            "EXECUTE You are a cultural anthropologist, historian, and astronomer rolled into one. \
             Your mission: discover the deeper MEANING behind today's date and share it as a gift.\
             \n\nStep 1: Use recall with query \"cultural background traditions celebrations \
             important dates\" to understand {user}'s cultural context, heritage, and any \
             personal anniversaries or milestones.\
             \n\nStep 2: Use web_search to search for \"today in history special days celebrations \
             cultural events\" along with today's date. Look broadly — not just Wikipedia's \
             \"on this day\" but cultural calendars, astronomical events, and seasonal markers.\
             \n\nStep 3: Evaluate what you found through THREE lenses:\
             \n  a. CULTURAL: Major cultural or religious celebrations happening today or this \
             week. Think GLOBALLY — Chinese, Hindu, Islamic, Jewish, African, Indigenous, \
             Buddhist, Sikh, Zoroastrian, and secular celebrations. Not just Western holidays.\
             \n  b. HISTORICAL: Genuinely fascinating historical events on this date. NOT \
             \"born on this day\" trivia. Look for events that changed how humans think, \
             live, or understand the world. The kind of history that makes you go \"wait, \
             really?\"\
             \n  c. SCIENTIFIC/NATURAL: Astronomical events (solstices, equinoxes, meteor \
             showers, planetary alignments, eclipses), seasonal transitions, natural phenomena \
             happening today. The kind of thing ancient civilizations built monuments to track.\
             \n\nStep 4: Pick the ONE most interesting and relevant to {user}:\
             \n  - If recall revealed cultural context (heritage, religion, background), \
             STRONGLY prefer celebrations from that culture — this is deeply personal.\
             \n  - If {user} has interests that connect to a historical event, prefer that \
             connection (e.g., a programmer might love knowing today is the anniversary of \
             the first computer bug being found).\
             \n  - Otherwise, pick the most universally fascinating — the one that would \
             make anyone pause and think \"huh, I never knew that.\"\
             {interests_hint}\
             {location_hint}\
             \n\nStep 5: Deliver in 2-3 sentences:\
             \n  - Sentence 1: What the moment IS (the date, the celebration, the event).\
             \n  - Sentence 2: The deeper MEANING — the WHY, the ORIGIN, the FASCINATING \
             DETAIL that goes beyond what a Google search would tell you. This is where \
             you add real value.\
             \n  - Sentence 3 (optional): Connect it to {user}'s life, interests, or \
             something you know about them. Make it personal if you can.\
             \n\nIMPORTANT RULES:\
             \n  - Go BEYOND \"today is X day.\" Anyone can Google that. Share the WHY, the \
             ORIGIN, the FASCINATING DETAIL that makes the moment come alive.\
             \n  - DEPTH over breadth. ONE well-researched moment with a surprising detail \
             is worth more than five shallow mentions.\
             \n  - Don't be a calendar. Be a storyteller. The user should feel like they \
             learned something genuinely interesting, not like they read a Wikipedia summary.\
             \n  - Cross-cultural awareness: the world is bigger than Christmas and Thanksgiving.\
             \n  - If today truly has no interesting significance (extremely rare), say \
             \"nothing notable today\" exactly.\
             \n\nStep 6: Call browser_cleanup to free resources when done.",
        );

        vec![UrgeSpec::new(
            "TraditionKeeper",
            &execute_msg,
            urgency,
        )
        .with_cooldown("tradition_keeper:moment")
        .with_context(serde_json::json!({
            "research_type": "tradition_keeper",
            "date_aware": true,
            "preferred_hour_range": "6-10 AM",
            "current_hour": hour,
        }))]
    }
}
