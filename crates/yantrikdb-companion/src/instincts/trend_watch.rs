//! Trend Watch instinct — proactive trending topic discovery.
//!
//! Periodically searches Google, X (Twitter), and Reddit via browser tools
//! to find interesting developing stories and trending topics.
//! Reports the most compelling finding to the user.
//!
//! IMPORTANT: Always calls browser_cleanup after finishing to free resources.

use std::sync::Mutex;

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

/// Sources to rotate through for trend discovery.
const SOURCES: &[(&str, &str)] = &[
    ("google_trends", "Google Trends"),
    ("reddit", "Reddit front page"),
    ("x_twitter", "X / Twitter trending"),
];

pub struct TrendWatchInstinct {
    /// Seconds between trend checks.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
    /// Rotating source index.
    source_index: Mutex<usize>,
}

impl TrendWatchInstinct {
    pub fn new(interval_minutes: f64) -> Self {
        Self {
            interval_secs: interval_minutes * 60.0,
            last_check_ts: Mutex::new(0.0),
            source_index: Mutex::new(0),
        }
    }
}

impl Instinct for TrendWatchInstinct {
    fn name(&self) -> &str {
        "TrendWatch"
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

        // Pick next source
        let (source_id, source_name) = {
            let mut idx = self.source_index.lock().unwrap();
            let src = SOURCES[*idx % SOURCES.len()];
            *idx = idx.wrapping_add(1);
            src
        };

        let user = &state.config_user_name;

        let search_query = match source_id {
            "google_trends" => "trending topics today",
            "reddit" => "site:reddit.com popular today",
            "x_twitter" => "trending on twitter/X today",
            _ => "trending topics today",
        };

        let execute_msg = format!(
            "EXECUTE Use web_search to search for \"{search_query}\" to see what's trending on {source_name}. \
             Then use browser_read on the most promising result to get details. \
             Analyze what you find and pick the ONE most interesting or developing story — \
             something {user} would actually want to know about. \
             Present it in 2-3 sentences: what's happening, why it matters, and any emerging angle. \
             If nothing is genuinely interesting, just say so briefly. \
             IMPORTANT: After you're done, call browser_cleanup to close the browser and free resources.",
        );

        vec![UrgeSpec::new(
            "TrendWatch",
            &execute_msg,
            0.6, // Medium-high urgency — interesting but not critical
        )
        .with_cooldown(&format!("trend_watch:{}", source_id))
        .with_context(serde_json::json!({
            "source": source_id,
            "source_name": source_name,
            "check_type": "trend_discovery",
        }))]
    }
}
