//! News Watch instinct — breaking news monitor.
//!
//! Periodically generates EXECUTE urges that tell the LLM to search for
//! breaking news and alert the user about genuinely significant events.
//! Uses web_search tool for fetching, not RSS.

use std::sync::Mutex;

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct NewsWatchInstinct {
    /// Seconds between news checks.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl NewsWatchInstinct {
    pub fn new(interval_minutes: f64) -> Self {
        Self {
            interval_secs: interval_minutes * 60.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for NewsWatchInstinct {
    fn name(&self) -> &str {
        "NewsWatch"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;

        // Rate-limit (cold-start guard: skip first eval after startup)
        {
            let mut last = self.last_check_ts.lock().unwrap();
            if *last == 0.0 {
                *last = now; // warm up — don't fire on first cycle
                return vec![];
            }
            if now - *last < self.interval_secs {
                return vec![];
            }
            *last = now;
        }

        let user = &state.config_user_name;

        // The EXECUTE prefix triggers handle_message_streaming with tool access
        let execute_msg = format!(
            "EXECUTE Use web_search to search for \"major breaking news today\". \
             Only report events that are truly significant — natural disasters, \
             wars, major political events, elections, major tech announcements, \
             or events relevant to {}. If something major happened, write a brief \
             1-2 sentence alert. If nothing significant, respond with just \"No major news.\"",
            user,
        );

        vec![UrgeSpec::new(
            "NewsWatch",
            &execute_msg,
            0.7, // High urgency — news can interrupt
        )
        .with_cooldown("news_watch:check")
        .with_context(serde_json::json!({
            "check_type": "breaking_news",
        }))]
    }
}
