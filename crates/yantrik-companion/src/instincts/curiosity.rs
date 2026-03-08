//! Curiosity instinct — idle R&D driven by user interests.
//!
//! When the system has been idle for a configurable period, this instinct
//! generates EXECUTE urges that tell the LLM to recall user preferences,
//! search the web for interesting developments, and share findings.
//!
//! Topics rotate through interest categories to avoid repetition.

use std::sync::Mutex;

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

/// Interest categories to cycle through when picking research topics.
const CATEGORIES: &[&str] = &[
    "hobby",
    "work",
    "general",
    "food",
    "travel",
    "health",
    "shopping",
];

pub struct CuriosityInstinct {
    /// Minimum idle time (seconds) before research triggers.
    idle_threshold_secs: f64,
    /// Minimum seconds between research sessions.
    interval_secs: f64,
    /// Last research timestamp.
    last_research_ts: Mutex<f64>,
    /// Rotating category index.
    category_index: Mutex<usize>,
}

impl CuriosityInstinct {
    pub fn new(idle_threshold_minutes: f64, interval_hours: f64) -> Self {
        Self {
            idle_threshold_secs: idle_threshold_minutes * 60.0,
            interval_secs: interval_hours * 3600.0,
            last_research_ts: Mutex::new(0.0),
            category_index: Mutex::new(0),
        }
    }
}

impl Instinct for CuriosityInstinct {
    fn name(&self) -> &str {
        "Curiosity"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;
        let idle_secs = now - state.last_interaction_ts;

        // Only fire when sufficiently idle
        if idle_secs < self.idle_threshold_secs {
            return vec![];
        }

        // Rate-limit (cold-start guard: skip first eval after startup)
        {
            let mut last = self.last_research_ts.lock().unwrap();
            if *last == 0.0 {
                *last = now; // warm up — don't fire on first cycle
                return vec![];
            }
            if now - *last < self.interval_secs {
                return vec![];
            }
            *last = now;
        }

        // Pick next category
        let category = {
            let mut idx = self.category_index.lock().unwrap();
            let cat = CATEGORIES[*idx % CATEGORIES.len()];
            *idx = idx.wrapping_add(1);
            cat
        };

        let user = &state.config_user_name;

        // The EXECUTE prefix triggers handle_message_streaming with tool access
        let execute_msg = format!(
            "EXECUTE STEP 1: Call date_calc to get today's date and current time.\n\
             STEP 2: Use recall_preferences with category \"{}\" to check what {} is interested in.\n\
             STEP 3: Call recall with query \"{} curiosity finding\" to check what you already \
             shared recently. Do NOT share the same finding again.\n\
             STEP 4: Use web_search to find one recent interesting development related to those interests. \
             If you find something genuinely noteworthy AND you haven't already shared it, \
             share it naturally in 1-2 sentences as a proactive message. \
             If nothing new or interesting turns up, just say so briefly. \
             After you're done, call browser_cleanup to free resources.",
            category, user, category,
        );

        vec![UrgeSpec::new(
            "Curiosity",
            &execute_msg,
            0.3, // Low urgency — background task, never interrupt
        )
        .with_cooldown(&format!("curiosity:{}", category))
        .with_context(serde_json::json!({
            "category": category,
            "idle_seconds": idle_secs,
            "research_type": "interest_based",
        }))]
    }
}
