//! Check-in instinct — a return summary when there is something concrete to report.
//!
//! Idleness alone is never a reason to speak (#755): a message whose only reason is that the
//! person has been away exists to bring them back. The check-in fires after an absence only when
//! the record holds a concrete reason — an overdue commitment, a message waiting for a reply, or
//! something that happened and hasn't been mentioned — and it carries that reason with it. It
//! never mentions how long the person was gone.

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, ModelTier, UrgeSpec};

pub struct CheckInInstinct {
    hours_threshold: f64,
}

impl CheckInInstinct {
    pub fn new(hours_threshold: f64) -> Self {
        Self { hours_threshold }
    }
}

impl Instinct for CheckInInstinct {
    fn name(&self) -> &str {
        "check_in"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // The absence is measured from the bond clock; unset (#156) means the
        // companion has never heard the person. You cannot be away from someone
        // you have never met, so there is nobody to check in on — silent until
        // the first scored turn gives the clock a real value.
        let Some(absent_secs) = state.absence_seconds() else {
            return vec![];
        };
        let hours_since = absent_secs / 3600.0;

        if hours_since < self.hours_threshold {
            return vec![];
        }

        // No concrete reason from the record, no check-in.
        let Some(reason) = concrete_reason(state) else {
            return vec![];
        };

        // Urgency: starts at 0.3, caps at 0.5 — check-ins are low priority
        let excess_ratio = ((hours_since - self.hours_threshold) / self.hours_threshold).min(1.0);
        let urgency = 0.3 + excess_ratio * 0.2;

        let user = &state.config_user_name;
        let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE Step 1: Call date_calc to get today's date, day of week, and current time.\n\
                 Step 2: Use recall with query \"{reason}\" for the details.\n\
                 \n\
                 The reason to speak, from the record: {reason}.\n\
                 Tell {user} plainly what is waiting or what changed, in 1-2 sentences, and offer one \
                 concrete next step. Do NOT greet without content. Do NOT mention how long {user} was away. \
                 Do NOT say you missed them or wondered about them.\n\
                 If the reason no longer holds, reply exactly \"Nothing to surface right now.\"",
            ),
            _ => format!(
                "EXECUTE Task: Tell {user} what is waiting for them.\n\
                 Input: reason={reason}.\n\
                 Rule: Say only what the reason states. Do not greet without content. Do not mention how long they were away.\n\
                 Fallback: \"Nothing to surface right now.\"\n\
                 Output: 1-2 sentences.",
            ),
        };

        vec![UrgeSpec::new(
            "check_in",
            &execute_msg,
            urgency,
        )
        .with_cooldown("check_in")
        .with_context(serde_json::json!({
            "hours_since_interaction": (hours_since * 10.0).round() / 10.0,
            "reason": reason,
        }))]
    }
}

/// A concrete reason, from the record, to say something after an absence — or `None`.
fn concrete_reason(state: &CompanionState) -> Option<String> {
    if state.overdue_commitment_count > 0 {
        let n = state.overdue_commitment_count;
        return Some(format!("{n} overdue commitment{}", if n == 1 { "" } else { "s" }));
    }
    if state.pending_attention_count > 0 {
        let n = state.pending_attention_count;
        return Some(format!(
            "{n} message{} waiting for a reply",
            if n == 1 { "" } else { "s" }
        ));
    }
    state
        .recent_events
        .iter()
        .rev()
        .find(|(_, _, reflected)| !reflected)
        .map(|(desc, _, _)| format!("recorded while away: {desc}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::state_idle_for;

    #[test]
    fn a_person_never_met_is_not_announced_as_long_absent() {
        let instinct = CheckInInstinct::new(2.0);
        // The clock is unset (#156): no interaction was ever scored, so there is
        // no absence. Subtracting it from "now" anyway used to report roughly
        // half a million hours away and check in on someone who was never here.
        assert!(instinct.evaluate(&state_idle_for(None)).is_empty());
    }

    #[test]
    fn idleness_alone_is_not_a_reason_to_speak() {
        let instinct = CheckInInstinct::new(2.0);
        // Two days away with nothing in the record: a message now would exist only to bring the
        // person back (#755).
        assert!(instinct.evaluate(&state_idle_for(Some(2.0 * 86400.0))).is_empty());
    }

    #[test]
    fn a_real_absence_with_something_waiting_checks_in_and_says_why() {
        let instinct = CheckInInstinct::new(2.0);
        let mut state = state_idle_for(Some(2.0 * 86400.0));
        state.overdue_commitment_count = 1;
        let urges = instinct.evaluate(&state);
        assert_eq!(urges.len(), 1);
        assert_eq!(urges[0].instinct_name, "check_in");
        assert_eq!(urges[0].context["reason"], "1 overdue commitment");
        let hours = urges[0].context["hours_since_interaction"].as_f64().unwrap();
        assert!((hours - 48.0).abs() < 0.1, "two days is 48 hours, got {hours}");
        assert!(!urges[0].reason.contains("hasn't been around"), "{}", urges[0].reason);
    }
}
