//! Proactive Pipeline — 4-stage event-driven proactive intelligence.
//!
//! Replaces the old urge→template delivery model with a proper pipeline:
//!
//! 1. **Detect**: Subscribe to EventBus, buffer events, detect opportunity triggers
//! 2. **Generate**: Convert triggers into candidate interventions (what *could* we say?)
//! 3. **Score**: Multi-axis scoring (urgency, confidence, interruptibility, novelty,
//!    expected value, annoyance risk, historical acceptance rate)
//! 4. **Deliver**: Apply delivery policy (silence, ambient, whisper, badge, scheduled)
//!
//! The pipeline consumes `YantrikEvent` from the event bus and the existing
//! `UrgeSpec` from instincts. Both feed into the Generate stage.

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::bond::BondLevel;
use crate::config::ProactiveConfig;
use crate::proactive_templates::TemplateEngine;
use crate::types::{ProactiveMessage, Urge};
use crate::urges::UrgeQueue;

// ── Stage 1: Detect ─────────────────────────────────────────────────────────

/// An opportunity trigger detected from events or urges.
#[derive(Debug, Clone)]
pub struct OpportunityTrigger {
    /// What kind of trigger.
    pub kind: TriggerKind,
    /// Source instinct or event type.
    pub source: String,
    /// Reason or context description.
    pub reason: String,
    /// Pre-composed suggested message (may be empty).
    pub suggested_message: String,
    /// Rich context for template rendering.
    pub context: serde_json::Value,
    /// Raw urgency from the source (0.0-1.0).
    pub raw_urgency: f64,
    /// Cooldown key for deduplication.
    pub cooldown_key: String,
    /// When this trigger was created.
    pub created_at: f64,
}

/// Classification of what triggered this opportunity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerKind {
    /// An instinct-produced urge (existing system).
    Urge,
    /// A commitment deadline approaching/overdue.
    CommitmentAlert,
    /// Routine deviation detected.
    RoutineDeviation,
    /// System resource pressure (battery, CPU, disk).
    ResourceAlert,
    /// User returned from idle — opportunity for summary.
    UserResumed,
    /// Event bus: whisper card was dismissed (learning signal).
    DismissalSignal,
    /// External signal (email arrival, calendar reminder, etc.).
    ExternalSignal,
}

/// Detector: converts events and urges into opportunity triggers.
pub struct Detector {
    /// Recent events buffered for pattern detection.
    event_buffer: Vec<BufferedEvent>,
    /// Max events to buffer before oldest are dropped.
    buffer_capacity: usize,
}

#[derive(Debug, Clone)]
struct BufferedEvent {
    kind: String,
    summary: String,
    timestamp: f64,
}

impl Detector {
    pub fn new() -> Self {
        Self {
            event_buffer: Vec::new(),
            buffer_capacity: 100,
        }
    }

    /// Buffer an event for pattern detection.
    pub fn buffer_event(&mut self, kind: &str, summary: &str, timestamp: f64) {
        self.event_buffer.push(BufferedEvent {
            kind: kind.to_string(),
            summary: summary.to_string(),
            timestamp,
        });
        if self.event_buffer.len() > self.buffer_capacity {
            self.event_buffer.remove(0);
        }
    }

    /// Convert an urge from the existing system into an opportunity trigger.
    pub fn trigger_from_urge(urge: &Urge) -> OpportunityTrigger {
        OpportunityTrigger {
            kind: TriggerKind::Urge,
            source: urge.instinct_name.clone(),
            reason: urge.reason.clone(),
            suggested_message: urge.suggested_message.clone(),
            context: urge.context.clone(),
            raw_urgency: urge.urgency,
            cooldown_key: urge.cooldown_key.clone(),
            created_at: now_ts(),
        }
    }

    /// Detect triggers from event patterns (e.g., user resumed after long idle).
    pub fn detect_from_events(&self) -> Vec<OpportunityTrigger> {
        let mut triggers = Vec::new();
        let now = now_ts();

        // Pattern: user resumed after significant idle
        for evt in self.event_buffer.iter().rev().take(5) {
            if evt.kind == "user_resumed" && now - evt.timestamp < 10.0 {
                triggers.push(OpportunityTrigger {
                    kind: TriggerKind::UserResumed,
                    source: "event_bus".into(),
                    reason: format!("User resumed: {}", evt.summary),
                    suggested_message: String::new(),
                    context: serde_json::json!({"event": evt.summary}),
                    raw_urgency: 0.4,
                    cooldown_key: "user_resumed".into(),
                    created_at: now,
                });
                break;
            }
        }

        // Pattern: multiple resource alerts in quick succession
        let recent_resource: Vec<_> = self.event_buffer.iter()
            .filter(|e| e.kind == "system" && now - e.timestamp < 300.0)
            .filter(|e| e.summary.contains("cpu") || e.summary.contains("memory") || e.summary.contains("battery"))
            .collect();
        if recent_resource.len() >= 2 {
            triggers.push(OpportunityTrigger {
                kind: TriggerKind::ResourceAlert,
                source: "event_bus".into(),
                reason: format!("{} resource alerts in 5min", recent_resource.len()),
                suggested_message: String::new(),
                context: serde_json::json!({"alert_count": recent_resource.len()}),
                raw_urgency: 0.5,
                cooldown_key: "resource_cluster".into(),
                created_at: now,
            });
        }

        triggers
    }
}

// ── Stage 2: Generate ───────────────────────────────────────────────────────

/// A candidate intervention — what we *could* say to the user.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// The trigger that generated this candidate.
    pub trigger: OpportunityTrigger,
    /// The intervention type.
    pub intervention: InterventionType,
    /// Draft message text (before scoring decides delivery).
    pub draft_text: String,
    /// Whether this is a question (affects question budget).
    pub is_question: bool,
}

/// What type of intervention are we considering?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterventionType {
    /// Remind about something time-sensitive.
    Remind,
    /// Warn about a potential problem.
    Warn,
    /// Prepare something proactively (stage files, draft reply).
    Prepare,
    /// Summarize activity since last interaction.
    Summarize,
    /// Perform an automated action with audit trail.
    Automate,
    /// Stay silent — sometimes the best intervention is none.
    StaySilent,
}

/// Generator: converts opportunity triggers into candidate interventions.
pub struct Generator {
    templates: TemplateEngine,
    user_name: String,
}

impl Generator {
    pub fn new(user_name: &str) -> Self {
        Self {
            templates: TemplateEngine::new(),
            user_name: user_name.to_string(),
        }
    }

    /// Generate candidate interventions from a trigger.
    pub fn generate(
        &mut self,
        trigger: &OpportunityTrigger,
        bond_level: BondLevel,
    ) -> Vec<Candidate> {
        let mut candidates = Vec::new();

        // Always consider staying silent as an option
        candidates.push(Candidate {
            trigger: trigger.clone(),
            intervention: InterventionType::StaySilent,
            draft_text: String::new(),
            is_question: false,
        });

        // Generate the primary intervention based on trigger kind
        let draft = self.compose_draft(trigger, bond_level);
        if draft.is_empty() {
            return candidates;
        }

        let is_question = draft.ends_with('?');
        let intervention = match &trigger.kind {
            TriggerKind::CommitmentAlert => InterventionType::Remind,
            TriggerKind::ResourceAlert => InterventionType::Warn,
            TriggerKind::UserResumed => InterventionType::Summarize,
            TriggerKind::RoutineDeviation => InterventionType::Prepare,
            TriggerKind::Urge | TriggerKind::ExternalSignal => {
                // Classify based on urgency
                if trigger.raw_urgency >= 0.8 {
                    InterventionType::Remind
                } else if trigger.raw_urgency >= 0.5 {
                    InterventionType::Warn
                } else {
                    InterventionType::Summarize
                }
            }
            TriggerKind::DismissalSignal => return candidates, // No action for dismissals
        };

        candidates.push(Candidate {
            trigger: trigger.clone(),
            intervention,
            draft_text: draft,
            is_question,
        });

        candidates
    }

    /// Compose a draft message for a trigger using templates or fallback.
    fn compose_draft(
        &mut self,
        trigger: &OpportunityTrigger,
        bond_level: BondLevel,
    ) -> String {
        // Build data slots for template rendering
        let mut data = HashMap::new();
        data.insert("user".into(), self.user_name.clone());
        data.insert("reason".into(), trigger.reason.clone());
        if !trigger.suggested_message.is_empty() {
            data.insert("message".into(), trigger.suggested_message.clone());
        }

        // Extract slots from context JSON
        if let Some(obj) = trigger.context.as_object() {
            for (key, val) in obj {
                let s = match val {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    _ => val.to_string(),
                };
                if !s.is_empty() && s != "null" {
                    data.insert(key.clone(), s);
                }
            }
        }

        // Try template engine
        let instinct = trigger.source.to_lowercase();
        if let Some(rendered) = self.templates.render(&instinct, &data, bond_level) {
            return rendered;
        }

        // Fallback: use suggested message or reason
        if !trigger.suggested_message.is_empty() {
            trigger.suggested_message.clone()
        } else if !trigger.reason.is_empty() {
            trigger.reason.clone()
        } else {
            String::new()
        }
    }
}

// ── Stage 3: Score ──────────────────────────────────────────────────────────

/// Multi-axis score for a candidate intervention.
#[derive(Debug, Clone)]
pub struct CandidateScore {
    /// Overall score (weighted combination of axes).
    pub total: f64,
    /// Component scores for debugging/logging.
    pub urgency: f64,
    pub confidence: f64,
    pub interruptibility: f64,
    pub novelty: f64,
    pub expected_value: f64,
    pub annoyance_risk: f64,
    pub acceptance_rate: f64,
}

/// Scorer: evaluates candidates with multi-axis scoring.
pub struct Scorer {
    /// Weights for each scoring axis.
    weights: ScoringWeights,
}

/// Configurable weights for the scoring axes.
#[derive(Debug, Clone)]
pub struct ScoringWeights {
    pub urgency: f64,
    pub confidence: f64,
    pub interruptibility: f64,
    pub novelty: f64,
    pub expected_value: f64,
    pub annoyance_risk: f64,
    pub acceptance_rate: f64,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            urgency: 1.5,
            confidence: 1.0,
            interruptibility: 1.2,
            novelty: 0.8,
            expected_value: 1.0,
            annoyance_risk: -1.5, // Negative: high annoyance reduces score
            acceptance_rate: 1.3,
        }
    }
}

impl Scorer {
    pub fn new() -> Self {
        Self {
            weights: ScoringWeights::default(),
        }
    }

    pub fn with_weights(weights: ScoringWeights) -> Self {
        Self { weights }
    }

    /// Score a candidate intervention.
    pub fn score(
        &self,
        candidate: &Candidate,
        ctx: &ScoringContext,
    ) -> CandidateScore {
        // StaySilent always gets a baseline score
        if candidate.intervention == InterventionType::StaySilent {
            return CandidateScore {
                total: ctx.silence_baseline,
                urgency: 0.0,
                confidence: 1.0,
                interruptibility: 1.0,
                novelty: 0.0,
                expected_value: 0.0,
                annoyance_risk: 0.0,
                acceptance_rate: 1.0,
            };
        }

        let urgency = candidate.trigger.raw_urgency;
        let confidence = self.compute_confidence(candidate);
        let interruptibility = ctx.interruptibility;
        let novelty = self.compute_novelty(candidate, ctx);
        let expected_value = self.compute_expected_value(candidate);
        let annoyance_risk = self.compute_annoyance_risk(candidate, ctx);
        let acceptance_rate = self.get_acceptance_rate(candidate, ctx);

        let total = self.weights.urgency * urgency
            + self.weights.confidence * confidence
            + self.weights.interruptibility * interruptibility
            + self.weights.novelty * novelty
            + self.weights.expected_value * expected_value
            + self.weights.annoyance_risk * annoyance_risk
            + self.weights.acceptance_rate * acceptance_rate;

        CandidateScore {
            total,
            urgency,
            confidence,
            interruptibility,
            novelty,
            expected_value,
            annoyance_risk,
            acceptance_rate,
        }
    }

    fn compute_confidence(&self, candidate: &Candidate) -> f64 {
        // Higher confidence for triggers with concrete messages
        if !candidate.draft_text.is_empty() {
            0.8
        } else {
            0.3
        }
    }

    fn compute_novelty(&self, candidate: &Candidate, ctx: &ScoringContext) -> f64 {
        // Check if we recently sent something with the same cooldown key
        let key = &candidate.trigger.cooldown_key;
        if key.is_empty() {
            return 0.7; // Unknown novelty → moderate
        }
        if let Some(last) = ctx.recent_deliveries.get(key) {
            let hours_ago = (ctx.now - last) / 3600.0;
            // Novelty increases with time since last delivery
            (hours_ago / 24.0).min(1.0)
        } else {
            1.0 // Never delivered → fully novel
        }
    }

    fn compute_expected_value(&self, candidate: &Candidate) -> f64 {
        match candidate.intervention {
            InterventionType::Remind => 0.8,
            InterventionType::Warn => 0.9,
            InterventionType::Prepare => 0.7,
            InterventionType::Summarize => 0.5,
            InterventionType::Automate => 0.6,
            InterventionType::StaySilent => 0.0,
        }
    }

    fn compute_annoyance_risk(&self, candidate: &Candidate, ctx: &ScoringContext) -> f64 {
        let mut risk: f64 = 0.0;

        // High annoyance if we've been noisy recently
        if ctx.deliveries_last_hour >= 3 {
            risk += 0.5;
        } else if ctx.deliveries_last_hour >= 2 {
            risk += 0.3;
        }

        // Questions are more interruptive than statements
        if candidate.is_question {
            risk += 0.2;
        }

        // Low interruptibility = user is focused = higher annoyance risk
        if ctx.interruptibility < 0.3 {
            risk += 0.3;
        }

        risk.min(1.0)
    }

    fn get_acceptance_rate(&self, candidate: &Candidate, ctx: &ScoringContext) -> f64 {
        // Look up historical acceptance rate for this source instinct
        let source = &candidate.trigger.source;
        ctx.acceptance_rates.get(source).copied().unwrap_or(0.5)
    }
}

/// Context provided to the scorer from the broader system.
#[derive(Debug, Clone)]
pub struct ScoringContext {
    pub now: f64,
    /// 0.0 = user deeply focused, 1.0 = idle/available.
    pub interruptibility: f64,
    /// Number of proactive messages delivered in the last hour.
    pub deliveries_last_hour: u32,
    /// Map of cooldown_key → last delivery timestamp.
    pub recent_deliveries: HashMap<String, f64>,
    /// Map of instinct_name → acceptance rate (0.0-1.0).
    pub acceptance_rates: HashMap<String, f64>,
    /// Baseline score for staying silent (adapts over time).
    pub silence_baseline: f64,
}

impl Default for ScoringContext {
    fn default() -> Self {
        Self {
            now: now_ts(),
            interruptibility: 0.5,
            deliveries_last_hour: 0,
            recent_deliveries: HashMap::new(),
            acceptance_rates: HashMap::new(),
            silence_baseline: 0.5, // Start neutral — learn over time
        }
    }
}

// ── Stage 4: Deliver ────────────────────────────────────────────────────────

/// Delivery policy decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryDecision {
    /// Do nothing — silence is golden.
    Silent,
    /// Show as a subtle ambient signal (status bar glow, dot).
    AmbientSignal,
    /// Show as a whisper card (dismissable, non-blocking).
    WhisperCard,
    /// Show as a badge/counter on the companion avatar.
    Badge,
    /// Show on lock screen (important, time-sensitive).
    LockScreenCard,
    /// Queue for the next daily/evening summary.
    ScheduledSummary,
    /// Execute autonomously with audit trail (high trust only).
    AutonomousAction,
}

/// Delivery policy engine: decides how (or whether) to present an intervention.
pub struct DeliveryPolicy {
    /// Bond level affects delivery aggressiveness.
    bond_level: BondLevel,
    /// Minimum score threshold to deliver (below this → silent).
    delivery_threshold: f64,
    /// Score threshold for whisper card (above this → card, below → ambient).
    whisper_threshold: f64,
    /// Score threshold for lock screen (above this → lock screen).
    lock_screen_threshold: f64,
    /// Question budget: statements delivered today.
    statements_today: u32,
    /// Question budget: questions delivered today.
    questions_today: u32,
    daily_reset_ts: f64,
    /// Cooldown tracking.
    last_delivery_ts: f64,
}

impl DeliveryPolicy {
    pub fn new(bond_level: BondLevel) -> Self {
        Self {
            bond_level,
            delivery_threshold: 0.4,
            whisper_threshold: 0.7,
            lock_screen_threshold: 1.5,
            statements_today: 0,
            questions_today: 0,
            daily_reset_ts: 0.0,
            last_delivery_ts: 0.0,
        }
    }

    pub fn set_bond_level(&mut self, level: BondLevel) {
        self.bond_level = level;
    }

    /// Apply delivery policy to a scored candidate.
    pub fn decide(
        &mut self,
        candidate: &Candidate,
        score: &CandidateScore,
    ) -> DeliveryDecision {
        let now = now_ts();

        // Reset daily counters
        if now - self.daily_reset_ts > 86400.0 {
            self.statements_today = 0;
            self.questions_today = 0;
            self.daily_reset_ts = now;
        }

        // StaySilent candidate wins if its score is highest
        if candidate.intervention == InterventionType::StaySilent {
            return DeliveryDecision::Silent;
        }

        // Below threshold → silent
        if score.total < self.delivery_threshold {
            return DeliveryDecision::Silent;
        }

        // Question budget: 2:1 statement-to-question ratio
        if candidate.is_question && self.questions_today > 0
            && self.statements_today < self.questions_today * 2
        {
            return DeliveryDecision::ScheduledSummary; // Defer the question
        }

        // Bond-based cooldown
        let cooldown_secs = self.effective_cooldown_secs();
        if now - self.last_delivery_ts < cooldown_secs {
            // Still in cooldown — only urgent items break through
            if score.urgency < 0.8 {
                return DeliveryDecision::ScheduledSummary;
            }
        }

        // Decide delivery channel based on score
        let decision = if score.total >= self.lock_screen_threshold {
            DeliveryDecision::LockScreenCard
        } else if score.total >= self.whisper_threshold {
            DeliveryDecision::WhisperCard
        } else {
            DeliveryDecision::AmbientSignal
        };

        // Autonomous action requires very high urgency + automation type
        if candidate.intervention == InterventionType::Automate && score.urgency >= 0.9 {
            return DeliveryDecision::AutonomousAction;
        }

        decision
    }

    /// Record that a delivery happened, for cooldown and budget tracking.
    pub fn record_delivery(&mut self, is_question: bool) {
        self.last_delivery_ts = now_ts();
        if is_question {
            self.questions_today += 1;
        } else {
            self.statements_today += 1;
        }
    }

    fn effective_cooldown_secs(&self) -> f64 {
        match self.bond_level {
            BondLevel::Stranger => 120.0 * 60.0,
            BondLevel::Acquaintance => 90.0 * 60.0,
            BondLevel::Friend => 60.0 * 60.0,
            BondLevel::Confidant => 40.0 * 60.0,
            BondLevel::PartnerInCrime => 25.0 * 60.0,
        }
    }
}

// ── Full Pipeline ───────────────────────────────────────────────────────────

/// The complete 4-stage proactive pipeline.
pub struct ProactivePipeline {
    pub detector: Detector,
    pub generator: Generator,
    pub scorer: Scorer,
    pub delivery: DeliveryPolicy,
    config: ProactiveConfig,
    bond_level: BondLevel,
}

impl ProactivePipeline {
    pub fn new(config: ProactiveConfig, user_name: &str, bond_level: BondLevel) -> Self {
        Self {
            detector: Detector::new(),
            generator: Generator::new(user_name),
            scorer: Scorer::new(),
            delivery: DeliveryPolicy::new(bond_level),
            config,
            bond_level,
        }
    }

    pub fn set_bond_level(&mut self, level: BondLevel) {
        self.bond_level = level;
        self.delivery.set_bond_level(level);
    }

    /// Run the full pipeline: detect → generate → score → deliver.
    ///
    /// Returns Some(ProactiveMessage) if a delivery should happen.
    pub fn run(
        &mut self,
        urge_queue: &UrgeQueue,
        conn: &Connection,
        scoring_ctx: &ScoringContext,
    ) -> Option<PipelineResult> {
        if !self.config.enabled {
            return None;
        }

        // Stage 1: Detect — gather triggers from urges + events
        let mut triggers = Vec::new();

        // From existing urge queue (backward compatibility)
        let pending = urge_queue.get_pending(conn, 3); // Check top 3 urges
        for urge in &pending {
            if urge.urgency >= self.config.delivery_threshold {
                triggers.push(Detector::trigger_from_urge(urge));
            }
        }

        // From event patterns
        triggers.extend(self.detector.detect_from_events());

        if triggers.is_empty() {
            return None;
        }

        // Stage 2: Generate — produce candidate interventions
        let mut all_candidates = Vec::new();
        for trigger in &triggers {
            let candidates = self.generator.generate(trigger, self.bond_level);
            all_candidates.extend(candidates);
        }

        if all_candidates.is_empty() {
            return None;
        }

        // Stage 3: Score — evaluate each candidate
        let mut scored: Vec<(Candidate, CandidateScore)> = all_candidates
            .into_iter()
            .map(|c| {
                let score = self.scorer.score(&c, scoring_ctx);
                (c, score)
            })
            .collect();

        // Sort by total score descending
        scored.sort_by(|a, b| b.1.total.partial_cmp(&a.1.total).unwrap_or(std::cmp::Ordering::Equal));

        // Stage 4: Deliver — apply policy to best candidate
        let (best_candidate, best_score) = &scored[0];
        let decision = self.delivery.decide(best_candidate, best_score);

        // If silence wins, return suppression info
        if decision == DeliveryDecision::Silent {
            return Some(PipelineResult {
                decision,
                message: None,
                score: best_score.clone(),
                candidate_count: scored.len(),
                winning_source: best_candidate.trigger.source.clone(),
            });
        }

        // Pop the urge from queue if it came from urge system
        if best_candidate.trigger.kind == TriggerKind::Urge {
            let _ = urge_queue.pop_for_interaction(conn, 1);
        }

        let msg = ProactiveMessage {
            text: best_candidate.draft_text.clone(),
            urge_ids: vec![best_candidate.trigger.cooldown_key.clone()],
            generated_at: now_ts(),
        };

        self.delivery.record_delivery(best_candidate.is_question);

        Some(PipelineResult {
            decision,
            message: Some(msg),
            score: best_score.clone(),
            candidate_count: scored.len(),
            winning_source: best_candidate.trigger.source.clone(),
        })
    }
}

/// The result of running the pipeline.
#[derive(Debug, Clone)]
pub struct PipelineResult {
    /// What the delivery policy decided.
    pub decision: DeliveryDecision,
    /// The message to deliver (None if Silent).
    pub message: Option<ProactiveMessage>,
    /// The winning candidate's score breakdown.
    pub score: CandidateScore,
    /// How many candidates were evaluated.
    pub candidate_count: usize,
    /// Which instinct/source produced the winning candidate.
    pub winning_source: String,
}

impl PipelineResult {
    /// True if a message should actually be shown to the user.
    pub fn should_deliver(&self) -> bool {
        self.message.is_some() && self.decision != DeliveryDecision::Silent
    }
}

// ── Persistence: Delivery Log ───────────────────────────────────────────────

/// Tracks proactive delivery outcomes for learning.
pub struct DeliveryLog;

impl DeliveryLog {
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS proactive_deliveries (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                source          TEXT NOT NULL,
                intervention    TEXT NOT NULL,
                decision        TEXT NOT NULL,
                score_total     REAL NOT NULL,
                score_urgency   REAL NOT NULL,
                score_annoyance REAL NOT NULL,
                message_preview TEXT,
                outcome         TEXT NOT NULL DEFAULT 'pending',
                delivered_at    REAL NOT NULL,
                outcome_at      REAL
            );
            CREATE INDEX IF NOT EXISTS idx_delivery_source ON proactive_deliveries(source);
            CREATE INDEX IF NOT EXISTS idx_delivery_time ON proactive_deliveries(delivered_at);
            CREATE INDEX IF NOT EXISTS idx_delivery_outcome ON proactive_deliveries(outcome);",
        )
        .expect("failed to create proactive_deliveries table");
    }

    /// Record a delivery for outcome tracking.
    pub fn record(
        conn: &Connection,
        result: &PipelineResult,
    ) {
        let preview = result.message.as_ref()
            .map(|m| m.text.chars().take(200).collect::<String>())
            .unwrap_or_default();

        let _ = conn.execute(
            "INSERT INTO proactive_deliveries
             (source, intervention, decision, score_total, score_urgency, score_annoyance,
              message_preview, delivered_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                result.winning_source,
                format!("{:?}", result.decision),
                format!("{:?}", result.decision),
                result.score.total,
                result.score.urgency,
                result.score.annoyance_risk,
                preview,
                now_ts(),
            ],
        );
    }

    /// Record the outcome of a delivered message (accepted, dismissed, ignored).
    pub fn record_outcome(conn: &Connection, delivery_id: i64, outcome: &str) {
        let _ = conn.execute(
            "UPDATE proactive_deliveries SET outcome = ?1, outcome_at = ?2 WHERE id = ?3",
            params![outcome, now_ts(), delivery_id],
        );
    }

    /// Get acceptance rate per source instinct (last 30 days).
    pub fn acceptance_rates(conn: &Connection) -> HashMap<String, f64> {
        let since = now_ts() - 30.0 * 86400.0;
        let mut rates = HashMap::new();

        let mut stmt = match conn.prepare(
            "SELECT source,
                    SUM(CASE WHEN outcome = 'accepted' THEN 1 ELSE 0 END) as accepted,
                    COUNT(*) as total
             FROM proactive_deliveries
             WHERE delivered_at >= ?1 AND outcome != 'pending'
             GROUP BY source
             HAVING total >= 3",
        ) {
            Ok(s) => s,
            Err(_) => return rates,
        };

        if let Ok(rows) = stmt.query_map(params![since], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        }) {
            for row in rows.flatten() {
                let (source, accepted, total) = row;
                if total > 0 {
                    rates.insert(source, accepted as f64 / total as f64);
                }
            }
        }

        rates
    }

    /// Count deliveries in the last N seconds.
    pub fn count_recent(conn: &Connection, seconds: f64) -> u32 {
        let since = now_ts() - seconds;
        conn.query_row(
            "SELECT COUNT(*) FROM proactive_deliveries WHERE delivered_at >= ?1",
            params![since],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0) as u32
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        DeliveryLog::ensure_table(&conn);
        conn
    }

    #[test]
    fn detector_creates_trigger_from_urge() {
        let urge = crate::types::Urge {
            urge_id: "u1".into(),
            instinct_name: "check_in".into(),
            reason: "Haven't heard from user".into(),
            urgency: 0.7,
            suggested_message: "Hey, everything okay?".into(),
            action: None,
            context: serde_json::json!({}),
            cooldown_key: "check_in_1".into(),
            status: "pending".into(),
            created_at: 0.0,
            delivered_at: None,
            expires_at: None,
            boost_count: 0,
        };

        let trigger = Detector::trigger_from_urge(&urge);
        assert_eq!(trigger.kind, TriggerKind::Urge);
        assert_eq!(trigger.source, "check_in");
        assert!((trigger.raw_urgency - 0.7).abs() < 0.01);
    }

    #[test]
    fn generator_produces_candidates_including_silence() {
        let mut gen = Generator::new("Sync");
        let trigger = OpportunityTrigger {
            kind: TriggerKind::Urge,
            source: "check_in".into(),
            reason: "Idle for 2 hours".into(),
            suggested_message: "Hey, everything okay?".into(),
            context: serde_json::json!({}),
            raw_urgency: 0.6,
            cooldown_key: "check_in".into(),
            created_at: now_ts(),
        };

        let candidates = gen.generate(&trigger, BondLevel::Friend);
        // Should have at least StaySilent + one real candidate
        assert!(candidates.len() >= 2);
        assert!(candidates.iter().any(|c| c.intervention == InterventionType::StaySilent));
        assert!(candidates.iter().any(|c| !c.draft_text.is_empty()));
    }

    #[test]
    fn scorer_silence_gets_baseline_score() {
        let scorer = Scorer::new();
        let trigger = OpportunityTrigger {
            kind: TriggerKind::Urge,
            source: "test".into(),
            reason: "test".into(),
            suggested_message: String::new(),
            context: serde_json::json!({}),
            raw_urgency: 0.5,
            cooldown_key: "test".into(),
            created_at: now_ts(),
        };
        let candidate = Candidate {
            trigger,
            intervention: InterventionType::StaySilent,
            draft_text: String::new(),
            is_question: false,
        };

        let ctx = ScoringContext {
            silence_baseline: 0.5,
            ..Default::default()
        };
        let score = scorer.score(&candidate, &ctx);
        assert!((score.total - 0.5).abs() < 0.01);
    }

    #[test]
    fn scorer_high_urgency_beats_silence() {
        let scorer = Scorer::new();
        let trigger = OpportunityTrigger {
            kind: TriggerKind::CommitmentAlert,
            source: "commitment".into(),
            reason: "Meeting in 15 minutes".into(),
            suggested_message: "Your meeting starts in 15 minutes.".into(),
            context: serde_json::json!({}),
            raw_urgency: 0.9,
            cooldown_key: "commit_1".into(),
            created_at: now_ts(),
        };

        let silent = Candidate {
            trigger: trigger.clone(),
            intervention: InterventionType::StaySilent,
            draft_text: String::new(),
            is_question: false,
        };
        let remind = Candidate {
            trigger,
            intervention: InterventionType::Remind,
            draft_text: "Your meeting starts in 15 minutes.".into(),
            is_question: false,
        };

        let ctx = ScoringContext {
            silence_baseline: 0.5,
            interruptibility: 0.8, // user is available
            ..Default::default()
        };

        let silent_score = scorer.score(&silent, &ctx);
        let remind_score = scorer.score(&remind, &ctx);
        assert!(remind_score.total > silent_score.total,
            "Remind ({:.2}) should beat silence ({:.2})",
            remind_score.total, silent_score.total);
    }

    #[test]
    fn delivery_policy_respects_cooldown() {
        let mut policy = DeliveryPolicy::new(BondLevel::Friend);
        policy.last_delivery_ts = now_ts(); // Just delivered

        let trigger = OpportunityTrigger {
            kind: TriggerKind::Urge,
            source: "test".into(),
            reason: "test".into(),
            suggested_message: "Hey".into(),
            context: serde_json::json!({}),
            raw_urgency: 0.5,
            cooldown_key: "test".into(),
            created_at: now_ts(),
        };
        let candidate = Candidate {
            trigger,
            intervention: InterventionType::Summarize,
            draft_text: "Hey".into(),
            is_question: false,
        };
        let score = CandidateScore {
            total: 0.6,
            urgency: 0.5,
            confidence: 0.8,
            interruptibility: 0.5,
            novelty: 0.7,
            expected_value: 0.5,
            annoyance_risk: 0.2,
            acceptance_rate: 0.5,
        };

        let decision = policy.decide(&candidate, &score);
        // Should be deferred due to cooldown (urgency < 0.8)
        assert_eq!(decision, DeliveryDecision::ScheduledSummary);
    }

    #[test]
    fn delivery_log_tracks_outcomes() {
        let conn = setup();

        let result = PipelineResult {
            decision: DeliveryDecision::WhisperCard,
            message: Some(ProactiveMessage {
                text: "Hey Sync, checking in.".into(),
                urge_ids: vec!["u1".into()],
                generated_at: now_ts(),
            }),
            score: CandidateScore {
                total: 0.8,
                urgency: 0.6,
                confidence: 0.8,
                interruptibility: 0.7,
                novelty: 0.9,
                expected_value: 0.5,
                annoyance_risk: 0.1,
                acceptance_rate: 0.6,
            },
            candidate_count: 3,
            winning_source: "check_in".into(),
        };

        DeliveryLog::record(&conn, &result);

        let count = DeliveryLog::count_recent(&conn, 3600.0);
        assert_eq!(count, 1);

        // Record outcome
        DeliveryLog::record_outcome(&conn, 1, "accepted");

        let rates = DeliveryLog::acceptance_rates(&conn);
        // Need at least 3 deliveries for rate calculation
        assert!(rates.is_empty()); // Only 1 delivery, needs 3

        // Add more deliveries
        DeliveryLog::record(&conn, &result);
        DeliveryLog::record_outcome(&conn, 2, "accepted");
        DeliveryLog::record(&conn, &result);
        DeliveryLog::record_outcome(&conn, 3, "dismissed");

        let rates = DeliveryLog::acceptance_rates(&conn);
        assert!(rates.contains_key("check_in"));
        let rate = rates["check_in"];
        assert!((rate - 0.667).abs() < 0.01, "Expected ~0.667, got {rate}");
    }

    #[test]
    fn annoyance_risk_increases_with_frequency() {
        let scorer = Scorer::new();
        let trigger = OpportunityTrigger {
            kind: TriggerKind::Urge,
            source: "test".into(),
            reason: "test".into(),
            suggested_message: "Hey".into(),
            context: serde_json::json!({}),
            raw_urgency: 0.5,
            cooldown_key: "test".into(),
            created_at: now_ts(),
        };
        let candidate = Candidate {
            trigger,
            intervention: InterventionType::Summarize,
            draft_text: "Hey".into(),
            is_question: false,
        };

        let calm_ctx = ScoringContext {
            deliveries_last_hour: 0,
            interruptibility: 0.8,
            ..Default::default()
        };
        let noisy_ctx = ScoringContext {
            deliveries_last_hour: 4,
            interruptibility: 0.1, // user is focused
            ..Default::default()
        };

        let calm_score = scorer.score(&candidate, &calm_ctx);
        let noisy_score = scorer.score(&candidate, &noisy_ctx);

        assert!(calm_score.total > noisy_score.total,
            "Calm ({:.2}) should score higher than noisy ({:.2})",
            calm_score.total, noisy_score.total);
    }
}
