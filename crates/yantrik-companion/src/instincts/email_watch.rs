//! Email Watch instinct — background email monitor.
//!
//! Periodically generates EXECUTE urges that tell the LLM to check for
//! new emails and alert about important ones.

use std::sync::Mutex;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct EmailWatchInstinct {
    interval_secs: f64,
    last_check_ts: Mutex<f64>,
}

impl EmailWatchInstinct {
    pub fn new(interval_minutes: f64) -> Self {
        Self {
            interval_secs: interval_minutes * 60.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for EmailWatchInstinct {
    fn name(&self) -> &str {
        "EmailWatch"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let now = state.current_ts;
        {
            let last = self.last_check_ts.lock().unwrap();
            if now - *last < self.interval_secs {
                return vec![];
            }
        }
        {
            let mut last = self.last_check_ts.lock().unwrap();
            *last = now;
        }

        let user = &state.config_user_name;
        let execute_msg = format!(
            "EXECUTE Use email_check to fetch new emails. If any look urgent or important \
             (from a known contact, contains deadline language, flagged by sender as important), \
             summarize the most important one in 1-2 sentences as a notification to {}. \
             If no new emails or nothing important, respond with just \"No new important emails.\"",
            user,
        );

        vec![UrgeSpec::new(
            "EmailWatch",
            &execute_msg,
            0.6,
        )
        .with_cooldown("email_watch:check")
        .with_context(serde_json::json!({
            "check_type": "email_monitor",
        }))]
    }
}
