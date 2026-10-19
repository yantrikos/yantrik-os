//! Follow-up instinct — surfaces important memories that are fading.
//!
//! The memory store raises a `decay_review` trigger when something the person said is fading
//! (#755). The message says what it is and when it was last used (the store records the last
//! access, not when it was said), and offers the one choice
//! that matters — keep it or let it go — instead of claiming the companion "was thinking about"
//! it. Memories with a painful tone are never raised this way: pain is met when the person
//! brings it up, not resurfaced later.

use crate::Instinct;
use chrono::TimeZone;
use yantrik_companion_core::types::{CompanionState, UrgeSpec};

/// Below this valence a memory is painful enough that it is never raised proactively.
const PAINFUL_VALENCE: f64 = -0.25;

/// Longest quote of the memory in the message, in characters.
const MAX_QUOTE_CHARS: usize = 140;

pub struct FollowUpInstinct;

impl Instinct for FollowUpInstinct {
    fn name(&self) -> &str {
        "follow_up"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let decay_triggers: Vec<&serde_json::Value> = state
            .pending_triggers
            .iter()
            .filter(|t| t.get("trigger_type").and_then(|v| v.as_str()) == Some("decay_review"))
            .take(2)
            .collect();

        decay_triggers
            .iter()
            .filter_map(|trigger| {
                let ctx = trigger.get("context").cloned().unwrap_or_default();
                let text = ctx.get("text").and_then(|v| v.as_str()).map(str::trim)?;
                if text.is_empty() {
                    return None;
                }
                let valence = ctx.get("valence").and_then(|v| v.as_f64()).unwrap_or(0.0);
                if valence < PAINFUL_VALENCE {
                    return None;
                }
                let reason = trigger
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("An important memory is fading");
                let urgency = trigger
                    .get("urgency")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.4)
                    .min(0.7);
                let trigger_id = trigger
                    .get("trigger_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                // `days_since_access` counts from the memory's last access, not from when the person
                // said it, so the message claims only that: "last used on …" (evidence, not a guess).
                let last_used = ctx
                    .get("days_since_access")
                    .and_then(|v| v.as_f64())
                    .map(|days| format!(", last used {}", day_of(state.current_ts - days * 86400.0, days)))
                    .unwrap_or_default();
                let message = format!(
                    "Something you mentioned{last_used}: \u{201c}{}\u{201d} Still useful, or should I let it go?",
                    quote(text)
                );

                Some(
                    UrgeSpec::new("follow_up", &format!("Follow up: {reason}"), urgency)
                        .with_cooldown(&format!("follow_up:{trigger_id}"))
                        .with_message(&message)
                        .with_context(ctx),
                )
            })
            .collect()
    }
}

/// "on Tuesday" within the last week, "on 29 Sep" before that — a real day, never "before".
fn day_of(ts: f64, days_since: f64) -> String {
    match chrono::Local.timestamp_opt(ts as i64, 0).single() {
        Some(day) if days_since < 6.0 => format!("on {}", day.format("%A")),
        Some(day) => format!("on {}", day.format("%-d %b")),
        None => "earlier".to_string(),
    }
}

fn quote(text: &str) -> String {
    if text.chars().count() <= MAX_QUOTE_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX_QUOTE_CHARS - 1).collect();
    format!("{}\u{2026}", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::state_idle_for;

    fn decay(text: &str, valence: f64, days: f64) -> serde_json::Value {
        serde_json::json!({
            "trigger_type": "decay_review",
            "trigger_id": "t1",
            "reason": "Important memory (importance=0.8) has decayed to 0.120 after 9 days",
            "urgency": 0.5,
            "context": { "text": text, "valence": valence, "days_since_access": days },
        })
    }

    #[test]
    fn says_what_and_when_instead_of_claiming_a_thought() {
        let mut state = state_idle_for(Some(3600.0));
        state.pending_triggers = vec![decay("Renew the passport before the Lisbon trip", 0.1, 2.0)];
        let urges = FollowUpInstinct.evaluate(&state);
        assert_eq!(urges.len(), 1);
        let msg = &urges[0].suggested_message;
        assert!(msg.starts_with("Something you mentioned, last used on "), "{msg}");
        assert!(msg.contains("Renew the passport"), "{msg}");
        assert!(!msg.to_lowercase().contains("thinking"), "{msg}");
        assert!(!msg.contains("importance="), "machinery must not reach the person: {msg}");
    }

    #[test]
    fn a_painful_memory_is_never_raised_proactively() {
        let mut state = state_idle_for(Some(3600.0));
        state.pending_triggers = vec![decay("My dad is in the hospital", -0.8, 3.0)];
        assert!(FollowUpInstinct.evaluate(&state).is_empty());
    }

    #[test]
    fn no_access_age_means_no_invented_day() {
        let mut state = state_idle_for(Some(3600.0));
        let mut trigger = decay("Renew the passport", 0.1, 2.0);
        trigger["context"].as_object_mut().unwrap().remove("days_since_access");
        state.pending_triggers = vec![trigger];
        let urges = FollowUpInstinct.evaluate(&state);
        assert!(urges[0].suggested_message.starts_with("Something you mentioned: "), "{}", urges[0].suggested_message);
    }
}
