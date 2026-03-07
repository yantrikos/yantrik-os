//! TimeCapture instinct — "This moment matters, let's capture it before it fades."
//!
//! Prompts micro-journaling at emotionally significant moments. When the user
//! sounds excited, accomplished, or deeply moved, offers to capture the feeling
//! for their future self. Uses `memorize` with tag "time_capsule" to preserve
//! emotional snapshots that gain meaning over time.
//!
//! This is the instinct that says:
//!   "You sound genuinely excited about this. Want to capture how you feel
//!    right now? Future-you will appreciate reading this."

use std::sync::Mutex;

use crate::bond::BondLevel;
use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct TimeCaptureInstinct {
    /// Seconds between time capture checks.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl TimeCaptureInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for TimeCaptureInstinct {
    fn name(&self) -> &str {
        "TimeCapture"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Gate: bond >= Friend
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        // Gate: session must be active
        if !state.session_active {
            return vec![];
        }

        // Gate: conversation must have some depth (turn_count > 3)
        if state.conversation_turn_count <= 3 {
            return vec![];
        }

        // Rate-limit (cold-start guard + 12h interval)
        let now = state.current_ts;
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

        let user = &state.config_user_name;

        let execute_msg = format!(
            "EXECUTE Use recall with query 'excited happy proud accomplished moved grateful \
             amazing beautiful' to find emotionally significant recent moments from {user}.\n\
             \n\
             Look for moments where {user} expressed genuine emotion — excitement about a \
             breakthrough, pride in something they built, gratitude for someone, wonder at \
             something beautiful, or deep satisfaction.\n\
             \n\
             If you find an emotionally charged moment from the recent conversation or memories:\n\
             - Compose a gentle, invitational prompt like: \"You sound genuinely excited about \
               this. Want to capture how you feel right now? Future-you will appreciate reading \
               this.\"\n\
             - Match the specific emotion — don't say 'excited' if they're 'grateful'.\n\
             - If the user accepts, use memorize to save their reflection with tag 'time_capsule'.\n\
             - Keep it warm and brief — one or two sentences max.\n\
             \n\
             If no emotional peaks found in recent context, respond with just \
             \"No time capture today.\""
        );

        vec![UrgeSpec::new("TimeCapture", &execute_msg, 0.35)
            .with_cooldown("time_capture:moment")
            .with_context(serde_json::json!({
                "instinct_type": "time_capture",
                "conversation_depth": state.conversation_turn_count,
            }))]
    }
}
