//! Memory Weaver instinct — drives proactive memory graph building during idle time.
//!
//! When the system is idle, this instinct generates a single EXECUTE urge to
//! organize, link, and make sense of memories. Uses a single cooldown key
//! to prevent repetitive messages.

use crate::instincts::Instinct;
use crate::types::{CompanionState, UrgeSpec};

pub struct MemoryWeaverInstinct {
    /// Minimum idle time (seconds) before weaving urges fire.
    idle_threshold_secs: f64,
    /// Minimum memories before weaving is worthwhile.
    min_memories: i64,
}

impl MemoryWeaverInstinct {
    pub fn new(idle_threshold_minutes: f64, min_memories: i64) -> Self {
        Self {
            idle_threshold_secs: idle_threshold_minutes * 60.0,
            min_memories,
        }
    }
}

impl Instinct for MemoryWeaverInstinct {
    fn name(&self) -> &str {
        "MemoryWeaver"
    }

    fn evaluate(&self, state: &CompanionState) -> Vec<UrgeSpec> {
        // Only weave when idle and have enough memories to link
        let idle_secs = state.current_ts - state.last_interaction_ts;
        if idle_secs < self.idle_threshold_secs || state.memory_count < self.min_memories {
            return vec![];
        }

        // Urgency scales with idle time: starts at 0.2, caps at 0.4
        // Deliberately low — weaving is background contemplation
        let idle_factor = ((idle_secs - self.idle_threshold_secs) / 3600.0).min(1.0);
        let urgency = 0.2 + idle_factor * 0.2;

        // Build context about what to weave
        let has_conflicts = state.open_conflicts_count > 0;
        let has_patterns = !state.active_patterns.is_empty();

        let execute_msg = format!(
            "EXECUTE Review my memory graph ({} memories{}{}). \
             Find one interesting connection between memories from different conversations \
             and share it as a brief, natural insight (1-2 sentences). \
             If nothing interesting stands out, say nothing.",
            state.memory_count,
            if has_conflicts { format!(", {} conflicts to resolve", state.open_conflicts_count) } else { String::new() },
            if has_patterns { format!(", {} active patterns", state.active_patterns.len()) } else { String::new() },
        );

        // Single urge, single cooldown key — prevents flooding
        vec![UrgeSpec::new(
            self.name(),
            &execute_msg,
            urgency,
        )
        .with_cooldown("weaver:digest")
        .with_context(serde_json::json!({
            "idle_seconds": idle_secs,
            "memory_count": state.memory_count,
            "conflicts": state.open_conflicts_count,
            "patterns": state.active_patterns.len(),
        }))]
    }
}
