//! Cortex Playbook Engine — deterministic, multi-signal anticipatory workflows.
//!
//! Playbooks are pure Rust functions that evaluate cortex state and emit
//! `CortexAction`s. They bypass the instinct/urge queue entirely for
//! time-sensitive anticipatory actions.
//!
//! # Design Principles
//!
//! 1. **No LLM calls** — playbooks are deterministic pattern matching
//! 2. **Multi-signal evidence** — require 2+ corroborating entity signals
//! 3. **Conviction scoring** — Beta-distribution helpfulness tracking per playbook
//! 4. **Implicit learning** — learn from user behavior, never ask "was this helpful?"
//! 5. **Global rate limiting** — max 3 anticipatory actions per hour
//! 6. **Explainable** — every action carries a human-readable explanation

use std::collections::{HashMap, VecDeque};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::focus::FocusContext;
use super::rules::AttentionItem;

// ─── Constants ──────────────────────────────────────────────────────────────

/// Maximum anticipatory actions per hour (global rate limit).
const MAX_ACTIONS_PER_HOUR: usize = 3;

/// Default conviction threshold — playbook must exceed this to fire.
const DEFAULT_CONVICTION_THRESHOLD: f64 = 0.3;

// ─── Core Types ─────────────────────────────────────────────────────────────

/// Actions the cortex can take — ordered by intrusiveness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CortexAction {
    /// Passive notification — just inform the user.
    Notify {
        title: String,
        body: String,
        explanation: String,
        playbook_id: String,
    },
    /// Suggest a tool invocation — user decides whether to execute.
    SuggestTool {
        tool_name: String,
        args: serde_json::Value,
        explanation: String,
        playbook_id: String,
    },
    /// Queue a persistent task for later autonomous processing.
    QueueTask {
        description: String,
        playbook_id: String,
    },
}

/// Implicit outcome of a playbook action — detected from user behavior.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ImplicitOutcome {
    /// User acted on suggestion within 10 minutes.
    ActedQuickly,
    /// User acted within 1 hour (may or may not be related).
    ActedEventually,
    /// User dismissed the notification.
    Dismissed,
    /// No action within 2 hours.
    Ignored,
    /// User expressed negative feedback ("stop", "don't remind me").
    NegativeFeedback,
}

impl ImplicitOutcome {
    /// Reward value for Beta-distribution update.
    pub fn reward(&self) -> f64 {
        match self {
            Self::ActedQuickly => 1.0,
            Self::ActedEventually => 0.3,
            Self::Dismissed | Self::Ignored => 0.0,
            Self::NegativeFeedback => -1.0,
        }
    }
}

/// Read-only state snapshot available to playbook evaluation functions.
/// Cheap to construct from the cortex + companion state.
pub struct PlaybookState<'a> {
    /// Current cortex attention items (from rules + baselines + patterns).
    pub attention_items: &'a [AttentionItem],
    /// Current user focus (what app/file/activity).
    pub current_focus: Option<&'a FocusContext>,
    /// Current timestamp (Unix epoch seconds).
    pub now_ts: f64,
    /// Current hour of day (0-23, UTC).
    pub user_hour: u32,
    /// SQLite connection for querying entities, pulses, emails, calendar.
    pub conn: &'a Connection,
    /// User's bond level as u8 (for permission gating).
    pub bond_level: u8,
}

// ─── Playbook Definition ────────────────────────────────────────────────────

/// A registered playbook — evaluated every think cycle.
pub struct Playbook {
    /// Unique identifier (e.g., "meeting_prep", "vip_email").
    pub id: &'static str,
    /// Human-readable name.
    pub name: &'static str,
    /// The evaluation function — pure Rust, no LLM.
    pub evaluate: fn(&PlaybookState) -> Vec<CortexAction>,
    /// Minimum cooldown between firings (seconds).
    pub cooldown_secs: f64,
    /// Default conviction threshold (can be adjusted by learning).
    pub conviction_threshold: f64,
}

// ─── Conviction Score ───────────────────────────────────────────────────────

/// Per-playbook conviction tracking (Beta distribution).
/// Warm start: α=2, β=1 → 67% helpful rate (biased toward firing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybookConviction {
    /// Beta distribution α (success count + prior).
    pub alpha: f64,
    /// Beta distribution β (failure count + prior).
    pub beta: f64,
    /// Total number of times this playbook has fired.
    pub fire_count: u32,
    /// Consecutive ignores (amplifies threshold).
    pub ignore_streak: u32,
    /// Timestamp of last firing.
    pub last_fire_ts: f64,
    /// Context hash of last firing (for same-pattern dedup).
    pub last_context_hash: u64,
}

impl PlaybookConviction {
    /// Create a new conviction with warm start (biased toward engagement).
    pub fn warm_start() -> Self {
        Self {
            alpha: 2.0,
            beta: 1.0,
            fire_count: 0,
            ignore_streak: 0,
            last_fire_ts: 0.0,
            last_context_hash: 0,
        }
    }

    /// Current helpfulness rate (Beta distribution mean).
    pub fn helpful_rate(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Should this playbook fire given current receptivity?
    /// Factors: helpfulness × receptivity vs. threshold (amplified by ignore streak).
    pub fn should_fire(&self, receptivity: f64, base_threshold: f64) -> bool {
        let score = self.helpful_rate() * receptivity;
        // After ignores, require progressively higher score to fire
        let adjusted_threshold = base_threshold * (1.0 + 0.2 * self.ignore_streak as f64);
        score > adjusted_threshold
    }

    /// Combined conviction score (for logging/diagnostics).
    pub fn score(&self, receptivity: f64) -> f64 {
        self.helpful_rate() * receptivity
    }

    /// Update conviction based on observed outcome.
    pub fn record_outcome(&mut self, outcome: ImplicitOutcome) {
        match outcome {
            ImplicitOutcome::ActedQuickly => {
                self.alpha += 1.0;
                self.ignore_streak = 0;
            }
            ImplicitOutcome::ActedEventually => {
                self.alpha += 0.3;
                self.ignore_streak = 0;
            }
            ImplicitOutcome::Dismissed | ImplicitOutcome::Ignored => {
                self.beta += 1.0;
                self.ignore_streak += 1;
            }
            ImplicitOutcome::NegativeFeedback => {
                self.beta += 3.0; // Strong negative signal
                self.ignore_streak += 3;
            }
        }
        self.fire_count += 1;
    }

    /// Apply exponential decay so old outcomes fade over time.
    /// Called lazily before evaluation.
    pub fn decay(&mut self, dt_secs: f64) {
        if dt_secs <= 0.0 {
            return;
        }
        // τ = 7 days — half-life of conviction memory
        let tau = 7.0 * 86400.0;
        let factor = (-dt_secs / tau).exp();
        self.alpha = 1.0 + (self.alpha - 1.0) * factor;
        self.beta = 1.0 + (self.beta - 1.0) * factor;
    }
}

// ─── Playbook Engine ────────────────────────────────────────────────────────

/// The PlaybookEngine manages registered playbooks, tracks convictions,
/// enforces cooldowns and rate limits, and records outcomes.
pub struct PlaybookEngine {
    /// Registered playbooks.
    playbooks: Vec<Playbook>,
    /// Per-playbook conviction tracking (id → conviction).
    convictions: HashMap<String, PlaybookConviction>,
    /// Per-playbook cooldowns (id:context_hash → last_fire_ts).
    cooldowns: HashMap<String, f64>,
    /// Timestamps of recent actions (for global rate limiting).
    actions_this_hour: VecDeque<f64>,
    /// Pending actions awaiting outcome detection (playbook_id → sent_ts).
    pending_outcomes: HashMap<String, f64>,
    /// Playbook overrides — user can mute specific playbooks via conversation.
    overrides: HashMap<String, bool>, // playbook_id → enabled
    /// Last decay timestamp for lazy decay.
    last_decay_ts: f64,
}

impl PlaybookEngine {
    /// Create a new engine with no playbooks registered.
    pub fn new() -> Self {
        Self {
            playbooks: Vec::new(),
            convictions: HashMap::new(),
            cooldowns: HashMap::new(),
            actions_this_hour: VecDeque::new(),
            pending_outcomes: HashMap::new(),
            overrides: HashMap::new(),
            last_decay_ts: 0.0,
        }
    }

    /// Register a playbook.
    pub fn register(&mut self, playbook: Playbook) {
        self.playbooks.push(playbook);
    }

    /// Evaluate all playbooks against current state.
    /// Returns actions that pass conviction, cooldown, and rate limit gates.
    pub fn evaluate(
        &mut self,
        state: &PlaybookState,
        user_receptivity: f64,
    ) -> Vec<CortexAction> {
        let now = state.now_ts;

        // Lazy decay — apply once per evaluation
        if self.last_decay_ts > 0.0 {
            let dt = now - self.last_decay_ts;
            if dt > 60.0 {
                for conv in self.convictions.values_mut() {
                    conv.decay(dt);
                }
            }
        }
        self.last_decay_ts = now;

        // Global rate limit — prune old entries
        self.actions_this_hour.retain(|&ts| now - ts < 3600.0);
        if self.actions_this_hour.len() >= MAX_ACTIONS_PER_HOUR {
            tracing::debug!(
                count = self.actions_this_hour.len(),
                "Playbook global rate limit reached ({}/hour)",
                MAX_ACTIONS_PER_HOUR
            );
            return vec![];
        }

        let mut all_actions = Vec::new();

        for playbook in &self.playbooks {
            // Check user override (muted playbooks)
            if let Some(&enabled) = self.overrides.get(playbook.id) {
                if !enabled {
                    continue;
                }
            }

            // Per-playbook cooldown check
            // Use playbook.id as the base key; context hash is checked inside
            let base_cooldown_key = playbook.id.to_string();
            if let Some(&last_fire) = self.cooldowns.get(&base_cooldown_key) {
                if now - last_fire < playbook.cooldown_secs {
                    continue;
                }
            }

            // Evaluate the playbook
            let candidates = (playbook.evaluate)(state);
            if candidates.is_empty() {
                continue;
            }

            // Compute context hash for dedup (hash all action titles/bodies)
            let context_hash = {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                for action in &candidates {
                    match action {
                        CortexAction::Notify { title, body, .. } => {
                            title.hash(&mut hasher);
                            body.hash(&mut hasher);
                        }
                        CortexAction::SuggestTool { tool_name, args, .. } => {
                            tool_name.hash(&mut hasher);
                            args.to_string().hash(&mut hasher);
                        }
                        CortexAction::QueueTask { description, .. } => {
                            description.hash(&mut hasher);
                        }
                    }
                }
                hasher.finish()
            };

            // Check context-specific dedup
            let context_key = format!("{}:{:x}", playbook.id, context_hash);
            if let Some(&last_fire) = self.cooldowns.get(&context_key) {
                if now - last_fire < playbook.cooldown_secs {
                    continue;
                }
            }

            // Conviction check
            let conviction = self.convictions
                .entry(playbook.id.to_string())
                .or_insert_with(PlaybookConviction::warm_start);

            // Skip if same context hash as last firing
            if conviction.last_context_hash == context_hash
                && conviction.last_fire_ts > 0.0
                && now - conviction.last_fire_ts < playbook.cooldown_secs
            {
                continue;
            }

            if !conviction.should_fire(user_receptivity, playbook.conviction_threshold) {
                tracing::debug!(
                    playbook = playbook.id,
                    conviction = %format!("{:.3}", conviction.score(user_receptivity)),
                    helpful_rate = %format!("{:.3}", conviction.helpful_rate()),
                    ignore_streak = conviction.ignore_streak,
                    threshold = %format!("{:.3}", playbook.conviction_threshold),
                    "Playbook suppressed by conviction"
                );
                continue;
            }

            // Check global rate limit again (in case multiple playbooks fire)
            if self.actions_this_hour.len() + candidates.len() > MAX_ACTIONS_PER_HOUR {
                tracing::debug!(
                    playbook = playbook.id,
                    "Playbook skipped — would exceed global rate limit"
                );
                continue;
            }

            // All gates passed — emit actions
            tracing::info!(
                playbook = playbook.id,
                action_count = candidates.len(),
                conviction = %format!("{:.3}", conviction.score(user_receptivity)),
                helpful_rate = %format!("{:.3}", conviction.helpful_rate()),
                fire_count = conviction.fire_count,
                "Playbook fired"
            );

            // Update tracking state
            conviction.last_fire_ts = now;
            conviction.last_context_hash = context_hash;
            self.cooldowns.insert(base_cooldown_key, now);
            self.cooldowns.insert(context_key, now);

            // Track for outcome detection
            self.pending_outcomes.insert(playbook.id.to_string(), now);

            for action in &candidates {
                self.actions_this_hour.push_back(now);
            }
            all_actions.extend(candidates);
        }

        all_actions
    }

    /// Record an implicit outcome for a playbook.
    /// Called from the bridge when user behavior is detected.
    pub fn record_outcome(&mut self, playbook_id: &str, outcome: ImplicitOutcome) {
        if let Some(conv) = self.convictions.get_mut(playbook_id) {
            conv.record_outcome(outcome);
            tracing::info!(
                playbook = playbook_id,
                outcome = ?outcome,
                helpful_rate = %format!("{:.3}", conv.helpful_rate()),
                ignore_streak = conv.ignore_streak,
                "Playbook outcome recorded"
            );
        }
        self.pending_outcomes.remove(playbook_id);
    }

    /// Check for timed-out pending outcomes (2h without response = Ignored).
    /// Called every think cycle.
    pub fn check_timeouts(&mut self, now_ts: f64) {
        let expired: Vec<String> = self.pending_outcomes
            .iter()
            .filter(|(_, &ts)| now_ts - ts > 7200.0) // 2 hours
            .map(|(id, _)| id.clone())
            .collect();

        for id in expired {
            self.record_outcome(&id, ImplicitOutcome::Ignored);
        }
    }

    /// Mute or unmute a playbook (via user conversation).
    pub fn set_override(&mut self, playbook_id: &str, enabled: bool) {
        self.overrides.insert(playbook_id.to_string(), enabled);
        tracing::info!(playbook = playbook_id, enabled, "Playbook override set");
    }

    /// Get diagnostic summary of all playbooks.
    pub fn diagnostic_summary(&self) -> String {
        let mut lines = Vec::new();
        for pb in &self.playbooks {
            let conv = self.convictions.get(pb.id);
            let enabled = self.overrides.get(pb.id).copied().unwrap_or(true);
            let rate = conv.map(|c| c.helpful_rate()).unwrap_or(0.667);
            let fires = conv.map(|c| c.fire_count).unwrap_or(0);
            let streak = conv.map(|c| c.ignore_streak).unwrap_or(0);
            let status = if enabled { "ON" } else { "MUTED" };
            lines.push(format!(
                "  {} [{}]: helpful={:.2} fires={} streak={} cooldown={}s",
                pb.name, status, rate, fires, streak, pb.cooldown_secs as u32
            ));
        }
        format!("Playbooks ({}):\n{}", self.playbooks.len(), lines.join("\n"))
    }

    // ─── SQLite Persistence ─────────────────────────────────────────────

    /// Initialize playbook tables.
    pub fn init_db(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS playbook_convictions (
                playbook_id TEXT PRIMARY KEY,
                alpha REAL NOT NULL DEFAULT 2.0,
                beta REAL NOT NULL DEFAULT 1.0,
                fire_count INTEGER NOT NULL DEFAULT 0,
                last_fire_ts REAL NOT NULL DEFAULT 0,
                ignore_streak INTEGER NOT NULL DEFAULT 0,
                last_context_hash INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS playbook_audit_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                playbook_id TEXT NOT NULL,
                fired_ts REAL NOT NULL,
                action_type TEXT NOT NULL,
                explanation TEXT NOT NULL,
                conviction REAL NOT NULL,
                context_hash INTEGER NOT NULL DEFAULT 0,
                outcome TEXT,
                outcome_ts REAL
            );

            CREATE TABLE IF NOT EXISTS playbook_overrides (
                playbook_id TEXT PRIMARY KEY,
                enabled INTEGER NOT NULL DEFAULT 1,
                user_note TEXT
            );"
        ).ok();
    }

    /// Save conviction state to SQLite.
    pub fn save(&self, conn: &Connection) {
        for (id, conv) in &self.convictions {
            conn.execute(
                "INSERT INTO playbook_convictions (playbook_id, alpha, beta, fire_count, last_fire_ts, ignore_streak, last_context_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(playbook_id) DO UPDATE SET
                    alpha = ?2, beta = ?3, fire_count = ?4, last_fire_ts = ?5,
                    ignore_streak = ?6, last_context_hash = ?7",
                rusqlite::params![
                    id, conv.alpha, conv.beta, conv.fire_count,
                    conv.last_fire_ts, conv.ignore_streak,
                    conv.last_context_hash as i64,
                ],
            ).ok();
        }
        for (id, &enabled) in &self.overrides {
            conn.execute(
                "INSERT INTO playbook_overrides (playbook_id, enabled)
                 VALUES (?1, ?2)
                 ON CONFLICT(playbook_id) DO UPDATE SET enabled = ?2",
                rusqlite::params![id, enabled as i32],
            ).ok();
        }
    }

    /// Load conviction state from SQLite.
    pub fn load(&mut self, conn: &Connection) {
        // Load convictions
        if let Ok(mut stmt) = conn.prepare(
            "SELECT playbook_id, alpha, beta, fire_count, last_fire_ts, ignore_streak, last_context_hash
             FROM playbook_convictions"
        ) {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, u32>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            }) {
                for row in rows.flatten() {
                    self.convictions.insert(row.0, PlaybookConviction {
                        alpha: row.1,
                        beta: row.2,
                        fire_count: row.3,
                        last_fire_ts: row.4,
                        ignore_streak: row.5,
                        last_context_hash: row.6 as u64,
                    });
                }
            }
        }

        // Load overrides
        if let Ok(mut stmt) = conn.prepare(
            "SELECT playbook_id, enabled FROM playbook_overrides"
        ) {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i32>(1)?,
                ))
            }) {
                for row in rows.flatten() {
                    self.overrides.insert(row.0, row.1 != 0);
                }
            }
        }
    }

    /// Log an action to the audit log (for transparency and debugging).
    pub fn audit_log(
        conn: &Connection,
        playbook_id: &str,
        fired_ts: f64,
        action_type: &str,
        explanation: &str,
        conviction: f64,
        context_hash: u64,
    ) {
        conn.execute(
            "INSERT INTO playbook_audit_log (playbook_id, fired_ts, action_type, explanation, conviction, context_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![playbook_id, fired_ts, action_type, explanation, conviction, context_hash as i64],
        ).ok();
    }
}

// ─── Playbook Registry ──────────────────────────────────────────────────────

/// Register all built-in playbooks.
pub fn register_default_playbooks(engine: &mut PlaybookEngine) {
    engine.register(Playbook {
        id: "meeting_prep",
        name: "Meeting Prep",
        evaluate: playbooks::meeting_prep::evaluate,
        cooldown_secs: 900.0,  // 15 minutes
        conviction_threshold: DEFAULT_CONVICTION_THRESHOLD,
    });

    engine.register(Playbook {
        id: "vip_email",
        name: "VIP Email Escalation",
        evaluate: playbooks::vip_email::evaluate,
        cooldown_secs: 7200.0, // 2 hours
        conviction_threshold: DEFAULT_CONVICTION_THRESHOLD,
    });

    engine.register(Playbook {
        id: "context_recovery",
        name: "Context Recovery",
        evaluate: playbooks::context_recovery::evaluate,
        cooldown_secs: 1800.0, // 30 minutes
        conviction_threshold: DEFAULT_CONVICTION_THRESHOLD,
    });

    tracing::info!(
        count = engine.playbooks.len(),
        "Playbook engine initialized"
    );
}

// ─── Playbook Implementations ───────────────────────────────────────────────

#[path = "playbooks"]
pub mod playbooks {
    pub mod meeting_prep;
    pub mod vip_email;
    pub mod context_recovery;
}
