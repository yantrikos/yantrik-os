//! Core types for the companion agent.

use serde::{Deserialize, Serialize};

pub use crate::bond::BondLevel;
pub use yantrik_ml::ModelTier;

/// How time-sensitive an urge is. Determines selection tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimeSensitivity {
    /// Tier 0: Must fire this cycle. Scheduled tasks, due reminders, fired automations.
    Immediate = 0,
    /// Tier 1: Should fire within the next hour. Morning brief, time-sensitive goals.
    Today = 1,
    /// Tier 2: Should fire sometime today. Health pulse, news, growth check-ins.
    Soon = 2,
    /// Tier 3: Background/ambient. Humor, wonder, deep dives, philosophy.
    Ambient = 3,
}

impl TimeSensitivity {
    pub fn tier(&self) -> u8 {
        *self as u8
    }
}

/// Functional category for fairness tracking and token budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InstinctCategory {
    /// GoalKeeper, Scheduler, Reminder, Automation, MorningBrief, EmailWatch, etc.
    Anticipatory,
    /// CheckIn, HealthPulse, EnergyMap, NightOwl, CognitiveLoad
    Wellbeing,
    /// Humor, RelationshipRadar, ConnectionWeaver, ConversationalCallback, etc.
    Social,
    /// SkillForge, MentorMatch, FutureSelf, GrowthMirror, DecisionLab, MoneyMind
    Growth,
    /// WorldSense, TrendWatch, LocalPulse, Curiosity, WeatherWatch, etc.
    Awareness,
    /// EmotionalAwareness, EveningReflection, DreamKeeper, FollowUp, etc.
    Emotional,
    /// PatternBreaker, DevilsAdvocate, PhilosophyCompanion, MemoryWeaver, Cortex, etc.
    Meta,
}

/// How much user interaction an urge requires before acting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutonomyTier {
    /// Act silently in the background. User sees nothing unless they check.
    /// Example: memory consolidation, cache cleanup.
    SilentBackground,
    /// Show a notification/suggestion. User can ignore it.
    /// Example: "You might want to check your email" whisper card.
    NotifySuggestion,
    /// Ask permission before acting. Block until approved.
    /// Example: "Should I send this email?" with approve/deny buttons.
    AskPermission,
    /// Interrupt immediately — break focus mode for urgent items.
    /// Example: security alert, critical system failure.
    InterruptNow,
}

impl Default for AutonomyTier {
    fn default() -> Self {
        Self::NotifySuggestion
    }
}

/// An urge specification produced by an instinct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrgeSpec {
    pub instinct_name: String,
    pub reason: String,
    /// 0.0 (ignore) to 1.0 (act immediately).
    pub urgency: f64,
    pub suggested_message: String,
    pub action: Option<String>,
    pub context: serde_json::Value,
    /// Used for deduplication — if a pending urge has the same key, boost instead.
    pub cooldown_key: String,
    /// Selection tier: determines processing order and selection algorithm.
    pub time_sensitivity: TimeSensitivity,
    /// Functional category: determines fairness tracking and token budgets.
    pub category: InstinctCategory,
    /// If true, bypasses urgency threshold — always delivers (e.g. morning brief).
    pub guaranteed: bool,
    /// How much user interaction is needed before this urge acts.
    #[serde(default)]
    pub autonomy: AutonomyTier,
}

impl UrgeSpec {
    pub fn new(instinct_name: &str, reason: &str, urgency: f64) -> Self {
        Self {
            instinct_name: instinct_name.to_string(),
            reason: reason.to_string(),
            urgency,
            suggested_message: String::new(),
            action: None,
            context: serde_json::json!({}),
            cooldown_key: String::new(),
            time_sensitivity: crate::urge_defaults::default_time_sensitivity(instinct_name),
            category: crate::urge_defaults::default_category(instinct_name),
            guaranteed: false,
            autonomy: crate::urge_defaults::default_autonomy(instinct_name),
        }
    }

    pub fn with_cooldown(mut self, key: &str) -> Self {
        self.cooldown_key = key.to_string();
        self
    }

    pub fn with_context(mut self, ctx: serde_json::Value) -> Self {
        self.context = ctx;
        self
    }

    pub fn with_message(mut self, msg: &str) -> Self {
        self.suggested_message = msg.to_string();
        self
    }

    pub fn guaranteed(mut self) -> Self {
        self.guaranteed = true;
        self
    }

    pub fn with_time_sensitivity(mut self, ts: TimeSensitivity) -> Self {
        self.time_sensitivity = ts;
        self
    }

    pub fn with_category(mut self, cat: InstinctCategory) -> Self {
        self.category = cat;
        self
    }

    pub fn with_autonomy(mut self, tier: AutonomyTier) -> Self {
        self.autonomy = tier;
        self
    }
}

/// Read-only snapshot of companion state for instinct evaluation.
#[derive(Debug, Clone)]
pub struct CompanionState {
    pub last_interaction_ts: f64,
    pub current_ts: f64,
    pub session_active: bool,
    pub conversation_turn_count: usize,
    pub recent_valence_avg: Option<f64>,
    pub pending_triggers: Vec<serde_json::Value>,
    pub active_patterns: Vec<serde_json::Value>,
    pub open_conflicts_count: usize,
    pub memory_count: i64,
    pub config_user_name: String,
    // Bond state
    pub bond_level: BondLevel,
    pub bond_score: f64,
    // Evolution state
    pub formality: f64,
    pub opinions_count: usize,
    pub shared_references_count: usize,
    /// Whether bond level changed on the last interaction.
    pub bond_level_changed: bool,
    // Phase 2: Proactive Intelligence fields
    pub current_hour: u32,
    pub current_day_of_week: u32,
    pub idle_seconds: f64,
    pub interactions_last_hour: u32,
    pub workflow_hints: Vec<serde_json::Value>,
    pub maintenance_report: Vec<serde_json::Value>,
    // Natural Communication fields
    /// Recent significant events for aftermath instinct: (description, timestamp, reflected)
    pub recent_events: Vec<(String, f64, bool)>,
    /// Average user message length over last 5 messages (for conversational metabolism)
    pub avg_user_msg_length: f64,
    /// Number of proactive messages sent today
    pub daily_proactive_count: u32,
    /// Last N messages sent by companion (for anti-repetition)
    pub recent_sent_messages: Vec<String>,
    /// Suppressed urges log: (urge_key, reason, timestamp)
    pub suppressed_urges: Vec<(String, String, f64)>,
    /// User's known interests (from memory/preferences).
    pub user_interests: Vec<String>,
    /// User's location (city/region) for local relevance.
    pub user_location: String,
    // Open Loops Guardian
    /// Number of open life threads (open, stalled, overdue).
    pub open_loops_count: i64,
    /// Number of overdue commitments.
    pub overdue_commitment_count: usize,
    /// Number of pending attention items across all channels.
    pub pending_attention_count: i64,
    /// Model tier for prompt complexity scaling.
    pub model_tier: ModelTier,
}

/// Response from handle_message().
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    pub message: String,
    pub memories_recalled: usize,
    pub urges_delivered: Vec<String>,
    pub tool_calls_made: Vec<String>,
    /// True if the response came from the offline responder (LLM was unavailable).
    #[serde(default)]
    pub offline_mode: bool,
}

/// A stored urge row from SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Urge {
    pub urge_id: String,
    pub instinct_name: String,
    pub reason: String,
    pub urgency: f64,
    pub suggested_message: String,
    pub action: Option<String>,
    pub context: serde_json::Value,
    pub cooldown_key: String,
    pub status: String,
    pub created_at: f64,
    pub delivered_at: Option<f64>,
    pub expires_at: Option<f64>,
    pub boost_count: i64,
}

/// A proactive message generated by background cognition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProactiveMessage {
    pub text: String,
    pub urge_ids: Vec<String>,
    pub generated_at: f64,
}
