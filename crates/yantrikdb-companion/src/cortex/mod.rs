//! Context Cortex — cross-system intelligence framework.
//!
//! Transforms Yantrik from a toolbox into Jarvis by enabling compound reasoning
//! across Email, Jira, Git, Calendar, Browser, and Filesystem.
//!
//! Architecture (4-layer):
//! 1. **Pulses** — cheap event capture from every tool call
//! 2. **Entity Graph** — canonical entities + relationships across systems
//! 3. **Self-Learning Intelligence** — baselines, patterns, LLM reflection
//! 4. **Situation Briefing** — structured LLM-ready context when attention fires
//!
//! The cortex does NOT call the LLM directly. It outputs `AttentionItem`s
//! that get packaged as EXECUTE urges through the existing instinct pipeline.
//!
//! Self-learning replaces infinite hardcoded rules with three mechanisms:
//! - **Baseline Tracker** — Welford's algorithm, learns normal, flags deviations
//! - **Pattern Miner** — temporal association rules from pulse co-occurrence
//! - **LLM Reasoner** — periodic deep reflection (every ~4h, not every cycle)

pub mod schema;
pub mod pulse;
pub mod entity;
pub mod focus;
pub mod rules;
pub mod situation;
pub mod baselines;
pub mod patterns;
pub mod reasoner;
pub mod playbook;

use rusqlite::Connection;

pub use entity::{CanonicalEntity, EntityResolver, EntityType, SystemSource};
pub use focus::{ActivityType, FocusContext, FocusDetector};
pub use pulse::{Pulse, PulseCollector, PulseType};
pub use rules::{AttentionItem, RuleEngine};
pub use situation::{Situation, SituationBuilder};

/// The Context Cortex — owns the entity graph, pulse stream, rule engine,
/// and self-learning intelligence (baselines, patterns, reasoner).
///
/// Lives inside `CompanionService` and is called from the think cycle.
pub struct ContextCortex {
    entity_resolver: EntityResolver,
    pulse_collector: PulseCollector,
    rule_engine: RuleEngine,
    focus_detector: FocusDetector,
    situation_builder: SituationBuilder,
    baseline_tracker: baselines::BaselineTracker,
    pattern_miner: patterns::PatternMiner,
    llm_reasoner: reasoner::LlmReasoner,
    /// Counter for hourly tasks (pattern mining, baseline updates).
    think_count: u64,
}

impl ContextCortex {
    /// Initialize the cortex, creating tables if needed.
    pub fn init(conn: &Connection) -> Result<Self, Box<dyn std::error::Error>> {
        schema::create_tables(conn)?;
        Ok(Self {
            entity_resolver: EntityResolver::new(),
            pulse_collector: PulseCollector::new(),
            rule_engine: RuleEngine::new(),
            focus_detector: FocusDetector::new(),
            situation_builder: SituationBuilder::new(),
            baseline_tracker: baselines::BaselineTracker::new(),
            pattern_miner: patterns::PatternMiner::new(),
            llm_reasoner: reasoner::LlmReasoner::new(),
            think_count: 0,
        })
    }

    /// Initialize with a set of enabled services (gates which rules fire).
    pub fn init_with_services(conn: &Connection, services: &[String]) -> Result<Self, Box<dyn std::error::Error>> {
        schema::create_tables(conn)?;
        Ok(Self {
            entity_resolver: EntityResolver::new(),
            pulse_collector: PulseCollector::new(),
            rule_engine: RuleEngine::with_services(services),
            focus_detector: FocusDetector::new(),
            situation_builder: SituationBuilder::new(),
            baseline_tracker: baselines::BaselineTracker::new(),
            pattern_miner: patterns::PatternMiner::new(),
            llm_reasoner: reasoner::LlmReasoner::new(),
            think_count: 0,
        })
    }

    /// Update enabled services (used by Skill Store to override config).
    pub fn set_services(&mut self, services: &[String]) {
        self.rule_engine.set_services(services);
    }

    /// Ingest a tool result into the pulse stream.
    ///
    /// Called after every tool execution in the agent loop.
    /// Extracts entities, relationships, and feeds baselines.
    pub fn ingest_tool_result(
        &mut self,
        conn: &Connection,
        tool_name: &str,
        tool_args: &serde_json::Value,
        tool_result: &str,
    ) {
        let pulses = self.pulse_collector.extract_pulses(tool_name, tool_args, tool_result);
        for pulse in &pulses {
            // Resolve entities referenced in the pulse
            let resolved_entities = self.entity_resolver.resolve_pulse_entities(conn, pulse);

            // Store pulse and entity links
            if let Err(e) = schema::store_pulse(conn, pulse, &resolved_entities) {
                tracing::warn!(error = %e, "Failed to store cortex pulse");
            }
            // Derive relationships from pulse
            if let Err(e) = schema::derive_relationships(conn, pulse, &resolved_entities) {
                tracing::warn!(error = %e, "Failed to derive cortex relationships");
            }

            // Feed baseline tracker with metrics from this pulse
            let entity_ids: Vec<String> = resolved_entities
                .iter()
                .map(|re| re.entity_id.clone())
                .collect();
            baselines::extract_metrics_from_pulse(
                conn,
                &self.baseline_tracker,
                pulse.event_type.as_str(),
                &entity_ids,
                &pulse.metadata,
            );
        }
    }

    /// Update focus context from system snapshot.
    ///
    /// Called from the 60-second think cycle in bridge.rs.
    pub fn update_focus(
        &mut self,
        conn: &Connection,
        window_title: &str,
        process_name: &str,
        idle_seconds: u64,
    ) {
        self.focus_detector.update(conn, window_title, process_name, idle_seconds);
    }

    /// Run the cortex think step. Returns attention items from all sources.
    ///
    /// Called from the 60-second think cycle in bridge.rs.
    /// Combines: hardcoded rules + baseline deviations + pattern alerts.
    pub fn think(&mut self, conn: &Connection) -> Vec<AttentionItem> {
        let focus = self.focus_detector.current_focus();
        self.think_count += 1;

        // Decay relevance scores
        schema::decay_relevance(conn);

        // ── Layer 1: Hardcoded rules (seed set, fast, free) ──
        let mut attention = self.rule_engine.evaluate(conn, focus.as_ref());

        // ── Layer 2: Baseline deviations (learned norms) ──
        let deviations = self.baseline_tracker.check_deviations(conn);
        if !deviations.is_empty() {
            tracing::info!(count = deviations.len(), "Baseline deviations detected");
        }
        attention.extend(deviations);

        // ── Layer 3: Pattern-based alerts (missing consequents) ──
        let pattern_alerts = self.pattern_miner.check_missing_consequents(conn);
        if !pattern_alerts.is_empty() {
            tracing::info!(count = pattern_alerts.len(), "Pattern alerts detected");
        }
        attention.extend(pattern_alerts);

        // ── Hourly maintenance: mine patterns + update baselines ──
        // Every 60 think cycles ≈ every hour
        if self.think_count % 60 == 0 {
            tracing::info!("Cortex hourly maintenance: mining patterns + updating baselines");
            self.pattern_miner.mine(conn);
            self.baseline_tracker.update_from_pulses(conn);
            // Prune old pulses (keep 30 days)
            schema::prune_old_pulses(conn, 30.0 * 86400.0);
        }

        // Sort by priority, cap at top 3
        attention.sort_by(|a, b| {
            b.priority
                .partial_cmp(&a.priority)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        attention.truncate(3);

        attention
    }

    /// Check if the LLM reasoner wants to run (every ~4 hours).
    ///
    /// Returns an EXECUTE prompt string if it's time for deep reflection.
    /// Called from bridge.rs think cycle — separate from `think()` because
    /// this generates an EXECUTE urge directly rather than attention items.
    pub fn maybe_deep_reflection(&mut self, conn: &Connection) -> Option<String> {
        if !self.llm_reasoner.should_run() {
            return None;
        }
        let focus = self.focus_detector.current_focus();
        self.llm_reasoner.build_reflection_prompt(conn, focus.as_ref())
    }

    /// Build a situation briefing from attention items for LLM consumption.
    pub fn build_briefing(
        &self,
        conn: &Connection,
        focus: Option<&FocusContext>,
        attention: &[AttentionItem],
    ) -> String {
        let situation = self.situation_builder.build(conn, focus, attention);
        situation.as_llm_prompt()
    }

    /// Get the current focus context (for external callers).
    pub fn current_focus(&self) -> Option<FocusContext> {
        self.focus_detector.current_focus()
    }

    /// Seed entities from external connectors (Google, Spotify, etc).
    ///
    /// Maps SeedEntity fields to canonical cortex entities and upserts them.
    pub fn seed_entities(
        &self,
        conn: &Connection,
        entities: &[crate::connectors::SeedEntity],
    ) {
        for seed in entities {
            let entity_type = match entity::EntityType::from_str(seed.entity_type) {
                Some(t) => t,
                None => continue,
            };

            let source = match seed.source_system {
                "google" => entity::SystemSource::Calendar, // best fit for Google
                "spotify" => entity::SystemSource::Browser,  // no Spotify source, use Browser
                _ => entity::SystemSource::Memory,
            };

            let canonical_id = format!(
                "{}:{}",
                entity_type.as_str(),
                seed.identifier.to_lowercase().replace(' ', "-")
            );

            if let Err(e) = schema::upsert_entity(
                conn,
                &canonical_id,
                &seed.display_name,
                entity_type,
                source,
                &seed.external_id,
            ) {
                tracing::warn!(
                    entity = %canonical_id,
                    error = %e,
                    "Failed to seed connector entity"
                );
            }
        }

        if !entities.is_empty() {
            tracing::info!(
                count = entities.len(),
                "Seeded connector entities into cortex"
            );
        }
    }
}
