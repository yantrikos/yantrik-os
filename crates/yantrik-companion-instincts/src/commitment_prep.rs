//! Commitment prep — when the prep window opens on something the person promised, and the promise
//! names files, ask for it to be prepared: files opened in Mind View, the email drafted, nothing sent.
//!
//! The instinct only decides *whether*. It reads the `commitment_approaching` triggers the open-loops
//! monitor's scan produces (a commitment inside its prep window, with its promisee, deadline and the
//! number of files it names) and raises one EXECUTE urge per commitment. What *doing* it means is
//! `yantrik_companion::commitment_prep`, which the urge's context names ([`EXECUTOR`]): it is not a
//! prompt for the model, so it runs with the model offline and the same way every time.

use crate::Instinct;
use yantrik_companion_core::types::{CompanionState, InstinctCategory, TimeSensitivity, UrgeSpec};

/// The `executor` the urge's context names. The shell hands an urge that carries it to
/// `commitment_prep` instead of to the model.
pub const EXECUTOR: &str = "commitment_prep";

/// The trigger the open-loops scan raises for a commitment inside its prep window.
const TRIGGER: &str = "commitment_approaching";

pub struct CommitmentPrepInstinct;

impl Instinct for CommitmentPrepInstinct {
    fn name(&self) -> &str {
        "commitment_prep"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        state
            .pending_triggers
            .iter()
            .filter(|t| t.get("trigger_type").and_then(|v| v.as_str()) == Some(TRIGGER))
            .filter(|t| t.get("file_count").and_then(|v| v.as_u64()).unwrap_or(0) > 0)
            .filter_map(|t| {
                let id = t.get("commitment_id").and_then(|v| v.as_i64())?;
                let action = t.get("action").and_then(|v| v.as_str()).unwrap_or("what you promised");
                // `EXECUTE` is how the shell recognises an urge that is to be carried out rather
                // than said; the instruction is read by `commitment_prep`, not by a model.
                let reason = format!("EXECUTE Prepare what you promised: {action}");
                Some(
                    UrgeSpec::new("commitment_prep", &reason, 0.8)
                        .with_cooldown(&format!("commitment_prep:{id}"))
                        .with_message(&reason)
                        .with_time_sensitivity(TimeSensitivity::Soon)
                        .with_category(InstinctCategory::Anticipatory)
                        .with_context(serde_json::json!({ "executor": EXECUTOR, "commitment_id": id })),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{state_at, TEST_NOW};
    use serde_json::json;

    fn state_with(triggers: Vec<serde_json::Value>) -> CompanionState {
        let mut state = state_at(TEST_NOW, Some(60.0));
        state.pending_triggers = triggers;
        state
    }

    #[test]
    fn nothing_promised_is_nothing_raised() {
        assert!(CommitmentPrepInstinct.evaluate(&state_with(vec![])).is_empty());
    }

    #[test]
    fn a_promise_in_its_window_that_names_files_raises_one_urge_for_the_executor() {
        let urges = CommitmentPrepInstinct.evaluate(&state_with(vec![json!({
            "trigger_type": "commitment_approaching", "commitment_id": 7, "action": "sending Maya the Q4 deck",
            "file_count": 2,
        })]));
        assert_eq!(urges.len(), 1);
        assert!(urges[0].reason.starts_with("EXECUTE "));
        assert_eq!(urges[0].cooldown_key, "commitment_prep:7");
        assert_eq!(urges[0].context["executor"], EXECUTOR);
        assert_eq!(urges[0].context["commitment_id"], 7);
    }

    #[test]
    fn a_promise_that_names_no_file_has_nothing_to_prepare() {
        let urges = CommitmentPrepInstinct.evaluate(&state_with(vec![json!({
            "trigger_type": "commitment_approaching", "commitment_id": 8, "action": "call Sam", "file_count": 0,
        })]));
        assert!(urges.is_empty());
    }

    #[test]
    fn an_overdue_or_other_trigger_is_not_a_prep_window() {
        let urges = CommitmentPrepInstinct.evaluate(&state_with(vec![
            json!({ "trigger_type": "commitment_overdue", "commitment_id": 9, "file_count": 1 }),
            json!({ "trigger_type": "automation", "commitment_id": 10, "file_count": 1 }),
        ]));
        assert!(urges.is_empty());
    }
}
