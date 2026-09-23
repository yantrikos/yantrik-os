//! Shared fixtures for instinct tests.
//!
//! The absence-driven instincts read two coupled fields: `last_interaction_ts`
//! (the bond store's clock — unset, 0.0, until the first scored turn, #156) and
//! the `idle_seconds` that `CompanionService::build_state` derives from it.
//! These fixtures keep the two coupled the way production does, so a test that
//! sets one cannot accidentally contradict the other.

use yantrik_companion_core::types::{BondLevel, CompanionState, ModelTier};

/// A plausible "now" (September 2026) so timestamps subtract the way they do in
/// production. Read as an absence, an unset clock would be this many seconds
/// since the epoch — the decades no absence gate may believe (#156).
pub(crate) const TEST_NOW: f64 = 1_790_000_000.0;

/// State as `build_state` produces it over a store whose last scored interaction
/// was `ago` seconds before `current_ts` — or over a store the person never
/// wrote to when `ago` is `None`: the clock stays unset and the derived
/// `idle_seconds` reads as the time since 1970.
pub(crate) fn state_at(current_ts: f64, ago: Option<f64>) -> CompanionState {
    let last_interaction_ts = ago.map_or(0.0, |ago| current_ts - ago);
    CompanionState {
        last_interaction_ts,
        current_ts,
        session_active: false,
        conversation_turn_count: 0,
        recent_valence_avg: None,
        pending_triggers: vec![],
        active_patterns: vec![],
        open_conflicts_count: 0,
        memory_count: 0,
        config_user_name: "Test".into(),
        bond_level: BondLevel::Stranger,
        bond_score: 0.0,
        formality: 0.5,
        opinions_count: 0,
        shared_references_count: 0,
        bond_level_changed: false,
        current_hour: 10,
        current_day_of_week: 1,
        idle_seconds: current_ts - last_interaction_ts,
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
        // Medium is what the configured model actually reports (the same choice
        // open_loops_guardian's fixture makes), so tests exercise the tier
        // branch the desktop takes.
        model_tier: ModelTier::Medium,
    }
}

/// [`state_at`] `TEST_NOW`.
pub(crate) fn state_idle_for(ago: Option<f64>) -> CompanionState {
    state_at(TEST_NOW, ago)
}
