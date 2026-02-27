//! Configuration for the companion agent.
//!
//! Mirrors the Python `CompanionConfig` Pydantic model.
//! Supports YAML deserialization.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalityConfig {
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
}

fn default_name() -> String {
    "Yantrik".to_string()
}

fn default_system_prompt() -> String {
    "You are Yantrik, a thoughtful personal companion with real memory.".to_string()
}

impl Default for PersonalityConfig {
    fn default() -> Self {
        Self {
            name: default_name(),
            system_prompt: default_system_prompt(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMConfig {
    /// Path to GGUF model file (for in-process inference).
    #[serde(default)]
    pub gguf_path: Option<String>,
    /// Path to tokenizer.json.
    #[serde(default)]
    pub tokenizer_path: Option<String>,
    /// Directory containing *.gguf + tokenizer.json.
    #[serde(default)]
    pub model_dir: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
}

fn default_max_tokens() -> usize {
    256
}
fn default_temperature() -> f64 {
    0.7
}
fn default_max_context_tokens() -> usize {
    1024
}

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            gguf_path: None,
            tokenizer_path: None,
            model_dir: None,
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            max_context_tokens: default_max_context_tokens(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}
fn default_port() -> u16 {
    8340
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YantrikDBConfig {
    #[serde(default = "default_db_path")]
    pub db_path: String,
    #[serde(default = "default_embedding_dim")]
    pub embedding_dim: usize,
    /// Path to embedder model directory (MiniLM safetensors).
    #[serde(default)]
    pub embedder_model_dir: Option<String>,
}

fn default_db_path() -> String {
    "memory.db".to_string()
}
fn default_embedding_dim() -> usize {
    384
}

impl Default for YantrikDBConfig {
    fn default() -> Self {
        Self {
            db_path: default_db_path(),
            embedding_dim: default_embedding_dim(),
            embedder_model_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationConfig {
    #[serde(default = "default_max_history_turns")]
    pub max_history_turns: usize,
    #[serde(default = "default_session_timeout_minutes")]
    pub session_timeout_minutes: u64,
}

fn default_max_history_turns() -> usize {
    10
}
fn default_session_timeout_minutes() -> u64 {
    30
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            max_history_turns: default_max_history_turns(),
            session_timeout_minutes: default_session_timeout_minutes(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: usize,
}

fn default_true() -> bool {
    // Disabled by default — 0.5B model can't do tool calling reliably.
    // Enable for larger models (7B+) via config.
    false
}
fn default_max_tool_rounds() -> usize {
    3
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            max_tool_rounds: default_max_tool_rounds(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitionConfig {
    #[serde(default = "default_think_interval")]
    pub think_interval_minutes: u64,
    #[serde(default = "default_think_active")]
    pub think_interval_active_minutes: u64,
    #[serde(default = "default_think_idle")]
    pub idle_think_interval_minutes: u64,
    #[serde(default = "default_proactive_threshold")]
    pub proactive_urgency_threshold: f64,
}

fn default_think_interval() -> u64 {
    15
}
fn default_think_active() -> u64 {
    5
}
fn default_think_idle() -> u64 {
    30
}
fn default_proactive_threshold() -> f64 {
    0.7
}

impl Default for CognitionConfig {
    fn default() -> Self {
        Self {
            think_interval_minutes: default_think_interval(),
            think_interval_active_minutes: default_think_active(),
            idle_think_interval_minutes: default_think_idle(),
            proactive_urgency_threshold: default_proactive_threshold(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstinctSettings {
    #[serde(default = "default_true")]
    pub check_in_enabled: bool,
    #[serde(default = "default_check_in_hours")]
    pub check_in_hours: f64,
    #[serde(default = "default_true")]
    pub emotional_awareness_enabled: bool,
    #[serde(default = "default_true")]
    pub follow_up_enabled: bool,
    #[serde(default = "default_follow_up_hours")]
    pub follow_up_min_hours: f64,
    #[serde(default = "default_true")]
    pub reminder_enabled: bool,
    #[serde(default = "default_true")]
    pub pattern_surfacing_enabled: bool,
    #[serde(default = "default_true")]
    pub conflict_alerting_enabled: bool,
    #[serde(default = "default_conflict_threshold")]
    pub conflict_alert_threshold: usize,
}

fn default_check_in_hours() -> f64 {
    8.0
}
fn default_follow_up_hours() -> f64 {
    4.0
}
fn default_conflict_threshold() -> usize {
    5
}

impl Default for InstinctSettings {
    fn default() -> Self {
        Self {
            check_in_enabled: true,
            check_in_hours: default_check_in_hours(),
            emotional_awareness_enabled: true,
            follow_up_enabled: true,
            follow_up_min_hours: default_follow_up_hours(),
            reminder_enabled: true,
            pattern_surfacing_enabled: true,
            conflict_alerting_enabled: true,
            conflict_alert_threshold: default_conflict_threshold(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrgeQueueConfig {
    #[serde(default = "default_expiry_hours")]
    pub expiry_hours: f64,
    #[serde(default = "default_max_pending")]
    pub max_pending: usize,
    #[serde(default = "default_boost")]
    pub boost_increment: f64,
}

fn default_expiry_hours() -> f64 {
    48.0
}
fn default_max_pending() -> usize {
    20
}
fn default_boost() -> f64 {
    0.1
}

impl Default for UrgeQueueConfig {
    fn default() -> Self {
        Self {
            expiry_hours: default_expiry_hours(),
            max_pending: default_max_pending(),
            boost_increment: default_boost(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BondConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for BondConfig {
    fn default() -> Self {
        Self {
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionConfig {
    #[serde(default = "default_formality_alpha")]
    pub formality_alpha: f64,
    #[serde(default = "default_opinion_threshold")]
    pub opinion_threshold: usize,
}

fn default_formality_alpha() -> f64 {
    0.05
}
fn default_opinion_threshold() -> usize {
    3
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self {
            formality_alpha: default_formality_alpha(),
            opinion_threshold: default_opinion_threshold(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeConfig {
    #[serde(default = "default_narrative_interval")]
    pub update_interval_interactions: usize,
    #[serde(default = "default_narrative_max_tokens")]
    pub max_tokens: usize,
}

fn default_narrative_interval() -> usize {
    10
}
fn default_narrative_max_tokens() -> usize {
    300
}

impl Default for NarrativeConfig {
    fn default() -> Self {
        Self {
            update_interval_interactions: default_narrative_interval(),
            max_tokens: default_narrative_max_tokens(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Whisper model ID for HuggingFace Hub download.
    #[serde(default = "default_whisper_model")]
    pub whisper_model: String,
    /// Local directory override for Whisper model files.
    #[serde(default)]
    pub whisper_model_dir: Option<String>,
    /// Local path to Piper voice config (.onnx.json).
    #[serde(default)]
    pub piper_config_path: Option<String>,
    /// Piper voice name for HuggingFace Hub download (e.g. "en_US-lessac-medium").
    #[serde(default = "default_piper_voice")]
    pub piper_voice: String,
    /// VAD silence threshold (RMS energy).
    #[serde(default = "default_silence_threshold")]
    pub silence_threshold: f32,
    /// Milliseconds of silence before end-of-speech.
    #[serde(default = "default_silence_duration_ms")]
    pub silence_duration_ms: u64,
}

fn default_whisper_model() -> String {
    "openai/whisper-tiny".to_string()
}
fn default_piper_voice() -> String {
    "en_US-lessac-medium".to_string()
}
fn default_silence_threshold() -> f32 {
    0.01
}
fn default_silence_duration_ms() -> u64 {
    800
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            whisper_model: default_whisper_model(),
            whisper_model_dir: None,
            piper_config_path: None,
            piper_voice: default_piper_voice(),
            silence_threshold: default_silence_threshold(),
            silence_duration_ms: default_silence_duration_ms(),
        }
    }
}

/// Top-level companion configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionConfig {
    #[serde(default = "default_user_name")]
    pub user_name: String,
    #[serde(default)]
    pub personality: PersonalityConfig,
    #[serde(default)]
    pub llm: LLMConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub yantrikdb: YantrikDBConfig,
    #[serde(default)]
    pub conversation: ConversationConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub cognition: CognitionConfig,
    #[serde(default)]
    pub instincts: InstinctSettings,
    #[serde(default)]
    pub urges: UrgeQueueConfig,
    #[serde(default)]
    pub bond: BondConfig,
    #[serde(default)]
    pub evolution: EvolutionConfig,
    #[serde(default)]
    pub narrative: NarrativeConfig,
    #[serde(default)]
    pub voice: VoiceConfig,
}

fn default_user_name() -> String {
    "User".to_string()
}

impl Default for CompanionConfig {
    fn default() -> Self {
        Self {
            user_name: default_user_name(),
            personality: PersonalityConfig::default(),
            llm: LLMConfig::default(),
            server: ServerConfig::default(),
            yantrikdb: YantrikDBConfig::default(),
            conversation: ConversationConfig::default(),
            tools: ToolsConfig::default(),
            cognition: CognitionConfig::default(),
            instincts: InstinctSettings::default(),
            urges: UrgeQueueConfig::default(),
            bond: BondConfig::default(),
            evolution: EvolutionConfig::default(),
            narrative: NarrativeConfig::default(),
            voice: VoiceConfig::default(),
        }
    }
}

impl CompanionConfig {
    /// Load from a YAML file.
    pub fn from_yaml(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = serde_yaml::from_str(&content)?;
        Ok(config)
    }
}
