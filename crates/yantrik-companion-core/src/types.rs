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
    /// A safety signal: crisis language, physical danger, a security warning, or a warning
    /// before an irreversible mistake. Set only by the source that detects it, never by a model.
    /// No throttle, backoff or "Less of this" ever holds it back (#754).
    #[serde(default)]
    pub safety: bool,
    /// The memories this urge was made from, so "Don't use this memory" can exclude them from
    /// every later proactive message (#754). Empty when it came from no memory.
    #[serde(default)]
    pub memory_ids: Vec<String>,
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
            safety: false,
            memory_ids: Vec::new(),
        }
    }

    /// Mark this urge a safety signal (see [`UrgeSpec::safety`]).
    pub fn safety(mut self) -> Self {
        self.safety = true;
        self
    }

    /// The memories this urge was made from (see [`UrgeSpec::memory_ids`]).
    pub fn from_memories<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for id in ids {
            let id = id.into();
            if !id.is_empty() && !self.memory_ids.contains(&id) {
                self.memory_ids.push(id);
            }
        }
        self
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

impl CompanionState {
    /// How long the person has been away, in seconds — or `None` if they have
    /// never been here.
    ///
    /// Shorthand for `absence_seconds(self.last_interaction_ts, self.current_ts)`;
    /// every reader that measures absence should go through it.
    pub fn absence_seconds(&self) -> Option<f64> {
        absence_seconds(self.last_interaction_ts, self.current_ts)
    }
}

/// How long the person has been away, in seconds, measured at `now` — or `None`
/// if there is no absence to measure.
///
/// The bond store's last-interaction clock stays unset (`<= 0.0`) until the first
/// scored conversation turn (#156: it used to be stamped with the boot time, and
/// a store nobody ever wrote to loads nothing). An unset clock must not read as
/// "away since 1970": you cannot be away from someone you have never met. Every
/// absence-driven behavior — the check-in, memory-weaver and curiosity gates,
/// idle logs, the status endpoint — has to stay quiet on `None` and wait for the
/// person's first turn to give the clock a real value.
pub fn absence_seconds(last_interaction_ts: f64, now: f64) -> Option<f64> {
    (last_interaction_ts > 0.0).then_some(now - last_interaction_ts)
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
    /// A safety signal, never throttled (see [`UrgeSpec::safety`]).
    #[serde(default)]
    pub safety: bool,
    /// The memories it was made from (see [`UrgeSpec::memory_ids`]).
    #[serde(default)]
    pub memory_ids: Vec<String>,
}

/// A proactive message generated by background cognition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProactiveMessage {
    pub text: String,
    pub urge_ids: Vec<String>,
    pub generated_at: f64,
    /// The kind of message, as `proactive_backoff::ProactiveKind::key` names it (#754). Filled
    /// from the first urge id when the message is queued, if the maker left it empty.
    #[serde(default)]
    pub kind: String,
    /// Why it appeared, in the person's words, from fixed templates (`proactive_backoff::why_text`),
    /// never from a model. Filled when the message is queued, if the maker left it empty.
    #[serde(default)]
    pub why: String,
    /// A safety signal (#754): crisis language, physical danger, a security warning, or a warning
    /// before an irreversible mistake. Set only by the source that detected it. No throttle, Do Not
    /// Disturb hold or kind level holds it back, and it offers no "Less of this".
    #[serde(default)]
    pub safety: bool,
    /// The memories this message was made from (#754), so "Don't use this memory" can exclude
    /// them from every later proactive message. Empty when it came from no memory, and then that
    /// choice is not offered.
    #[serde(default)]
    pub memory_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::absence_seconds;

    /// A plausible "now" (September 2026): read as an absence, an unset clock
    /// would be this many seconds since the epoch — the fifty-odd years no
    /// reader may report (#156).
    const NOW: f64 = 1_790_000_000.0;

    #[test]
    fn an_unset_clock_reports_no_absence_not_five_decades() {
        assert_eq!(absence_seconds(0.0, NOW), None);
    }

    #[test]
    fn a_real_last_interaction_measures_the_absence_from_it() {
        let two_days = 2.0 * 86400.0;
        assert_eq!(absence_seconds(NOW - two_days, NOW), Some(two_days));
    }
}
