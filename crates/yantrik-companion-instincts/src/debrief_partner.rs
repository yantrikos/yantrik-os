//! DebriefPartner instinct — "What happened, what surprised you, what would you do differently?"
//!
//! After significant events (meetings, presentations, difficult conversations,
//! milestones), offers a structured debrief. Checks `recent_events` for entries
//! that haven't been reflected on yet and prompts the user with three focused
//! questions. Stores insights via `memorize` with tag "debrief" for future
//! reference before similar events.
//!
//! This is the instinct that says:
//!   "You just went through that product demo. Three quick questions:
//!    What went well? What surprised you? What would you do differently?"

use std::sync::Mutex;

use yantrik_companion_core::bond::BondLevel;
use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};

/// Maximum age of an unreflected event to trigger a debrief (6 hours).
const EVENT_FRESHNESS_SECS: f64 = 6.0 * 3600.0;

pub struct DebriefPartnerInstinct {
    /// Seconds between debrief checks.
    interval_secs: f64,
    /// Last check timestamp.
    last_check_ts: Mutex<f64>,
}

impl DebriefPartnerInstinct {
    pub fn new(interval_hours: f64) -> Self {
        Self {
            interval_secs: interval_hours * 3600.0,
            last_check_ts: Mutex::new(0.0),
        }
    }
}

impl Instinct for DebriefPartnerInstinct {
    fn name(&self) -> &str {
        "DebriefPartner"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Gate: bond >= Friend
        if state.bond_level < BondLevel::Friend {
            return vec![];
        }

        // Gate: must have recent events to debrief
        if state.recent_events.is_empty() {
            return vec![];
        }

        // Rate-limit (cold-start guard + 6h interval)
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

        // Find unreflected events within the freshness window
        let unreflected: Vec<&(String, f64, bool)> = state
            .recent_events
            .iter()
            .filter(|(_, ts, reflected)| !reflected && (now - ts) < EVENT_FRESHNESS_SECS)
            .collect();

        if unreflected.is_empty() {
            return vec![];
        }

        // Pick the most recent unreflected event
        let (event_desc, _event_ts, _) = unreflected
            .iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap();

        let user = &state.config_user_name;

        let execute_msg = format!(
            "EXECUTE Use recall with query '{event}' to gather context about this recent event \
             that {user} went through.\n\
             \n\
             {user} recently experienced: \"{event}\"\n\
             \n\
             Compose a gentle, structured debrief prompt:\n\
             - Acknowledge the event warmly: \"You just went through [event].\"\n\
             - Offer three focused questions: \"Three quick questions: What went well? \
               What surprised you? What would you do differently?\"\n\
             - Keep the tone warm but structured — this is a thinking tool, not therapy.\n\
             - Make the questions specific to the event type (a meeting debrief differs \
               from a presentation debrief).\n\
             - Keep it to 2-3 sentences total.\n\
             \n\
             If the user engages with the debrief, use memorize to store their insights \
             with tag 'debrief' so they can reference them before similar future events.\n\
             \n\
             If the event context is too vague to debrief meaningfully, respond with just \
             \"No debrief needed.\"",
            event = event_desc,
        );

        vec![UrgeSpec::new("DebriefPartner", &execute_msg, 0.45)
            .with_cooldown("debrief_partner:session")
            .with_context(serde_json::json!({
                "instinct_type": "debrief_partner",
                "event": event_desc,
                "unreflected_count": unreflected.len(),
            }))]
    }
}
