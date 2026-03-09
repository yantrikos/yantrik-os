//! Open Loops Guardian — surfaces unresolved commitments, unanswered messages,
//! and stalled life threads so nothing falls through the cracks.
//!
//! Fires when:
//! - There are overdue commitments (high urgency)
//! - Open loops count exceeds a threshold (moderate urgency)
//! - Pending attention items across any channel (email, WhatsApp, etc.)

use crate::instincts::Instinct;
use crate::types::{CompanionState, InstinctCategory, TimeSensitivity, UrgeSpec};

pub struct OpenLoopsGuardianInstinct {
    /// Minimum open loops before the instinct fires.
    threshold: i64,
}

impl OpenLoopsGuardianInstinct {
    pub fn new(threshold: i64) -> Self {
        Self { threshold }
    }
}

impl Instinct for OpenLoopsGuardianInstinct {
    fn name(&self) -> &str {
        "open_loops_guardian"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        let mut urges = Vec::new();

        // 1. Overdue commitments — high urgency
        if state.overdue_commitment_count > 0 {
            let count = state.overdue_commitment_count;
            let urgency = (0.6 + count as f64 * 0.1).min(0.95);
            let msg = if count == 1 {
                "You have an overdue commitment. Want me to show your open loops?".to_string()
            } else {
                format!(
                    "You have {} overdue commitments. Want me to review your open loops?",
                    count
                )
            };
            urges.push(
                UrgeSpec::new(
                    "open_loops_guardian",
                    &format!("{} overdue commitment(s) need attention", count),
                    urgency,
                )
                .with_cooldown("open_loops:overdue")
                .with_message(&msg)
                .with_time_sensitivity(TimeSensitivity::Immediate)
                .with_category(InstinctCategory::Anticipatory)
                .with_context(serde_json::json!({
                    "trigger": "overdue_commitments",
                    "count": count,
                })),
            );
        }

        // 2. Too many open loops accumulating
        if state.open_loops_count >= self.threshold {
            let count = state.open_loops_count;
            let urgency = (0.3 + (count as f64 - self.threshold as f64) * 0.05).min(0.7);
            urges.push(
                UrgeSpec::new(
                    "open_loops_guardian",
                    &format!("{} open loops — consider reviewing", count),
                    urgency,
                )
                .with_cooldown("open_loops:accumulation")
                .with_message(&format!(
                    "You have {} open items across commitments, messages, and tasks. \
                     Want to review what's pending?",
                    count
                ))
                .with_time_sensitivity(TimeSensitivity::Soon)
                .with_category(InstinctCategory::Anticipatory)
                .with_context(serde_json::json!({
                    "trigger": "accumulation",
                    "count": count,
                    "threshold": self.threshold,
                })),
            );
        }

        // 3. Pending attention items (unanswered messages across channels)
        if state.pending_attention_count > 0 {
            let count = state.pending_attention_count;
            let urgency = (0.35 + count as f64 * 0.08).min(0.75);
            let msg = if count == 1 {
                "You have an unanswered message that's been waiting. Want me to show it?"
                    .to_string()
            } else {
                format!(
                    "You have {} unanswered messages across your channels. Want to review them?",
                    count
                )
            };
            urges.push(
                UrgeSpec::new(
                    "open_loops_guardian",
                    &format!("{} unanswered message(s) waiting", count),
                    urgency,
                )
                .with_cooldown("open_loops:attention")
                .with_message(&msg)
                .with_time_sensitivity(TimeSensitivity::Soon)
                .with_category(InstinctCategory::Anticipatory)
                .with_context(serde_json::json!({
                    "trigger": "pending_attention",
                    "count": count,
                })),
            );
        }

        urges
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BondLevel;

    fn base_state() -> CompanionState {
        CompanionState {
            last_interaction_ts: 0.0,
            current_ts: 1000.0,
            session_active: false,
            conversation_turn_count: 0,
            recent_valence_avg: None,
            pending_triggers: vec![],
            active_patterns: vec![],
            open_conflicts_count: 0,
            memory_count: 0,
            config_user_name: "Test".into(),
            bond_level: BondLevel::Acquaintance,
            bond_score: 0.0,
            formality: 0.5,
            opinions_count: 0,
            shared_references_count: 0,
            bond_level_changed: false,
            current_hour: 10,
            current_day_of_week: 1,
            idle_seconds: 0.0,
            interactions_last_hour: 0,
            workflow_hints: vec![],
            maintenance_report: vec![],
            recent_events: vec![],
            avg_user_msg_length: 0.0,
            daily_proactive_count: 0,
            recent_sent_messages: vec![],
            suppressed_urges: vec![],
            user_interests: vec![],
            user_location: String::new(),
            open_loops_count: 0,
            overdue_commitment_count: 0,
            pending_attention_count: 0,
        }
    }

    #[test]
    fn no_urges_when_clean() {
        let instinct = OpenLoopsGuardianInstinct::new(5);
        let state = base_state();
        let urges = instinct.evaluate(&state);
        assert!(urges.is_empty());
    }

    #[test]
    fn fires_on_overdue_commitments() {
        let instinct = OpenLoopsGuardianInstinct::new(5);
        let mut state = base_state();
        state.overdue_commitment_count = 2;
        let urges = instinct.evaluate(&state);
        assert_eq!(urges.len(), 1);
        assert!(urges[0].urgency >= 0.7);
        assert!(urges[0].cooldown_key.contains("overdue"));
    }

    #[test]
    fn fires_on_accumulation() {
        let instinct = OpenLoopsGuardianInstinct::new(3);
        let mut state = base_state();
        state.open_loops_count = 5;
        let urges = instinct.evaluate(&state);
        assert_eq!(urges.len(), 1);
        assert!(urges[0].cooldown_key.contains("accumulation"));
    }

    #[test]
    fn fires_on_pending_attention() {
        let instinct = OpenLoopsGuardianInstinct::new(10);
        let mut state = base_state();
        state.pending_attention_count = 3;
        let urges = instinct.evaluate(&state);
        assert_eq!(urges.len(), 1);
        assert!(urges[0].cooldown_key.contains("attention"));
    }

    #[test]
    fn multiple_triggers_produce_multiple_urges() {
        let instinct = OpenLoopsGuardianInstinct::new(3);
        let mut state = base_state();
        state.overdue_commitment_count = 1;
        state.open_loops_count = 5;
        state.pending_attention_count = 2;
        let urges = instinct.evaluate(&state);
        assert_eq!(urges.len(), 3, "Should produce separate urges for each trigger");
    }

    #[test]
    fn urgency_scales_with_count() {
        let instinct = OpenLoopsGuardianInstinct::new(5);
        let mut state = base_state();

        state.overdue_commitment_count = 1;
        let u1 = instinct.evaluate(&state)[0].urgency;

        state.overdue_commitment_count = 5;
        let u5 = instinct.evaluate(&state)[0].urgency;

        assert!(u5 > u1, "More overdue items should mean higher urgency");
    }
}
