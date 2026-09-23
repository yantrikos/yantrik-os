//! Check-in instinct — urges companion to reach out when idle.
//!
//! Uses EXECUTE to produce contextual check-ins rather than generic greetings.

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

        // Urgency: starts at 0.3, caps at 0.5 — check-ins are low priority
        let excess_ratio = ((hours_since - self.hours_threshold) / self.hours_threshold).min(1.0);
        let urgency = 0.3 + excess_ratio * 0.2;

        let user = &state.config_user_name;
        let execute_msg = match state.model_tier {
            ModelTier::Large => format!(
                "EXECUTE Step 1: Call date_calc to get today's date, day of week, and current time.\n\
                 Step 2: Use recall with query \"working on tomorrow presentation deadline race plan\" \
                 to find what {user} was recently doing or has coming up.\n\
                 \n\
                 {user} hasn't been around for {hours_since:.0} hours. Send a brief, natural check-in (1-2 sentences) \
                 that is ANTICIPATORY — reference something coming up or in progress.\n\
                 Do NOT send a generic greeting. Reference SPECIFIC context from memory.\n\
                 If you can't recall anything specific, just say nothing.",
            ),
            _ => format!(
                "EXECUTE {user} hasn't been around for {hours_since:.0} hours. \
                 Recall what they were recently working on or have coming up. \
                 Send a brief, natural check-in (1-2 sentences) that references something specific — \
                 like how a presentation went, whether they're ready for tomorrow, or if they got a workout in. \
                 Do NOT send a generic greeting. If you can't recall anything specific, say nothing.",
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
        }))]
    }
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
    fn a_real_two_day_absence_still_checks_in() {
        let instinct = CheckInInstinct::new(2.0);
        let urges = instinct.evaluate(&state_idle_for(Some(2.0 * 86400.0)));
        assert_eq!(urges.len(), 1);
        assert_eq!(urges[0].instinct_name, "check_in");
        let hours = urges[0].context["hours_since_interaction"].as_f64().unwrap();
        assert!((hours - 48.0).abs() < 0.1, "two days is 48 hours, got {hours}");
    }
}
