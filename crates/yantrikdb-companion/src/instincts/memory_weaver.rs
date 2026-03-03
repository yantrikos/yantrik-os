//! Memory Weaver instinct — drives proactive memory graph building during idle time.
//!
//! When the system is idle, this instinct generates urges to organize, link,
//! and make sense of memories. It's the "thinking while staring at the ceiling"
//! behavior — making connections between things learned at different times.

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
        let mut urges = Vec::new();

        // Only weave when idle and have enough memories to link
        let idle_secs = state.current_ts - state.last_interaction_ts;
        if idle_secs < self.idle_threshold_secs || state.memory_count < self.min_memories {
            return urges;
        }

        // Urgency scales with idle time: starts at 0.2, caps at 0.5
        // This is deliberately low — weaving is background contemplation, not urgent
        let idle_factor = ((idle_secs - self.idle_threshold_secs) / 3600.0).min(1.0);
        let base_urgency = 0.2 + idle_factor * 0.3;

        // Core urge: link unconnected memories
        urges.push(
            UrgeSpec::new(
                self.name(),
                &format!(
                    "I have {} memories — some might be related in ways I haven't noticed yet.",
                    state.memory_count
                ),
                base_urgency,
            )
            .with_cooldown("weaver:link_pass")
            .with_context(serde_json::json!({
                "idle_seconds": idle_secs,
                "memory_count": state.memory_count,
                "weave_type": "link_discovery",
            })),
        );

        // If there are unresolved conflicts, weaving can help make sense of them
        if state.open_conflicts_count > 0 {
            urges.push(
                UrgeSpec::new(
                    self.name(),
                    &format!(
                        "I have {} conflicting memories — I should try to resolve what's actually true.",
                        state.open_conflicts_count
                    ),
                    (base_urgency + 0.1).min(0.6),
                )
                .with_cooldown("weaver:conflict_resolution")
                .with_context(serde_json::json!({
                    "conflicts": state.open_conflicts_count,
                    "weave_type": "conflict_sense",
                })),
            );
        }

        // If there are patterns, weaving can enrich them with memory connections
        if !state.active_patterns.is_empty() {
            urges.push(
                UrgeSpec::new(
                    self.name(),
                    "I've noticed some patterns — let me trace them back through my memories.",
                    base_urgency,
                )
                .with_cooldown("weaver:pattern_trace")
                .with_context(serde_json::json!({
                    "pattern_count": state.active_patterns.len(),
                    "weave_type": "pattern_enrichment",
                })),
            );
        }

        // Milestone urge: first time weaving with a decent memory base
        if state.memory_count >= 50 && state.memory_count < 55 {
            urges.push(
                UrgeSpec::new(
                    self.name(),
                    "I've built up 50 memories now. I should start connecting the dots — see what picture emerges.",
                    0.45,
                )
                .with_cooldown("weaver:milestone_50")
                .with_message(
                    "I've been thinking while you were away. I have over 50 memories now, and I'm starting to see how things connect — your work patterns, the things you care about, the people in your life. It's like a map that's slowly filling in."
                ),
            );
        }

        urges.into_iter().take(2).collect()
    }
}
