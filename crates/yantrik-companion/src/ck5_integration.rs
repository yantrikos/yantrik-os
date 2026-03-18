//! CK-5 Generative Understanding — companion integration.
//!
//! Wires CK-5 cognitive primitives (analogy, schema induction, narrative arcs,
//! counterfactual reasoning, belief propagation, experience replay, perspectives)
//! into the companion's think cycle so they fire during real usage.

use std::collections::HashMap;

use yantrikdb_core::{
    YantrikDB,
    // Narrative
    NarrativeEpisode, ArcAlert,
    // Schema induction
    EpisodeData, SchemaContext, SchemaCondition, SchemaId,
    // Replay
    ActionRecord, OutcomeData, DreamReport,
    // Perspective
    ActivationContext, PerspectiveTransition, CognitiveStyle,
    // Belief network
    NetworkHealth,
    // Analogy
    AnalogyMaintenanceReport,
    // Counterfactual
    DecisionRecord, RegretReport,
    // Types
    NodeId, NodeKind,
};

use crate::config::CK5Config;
use crate::types::{UrgeSpec, TimeSensitivity, InstinctCategory};

// ══════════════════════════════════════════════════════════════════════════════
// § 1  CK-5 Cycle Report
// ══════════════════════════════════════════════════════════════════════════════

/// Summary of all CK-5 operations run during one think cycle.
#[derive(Debug, Default)]
pub struct CK5CycleReport {
    /// Narrative arc alerts (stalled arcs, milestones).
    pub arc_alerts: Vec<ArcAlert>,
    /// Whether a new narrative episode was assigned to an arc.
    pub narrative_episode_assigned: bool,
    /// Matched schemas from recent context.
    pub matched_schemas: Vec<(SchemaId, f64)>,
    /// Dream cycle report (if run).
    pub dream_report: Option<DreamReport>,
    /// Perspective shifts detected.
    pub perspective_shifts: Vec<PerspectiveTransition>,
    /// Current cognitive style from perspective stack.
    pub cognitive_style: Option<CognitiveStyle>,
    /// Belief network health (if checked).
    pub network_health: Option<NetworkHealth>,
    /// Analogy maintenance report (if run).
    pub analogy_maintenance: Option<AnalogyMaintenanceReport>,
    /// Regret detection results (if run).
    pub regret_report: Option<RegretReport>,
    /// Number of schema episodes observed.
    pub schemas_observed: usize,
    /// Number of replay buffer entries pruned.
    pub replay_entries_pruned: usize,
}

// ══════════════════════════════════════════════════════════════════════════════
// § 2  Main Integration — Think Cycle Hook
// ══════════════════════════════════════════════════════════════════════════════

/// Run all CK-5 cognitive primitives as part of the background think cycle.
///
/// Called after memory evolution (step 9) in `background::run_think_cycle`.
/// Adapts intensity based on idle time:
/// - Active (< 5 min idle): lightweight ops only (perspective, schema match)
/// - Idle (5-30 min): add narrative, belief health, schema observation
/// - Deep idle (> 30 min): full dream cycle, analogy maintenance, regret detection
pub fn run_ck5_cycle(
    db: &YantrikDB,
    idle_secs: f64,
    config: &CK5Config,
    interaction_summary: Option<&InteractionSummary>,
) -> CK5CycleReport {
    let mut report = CK5CycleReport::default();
    let now_ms = now_ms();

    if !config.enabled {
        return report;
    }

    // ── Always: perspective shift detection ──
    if config.perspective_enabled {
        run_perspective_cycle(db, config, now_ms, &mut report);
    }

    // ── Always: schema matching against current context ──
    if config.schema_induction_enabled {
        run_schema_match(db, now_ms, &mut report);
    }

    // ── On interaction: record episode for schemas + narrative ──
    if let Some(summary) = interaction_summary {
        if config.schema_induction_enabled {
            observe_interaction_episode(db, summary, now_ms, &mut report);
        }
        if config.narrative_enabled {
            assign_narrative_episode(db, summary, now_ms, &mut report);
        }
        if config.replay_enabled {
            record_replay_experience(db, summary, now_ms);
        }
    }

    // ── Idle (> 5 min): narrative arc health, belief network health ──
    if idle_secs > config.idle_threshold_secs {
        if config.narrative_enabled {
            check_arc_health(db, now_ms, &mut report);
        }
        if config.belief_network_enabled {
            check_belief_health(db, &mut report);
        }
    }

    // ── Deep idle (> 30 min): dream cycle, analogy maintenance, regret detection ──
    if idle_secs > config.deep_idle_threshold_secs {
        if config.replay_enabled {
            run_dream_cycle(db, now_ms, &mut report);
        }
        if config.analogy_enabled {
            run_analogy_maintenance(db, now_ms, config, &mut report);
        }
        if config.counterfactual_enabled {
            run_regret_detection(db, &mut report);
        }

        // Replay buffer and schema maintenance
        if config.replay_enabled {
            run_replay_maintenance(db, now_ms, config, &mut report);
        }
        if config.schema_induction_enabled {
            run_schema_maintenance(db, now_ms, config);
        }
    }

    if report.has_activity() {
        tracing::debug!(
            arcs = report.arc_alerts.len(),
            schemas = report.matched_schemas.len(),
            shifts = report.perspective_shifts.len(),
            dream = report.dream_report.is_some(),
            "CK-5 cycle complete"
        );
    }

    report
}

// ══════════════════════════════════════════════════════════════════════════════
// § 3  Interaction Summary (bridge from companion events)
// ══════════════════════════════════════════════════════════════════════════════

/// Summary of a user interaction, used to feed CK-5 primitives.
#[derive(Debug, Clone)]
pub struct InteractionSummary {
    /// The topic/summary of what was discussed.
    pub summary: String,
    /// Tools that were called during the interaction.
    pub tools_used: Vec<String>,
    /// Domain categories detected.
    pub domains: Vec<String>,
    /// Sentiment/valence of the interaction [-1.0, 1.0].
    pub sentiment: f64,
    /// Whether the interaction outcome was positive.
    pub outcome_positive: bool,
    /// Node IDs involved (entities, memories, etc.).
    pub involved_nodes: Vec<NodeId>,
}

// ══════════════════════════════════════════════════════════════════════════════
// § 4  Individual CK-5 Subsystem Runners
// ══════════════════════════════════════════════════════════════════════════════

/// Detect perspective shifts and update cognitive style.
fn run_perspective_cycle(
    db: &YantrikDB,
    config: &CK5Config,
    _now_ms: u64,
    report: &mut CK5CycleReport,
) {
    // Ensure preset perspectives exist
    ensure_presets_initialized(db);

    // Build activation context from real companion state
    let hour = current_hour();
    let ctx = build_rich_activation_context(db, hour);

    // Detect shifts
    match db.detect_perspective_shifts(&ctx) {
        Ok(shifts) => {
            for shift in &shifts {
                tracing::info!(
                    activate = ?shift.activate,
                    deactivate = ?shift.deactivate,
                    reason = %shift.reason,
                    "CK-5: Perspective shift detected"
                );
                // Auto-activate recommended perspectives
                if let Err(e) = db.activate_perspective(shift.activate) {
                    tracing::warn!("Failed to activate perspective: {e}");
                }
                if let Some(deact) = shift.deactivate {
                    let _ = db.deactivate_perspective(deact);
                }
            }
            report.perspective_shifts = shifts;
        }
        Err(e) => tracing::debug!("Perspective shift detection skipped: {e}"),
    }

    // Cache cognitive style
    match db.active_cognitive_style() {
        Ok(style) => report.cognitive_style = Some(style),
        Err(e) => tracing::debug!("Cognitive style read failed: {e}"),
    }

    // Check for perspective conflicts
    if config.perspective_conflict_check {
        match db.check_perspective_conflicts() {
            Ok(conflicts) if !conflicts.is_empty() => {
                tracing::warn!(
                    count = conflicts.len(),
                    "CK-5: Perspective conflicts detected"
                );
            }
            _ => {}
        }
    }
}

/// Match current context against induced schemas.
fn run_schema_match(db: &YantrikDB, now_ms: u64, report: &mut CK5CycleReport) {
    let ctx = SchemaContext {
        node_kinds_present: vec![NodeKind::Entity, NodeKind::Episode],
        edge_kinds_present: Vec::new(),
        attributes: HashMap::new(),
        belief_confidences: HashMap::new(),
        now_ms,
    };

    match db.find_matching_schemas(&ctx) {
        Ok(matches) => {
            if !matches.is_empty() {
                tracing::debug!(
                    count = matches.len(),
                    top_score = matches.first().map(|m| m.1).unwrap_or(0.0),
                    "CK-5: Schema matches found"
                );
            }
            report.matched_schemas = matches;
        }
        Err(e) => tracing::debug!("Schema matching skipped: {e}"),
    }
}

/// Observe an interaction as a schema episode.
fn observe_interaction_episode(
    db: &YantrikDB,
    summary: &InteractionSummary,
    now_ms: u64,
    report: &mut CK5CycleReport,
) {
    let episode = EpisodeData {
        episode_id: episode_node_id(now_ms),
        conditions: vec![
            SchemaCondition::NodeKindPresent(NodeKind::Episode),
        ],
        action_type: if summary.tools_used.is_empty() {
            "conversation".to_string()
        } else {
            summary.tools_used.first().cloned().unwrap_or_else(|| "unknown".to_string())
        },
        outcome_positive: summary.outcome_positive,
        outcome_description: summary.summary.clone(),
        outcome_valence: summary.sentiment,
        timestamp_ms: now_ms,
    };

    match db.observe_episode_for_schema(&episode) {
        Ok(()) => report.schemas_observed += 1,
        Err(e) => tracing::debug!("Schema observation failed: {e}"),
    }
}

/// Assign a narrative episode to an arc.
fn assign_narrative_episode(
    db: &YantrikDB,
    summary: &InteractionSummary,
    now_ms: u64,
    report: &mut CK5CycleReport,
) {
    let episode = NarrativeEpisode {
        episode_id: episode_node_id(now_ms),
        summary: summary.summary.clone(),
        participants: summary.involved_nodes.clone(),
        domains: summary.domains.clone(),
        sentiment: summary.sentiment,
        timestamp_ms: now_ms,
        related_goal: None,
    };

    match db.assign_episode_to_arc(&episode) {
        Ok(arc_id) => {
            tracing::debug!(arc = arc_id.0, "CK-5: Episode assigned to narrative arc");
            report.narrative_episode_assigned = true;
        }
        Err(e) => tracing::debug!("Narrative arc assignment failed: {e}"),
    }
}

/// Record an interaction as a replay experience.
fn record_replay_experience(
    db: &YantrikDB,
    summary: &InteractionSummary,
    now_ms: u64,
) {
    let action = ActionRecord {
        description: summary.summary.clone(),
        domain: summary.domains.first().cloned().unwrap_or_else(|| "general".to_string()),
        involved_nodes: summary.involved_nodes.clone(),
    };

    let outcome = OutcomeData {
        utility: summary.sentiment,
        expected: summary.outcome_positive,
        domains: summary.domains.clone(),
        affected_nodes: summary.involved_nodes.clone(),
    };

    // Expected utility is neutral — deviation from this becomes the TD error
    let expected_utility = 0.0;

    if let Err(e) = db.record_experience(
        episode_node_id(now_ms),
        expected_utility,
        action,
        outcome,
        now_ms,
    ) {
        tracing::debug!("Replay experience recording failed: {e}");
    }
}

/// Check narrative arc health for stalled/concerning arcs.
fn check_arc_health(db: &YantrikDB, now_ms: u64, report: &mut CK5CycleReport) {
    match db.arc_health_report(now_ms) {
        Ok(alerts) => {
            for alert in &alerts {
                tracing::info!(
                    arc = alert.arc_id.0,
                    alert_type = ?alert.alert_type,
                    "CK-5: Narrative arc alert"
                );
            }
            report.arc_alerts = alerts;
        }
        Err(e) => tracing::debug!("Arc health check failed: {e}"),
    }
}

/// Check belief network health.
fn check_belief_health(db: &YantrikDB, report: &mut CK5CycleReport) {
    match db.belief_network_health() {
        Ok(health) => {
            if !health.extreme_priors.is_empty() || !health.potential_instabilities.is_empty() {
                tracing::info!(
                    extreme = health.extreme_priors.len(),
                    instabilities = health.potential_instabilities.len(),
                    healthy = health.healthy,
                    "CK-5: Belief network health issues"
                );
            }
            report.network_health = Some(health);
        }
        Err(e) => tracing::debug!("Belief network health check skipped: {e}"),
    }
}

/// Run the dream/replay cycle during deep idle, feeding real beliefs.
fn run_dream_cycle(db: &YantrikDB, now_ms: u64, report: &mut CK5CycleReport) {
    match db.should_run_replay(now_ms) {
        Ok(true) => {
            // Feed real beliefs from the flywheel into dream replay
            let current_beliefs: Vec<(NodeId, f64)> = db.load_belief_store()
                .map(|store| {
                    store.established().iter().map(|b| {
                        // Use the belief's dedup_key hash as a stable node ID
                        let seq = (b.id as u32) & 0x0FFF_FFFF;
                        (NodeId::new(NodeKind::Belief, seq), b.confidence)
                    }).collect()
                })
                .unwrap_or_default();

            match db.run_dream_cycle(&current_beliefs, now_ms) {
                Ok(dream) => {
                    if dream.replays_executed > 0 {
                        tracing::info!(
                            replays = dream.replays_executed,
                            "CK-5: Dream cycle completed"
                        );
                    }
                    report.dream_report = Some(dream);
                }
                Err(e) => tracing::debug!("Dream cycle failed: {e}"),
            }
        }
        Ok(false) => {}
        Err(e) => tracing::debug!("Replay check failed: {e}"),
    }
}

/// Run analogy store maintenance (decay old mappings).
fn run_analogy_maintenance(
    db: &YantrikDB,
    now_ms: u64,
    config: &CK5Config,
    report: &mut CK5CycleReport,
) {
    match db.run_analogy_maintenance(now_ms, config.analogy_max_age_ms) {
        Ok(maint) => {
            let total_pruned = maint.pruned_low_quality + maint.pruned_stale;
            if total_pruned > 0 {
                tracing::debug!(
                    pruned_quality = maint.pruned_low_quality,
                    pruned_stale = maint.pruned_stale,
                    remaining = maint.remaining,
                    "CK-5: Analogy maintenance"
                );
            }
            report.analogy_maintenance = Some(maint);
        }
        Err(e) => tracing::debug!("Analogy maintenance failed: {e}"),
    }
}

/// Detect regrets from recent decisions.
fn run_regret_detection(db: &YantrikDB, report: &mut CK5CycleReport) {
    // Build decision records from recent replay buffer
    match db.replay_engine_summary() {
        Ok(summary) if summary.buffer_size > 0 => {
            // Use empty decisions for now — regret detection will compare
            // against counterfactual outcomes internally
            let decisions: Vec<DecisionRecord> = Vec::new();
            match db.detect_regrets(&decisions) {
                Ok(regret) => {
                    if !regret.top_regrets.is_empty() {
                        tracing::info!(
                            regrets = regret.top_regrets.len(),
                            rate = regret.regret_rate,
                            "CK-5: Regrets detected"
                        );
                    }
                    // Persist actionable insight for context injection
                    if let Some(ref insight) = regret.actionable_insight {
                        let _ = db.conn().execute(
                            "INSERT OR REPLACE INTO meta (key, value) VALUES ('ck5_regret_insight', ?1)",
                            rusqlite::params![insight],
                        );
                    }
                    report.regret_report = Some(regret);
                }
                Err(e) => tracing::debug!("Regret detection failed: {e}"),
            }
        }
        _ => {}
    }
}

/// Prune expired entries from the replay buffer.
fn run_replay_maintenance(
    db: &YantrikDB,
    now_ms: u64,
    _config: &CK5Config,
    report: &mut CK5CycleReport,
) {
    match db.replay_buffer_maintenance(now_ms) {
        Ok(pruned) => report.replay_entries_pruned = pruned,
        Err(e) => tracing::debug!("Replay maintenance failed: {e}"),
    }
}

/// Run schema maintenance (evict expired schemas).
fn run_schema_maintenance(db: &YantrikDB, now_ms: u64, config: &CK5Config) {
    match db.run_schema_maintenance(now_ms, config.schema_max_age_ms) {
        Ok(maint) => {
            let total = maint.pruned_low_confidence + maint.decayed;
            if total > 0 {
                tracing::debug!(
                    pruned = maint.pruned_low_confidence,
                    decayed = maint.decayed,
                    merged = maint.merged,
                    "CK-5: Schema maintenance"
                );
            }
        }
        Err(e) => tracing::debug!("Schema maintenance failed: {e}"),
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// § 5  Helpers
// ══════════════════════════════════════════════════════════════════════════════

/// Build an ActivationContext enriched with real companion data.
fn build_rich_activation_context(db: &YantrikDB, hour: u8) -> ActivationContext {
    // Active goals from agenda items — map title hash to NodeId(Goal)
    let active_goals: Vec<NodeId> = db.conn()
        .prepare("SELECT title FROM agenda_items WHERE status = 'active' ORDER BY urgency DESC LIMIT 5")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<String>>())
        })
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, _)| NodeId::new(NodeKind::Goal, i as u32))
        .collect();

    // Detected patterns from the pattern store
    let detected_patterns: Vec<String> = db
        .get_patterns(None, Some("active"), 5)
        .unwrap_or_default()
        .iter()
        .map(|p| p.description.clone())
        .collect();

    // Active apps from system context (stored in meta by UI wire)
    let active_apps: Vec<String> = db.conn()
        .query_row(
            "SELECT value FROM meta WHERE key = 'active_apps'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
        .unwrap_or_default();

    ActivationContext {
        hour,
        active_apps,
        active_goals,
        stress_level: estimate_stress_level(db),
        detected_patterns,
    }
}

/// Ensure the three preset perspectives exist (created once).
fn ensure_presets_initialized(db: &YantrikDB) {
    // Check if we've already initialized by looking for any perspective
    match db.load_perspective_store() {
        Ok(store) if !store.is_empty() => return,
        Err(_) => return,
        _ => {}
    }

    let now = now_ms();
    for preset in &["creative", "deadline", "reflective"] {
        let _ = db.create_preset_perspective(preset, now);
    }
    tracing::info!("CK-5: Initialized preset perspectives");
}

/// Generate a unique episode NodeId from a timestamp.
fn episode_node_id(ts_ms: u64) -> NodeId {
    // Use lower 28 bits of ms timestamp as sequence (wraps every ~4.5 minutes, fine for uniqueness)
    let seq = (ts_ms as u32) & 0x0FFF_FFFF;
    NodeId::new(NodeKind::Episode, seq)
}

fn current_hour() -> u8 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let day_secs = ((secs % 86400) + 86400) % 86400;
    (day_secs / 3600) as u8
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Estimate stress level from recent cognitive signals.
fn estimate_stress_level(db: &YantrikDB) -> f64 {
    // Heuristic: high conflict count + negative valence = stress
    let conflicts = db
        .get_conflicts(Some("open"), None, None, None, 100)
        .map(|c| c.len())
        .unwrap_or(0);

    let base = (conflicts as f64 * 0.1).min(0.5);
    base
}

impl CK5CycleReport {
    /// Whether any CK-5 activity occurred worth logging.
    pub fn has_activity(&self) -> bool {
        !self.arc_alerts.is_empty()
            || self.narrative_episode_assigned
            || !self.matched_schemas.is_empty()
            || self.dream_report.is_some()
            || !self.perspective_shifts.is_empty()
            || self.network_health.is_some()
            || self.analogy_maintenance.is_some()
            || self.regret_report.is_some()
    }

    /// Build a summary string for logging/proactive messaging.
    pub fn summary_text(&self) -> Option<String> {
        let mut parts = Vec::new();

        if !self.arc_alerts.is_empty() {
            parts.push(format!("{} narrative arc alert(s)", self.arc_alerts.len()));
        }
        if !self.matched_schemas.is_empty() {
            parts.push(format!("{} schema match(es)", self.matched_schemas.len()));
        }
        if let Some(ref dream) = self.dream_report {
            if dream.replays_executed > 0 {
                parts.push(format!("{} dream replay(s)", dream.replays_executed));
            }
        }
        if !self.perspective_shifts.is_empty() {
            parts.push(format!("{} perspective shift(s)", self.perspective_shifts.len()));
        }
        if let Some(ref regret) = self.regret_report {
            if !regret.top_regrets.is_empty() {
                parts.push(format!("{} regret(s) detected", regret.top_regrets.len()));
            }
        }

        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// § 6  CK-5 Context Snippet — feeds insights into the LLM system prompt
// ══════════════════════════════════════════════════════════════════════════════

/// Compact CK-5 awareness for injection into the LLM system prompt.
/// Built from the latest CK5CycleReport + persistent DB state.
#[derive(Debug, Default, Clone)]
pub struct CK5ContextSnippet {
    /// Active narrative arc summaries (e.g., "Building Yantrik OS — active, 12 episodes").
    pub active_arcs: Vec<String>,
    /// Current cognitive style description (exploration vs exploitation, etc.).
    pub cognitive_style_hint: Option<String>,
    /// Top matched schema names + confidence (behavioral patterns the companion recognizes).
    pub schema_hints: Vec<String>,
    /// Actionable insight from regret detection (e.g., "Last time you did X, Y happened").
    pub regret_insight: Option<String>,
    /// Belief-derived self-knowledge (established autonomous beliefs).
    pub self_knowledge: Vec<String>,
}

impl CK5ContextSnippet {
    pub fn is_empty(&self) -> bool {
        self.active_arcs.is_empty()
            && self.cognitive_style_hint.is_none()
            && self.schema_hints.is_empty()
            && self.regret_insight.is_none()
            && self.self_knowledge.is_empty()
    }

    /// Format as a compact string for the system prompt (~100-300 chars).
    pub fn format_for_prompt(&self, max_chars: usize) -> String {
        let mut out = String::with_capacity(max_chars);

        // Narrative arcs — situational awareness
        if !self.active_arcs.is_empty() {
            out.push_str("Active life threads: ");
            out.push_str(&self.active_arcs.join("; "));
            out.push('\n');
        }

        // Cognitive style — behavioral tuning
        if let Some(ref style) = self.cognitive_style_hint {
            out.push_str(style);
            out.push('\n');
        }

        // Schema hints — pattern recognition
        if !self.schema_hints.is_empty() {
            out.push_str("Recognized patterns: ");
            out.push_str(&self.schema_hints.join(", "));
            out.push('\n');
        }

        // Self-knowledge — autonomous beliefs
        if !self.self_knowledge.is_empty() {
            out.push_str("Things you've learned: ");
            out.push_str(&self.self_knowledge.join("; "));
            out.push('\n');
        }

        // Regret insight — learning from mistakes
        if let Some(ref insight) = self.regret_insight {
            out.push_str("Lesson learned: ");
            out.push_str(insight);
            out.push('\n');
        }

        // Truncate to budget
        if out.len() > max_chars {
            let mut end = max_chars;
            while end > 0 && !out.is_char_boundary(end) {
                end -= 1;
            }
            out.truncate(end);
        }

        out
    }
}

/// Build a CK5ContextSnippet from the database state.
/// Called during context assembly (not during the CK-5 cycle itself).
pub fn build_context_snippet(db: &YantrikDB) -> CK5ContextSnippet {
    let mut snippet = CK5ContextSnippet::default();

    // 1. Active narrative arcs
    if let Ok(timeline) = db.load_timeline() {
        for arc in timeline.active_arcs() {
            let episode_count = arc.chapters.len();
            let theme = format!("{:?}", arc.theme).to_lowercase();
            snippet.active_arcs.push(format!(
                "{} ({}, {} episodes)", arc.title, theme, episode_count
            ));
        }
        // Cap at 3 arcs
        snippet.active_arcs.truncate(3);
    }

    // 2. Cognitive style
    if let Ok(style) = db.active_cognitive_style() {
        let explore = style.exploration_vs_exploitation;
        let mode = if explore > 0.6 {
            "You're in an exploratory mindset — open to new ideas and tangents."
        } else if explore < 0.4 {
            "You're in a focused/execution mindset — prioritize getting things done."
        } else {
            "You're balanced between exploration and execution."
        };
        snippet.cognitive_style_hint = Some(mode.to_string());
    }

    // 3. Top matched schemas (from last cycle, stored in meta)
    if let Ok(store) = db.load_induced_schema_store() {
        let mut schemas: Vec<_> = store.schemas.iter()
            .filter(|s| s.confidence > 0.5)
            .collect();
        schemas.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
        for schema in schemas.iter().take(2) {
            snippet.schema_hints.push(format!(
                "{} ({:.0}% confident)", schema.name, schema.confidence * 100.0
            ));
        }
    }

    // 4. Regret insight (from last regret detection, stored in meta table)
    if let Ok(insight) = db.conn().query_row(
        "SELECT value FROM meta WHERE key = 'ck5_regret_insight'",
        [],
        |row| row.get::<_, String>(0),
    ) {
        let insight = insight.trim_matches('"').to_string();
        if !insight.is_empty() {
            snippet.regret_insight = Some(insight);
        }
    }

    // 5. Established autonomous beliefs — self-knowledge
    if let Ok(belief_store) = db.load_belief_store() {
        let established = belief_store.established();
        for belief in established.iter().take(3) {
            snippet.self_knowledge.push(belief.description.clone());
        }
    }

    snippet
}

// ══════════════════════════════════════════════════════════════════════════════
// § 7  CK-5 Urge Generation — converts findings into proactive urges
// ══════════════════════════════════════════════════════════════════════════════

/// Convert CK-5 cycle findings into companion urges that drive proactive behavior.
pub fn generate_ck5_urges(report: &CK5CycleReport) -> Vec<UrgeSpec> {
    let mut urges = Vec::new();

    // Arc alerts → proactive nudges about stalled goals
    for alert in &report.arc_alerts {
        let urgency = match alert.alert_type {
            yantrikdb_core::ArcAlertType::Stalled => 0.5,
            yantrikdb_core::ArcAlertType::TrendingNegative => 0.6,
            yantrikdb_core::ArcAlertType::AbandonedUnresolved => 0.4,
        };
        let reason = match alert.alert_type {
            yantrikdb_core::ArcAlertType::Stalled => format!(
                "Your \"{}\" thread has gone quiet — maybe check in on it?",
                alert.arc_title
            ),
            yantrikdb_core::ArcAlertType::TrendingNegative => format!(
                "Things seem to be going sideways with \"{}\". Want to talk about it?",
                alert.arc_title
            ),
            yantrikdb_core::ArcAlertType::AbandonedUnresolved => format!(
                "You left \"{}\" unfinished — still on your mind or ready to close it?",
                alert.arc_title
            ),
        };

        let mut urge = UrgeSpec::new("ck5_narrative", &reason, urgency);
        urge.suggested_message = reason.clone();
        urge.cooldown_key = format!("ck5_arc_{}", alert.arc_id.0);
        urge.time_sensitivity = TimeSensitivity::Soon;
        urge.category = InstinctCategory::Meta;
        urges.push(urge);
    }

    // Regret detection → learning moment
    if let Some(ref regret) = report.regret_report {
        if let Some(ref insight) = regret.actionable_insight {
            if !insight.is_empty() {
                let mut urge = UrgeSpec::new(
                    "ck5_regret",
                    &format!("I noticed something from past decisions: {}", insight),
                    0.45,
                );
                urge.suggested_message = format!(
                    "Hey, I've been reflecting on some past decisions and noticed: {}",
                    insight
                );
                urge.cooldown_key = "ck5_regret_insight".to_string();
                urge.time_sensitivity = TimeSensitivity::Ambient;
                urge.category = InstinctCategory::Growth;
                urges.push(urge);
            }
        }
    }

    // Perspective shifts → awareness nudge
    for shift in &report.perspective_shifts {
        let mut urge = UrgeSpec::new(
            "ck5_perspective",
            &format!("Cognitive shift: {}", shift.reason),
            0.35,
        );
        urge.cooldown_key = format!("ck5_perspective_{:?}", shift.activate);
        urge.time_sensitivity = TimeSensitivity::Ambient;
        urge.category = InstinctCategory::Meta;
        urges.push(urge);
    }

    // Dream cycle insights — if replays surfaced something
    if let Some(ref dream) = report.dream_report {
        if dream.replays_executed > 2 {
            let mut urge = UrgeSpec::new(
                "ck5_dream",
                "I've been processing recent experiences and have some observations",
                0.3,
            );
            urge.suggested_message = format!(
                "I dreamed about {} recent experiences — patterns are emerging.",
                dream.replays_executed,
            );
            urge.cooldown_key = "ck5_dream_cycle".to_string();
            urge.time_sensitivity = TimeSensitivity::Ambient;
            urge.category = InstinctCategory::Meta;
            urges.push(urge);
        }
    }

    urges
}
