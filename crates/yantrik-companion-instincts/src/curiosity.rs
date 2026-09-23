//! Curiosity instinct — idle R&D driven by user interests.
//!
//! When the system has been idle for a configurable period, this instinct
//! generates EXECUTE urges that tell the LLM to recall user preferences,
//! search the web for interesting developments, and share findings.
//!
//! Topics rotate through interest categories to avoid repetition.

use std::sync::Mutex;

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, ModelTier, UrgeSpec};

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
        // The idle gate measures the person's absence from the bond clock; unset
        // (#156) means they have never been here — there is no established idle
        // to research in, so the rate limiter below is never even reached.
        let Some(idle_secs) = state.absence_seconds() else {
            return vec![];
        };

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
        let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE Step 1: Use recall_preferences with category \"{category}\" to check what {user} is interested in.\n\
                 Step 2: Call recall with query \"{category} curiosity finding\" to check what you already \
                 shared recently. Do NOT share the same finding again.\n\
                 Step 3: Use web_search to find one recent interesting development related to those interests. \
                 If you find something genuinely noteworthy AND you haven't already shared it, \
                 share it naturally in 1-2 sentences as a proactive message. \
                 If nothing new or interesting turns up, just say so briefly. \
                 Call browser_cleanup when done.",
            ),
            _ => format!(
                "EXECUTE Do some idle research for {user} in the \"{category}\" category. \
                 Check what {user} is interested in, then recall what curiosity findings you already shared recently. \
                 Search the web for one interesting recent development related to their interests. \
                 If you find something genuinely noteworthy that you haven't shared before, \
                 present it naturally in 1-2 sentences. \
                 If nothing new turns up, just say so briefly. \
                 Clean up the browser when done.",
                category = category,
            ),
        };

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{TEST_NOW, state_at, state_idle_for};

    /// 30 minutes of idle opens the gate; 6 hours separate research sessions.
    fn instinct() -> CuriosityInstinct {
        CuriosityInstinct::new(30.0, 6.0)
    }

    #[test]
    fn an_unset_clock_never_reaches_research() {
        let instinct = instinct();
        // Two evaluations far enough apart to clear the rate limit: were the
        // decades-since-1970 idle a real absence (#156), the second would fire.
        assert!(instinct.evaluate(&state_idle_for(None)).is_empty());
        let later = state_at(TEST_NOW + 7.0 * 3600.0, None);
        assert!(
            instinct.evaluate(&later).is_empty(),
            "nobody was ever here, so there is no idle to research in"
        );
    }

    #[test]
    fn a_real_two_day_absence_still_triggers_research() {
        let instinct = instinct();
        // The first evaluation warms the rate limiter (cold-start guard)...
        assert!(instinct.evaluate(&state_idle_for(Some(2.0 * 86400.0))).is_empty());
        // ...and once the interval has passed, the absence fires it.
        let later = state_at(TEST_NOW + 7.0 * 3600.0, Some(2.0 * 86400.0));
        let urges = instinct.evaluate(&later);
        assert_eq!(urges.len(), 1);
        assert_eq!(urges[0].instinct_name, "Curiosity");
    }
}
