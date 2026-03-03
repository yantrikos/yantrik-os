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
    /// LLM backend / provider name.
    ///
    /// In-process backends:
    ///   - `"candle"` — Candle GGUF (default, GPU via CUDA/Metal)
    ///   - `"llamacpp"` — llama.cpp via llama-cpp-2 crate
    ///
    /// API backends (OpenAI-compatible — set api_model, optionally api_key):
    ///   - `"ollama"` — Ollama (default: http://localhost:11434/v1)
    ///   - `"openai"` — OpenAI (default: https://api.openai.com/v1)
    ///   - `"deepseek"` — DeepSeek (default: https://api.deepseek.com)
    ///   - `"claude"` — Anthropic Claude via OpenAI-compat proxy
    ///   - `"vllm"` — vLLM server (set api_base_url)
    ///   - `"api"` — generic OpenAI-compatible (set api_base_url)
    #[serde(default = "default_backend")]
    pub backend: String,
    /// Base URL override for API backends.
    /// If omitted, uses the provider's default URL.
    #[serde(default)]
    pub api_base_url: Option<String>,
    /// Model name for API backends (e.g. "qwen2.5:3b-instruct", "gpt-4o", "deepseek-chat").
    #[serde(default)]
    pub api_model: Option<String>,
    /// API key (required for OpenAI/Claude/DeepSeek, optional for Ollama/vLLM).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Path to GGUF model file (for in-process inference).
    #[serde(default)]
    pub gguf_path: Option<String>,
    /// Path to tokenizer.json.
    #[serde(default)]
    pub tokenizer_path: Option<String>,
    /// Directory containing *.gguf + tokenizer.json.
    #[serde(default)]
    pub model_dir: Option<String>,
    /// HuggingFace Hub repo for GGUF model (e.g. "Qwen/Qwen2.5-3B-Instruct-GGUF").
    #[serde(default = "default_hub_repo")]
    pub hub_repo: String,
    /// GGUF filename within the hub repo.
    #[serde(default = "default_hub_gguf")]
    pub hub_gguf: String,
    /// HuggingFace Hub repo for tokenizer (e.g. "Qwen/Qwen2.5-3B-Instruct").
    #[serde(default = "default_hub_tokenizer")]
    pub hub_tokenizer: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
}

fn default_backend() -> String {
    "candle".to_string()
}
fn default_hub_repo() -> String {
    "Qwen/Qwen2.5-0.5B-Instruct-GGUF".to_string()
}
fn default_hub_gguf() -> String {
    "qwen2.5-0.5b-instruct-q4_k_m.gguf".to_string()
}
fn default_hub_tokenizer() -> String {
    "Qwen/Qwen2.5-0.5B-Instruct".to_string()
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
            backend: default_backend(),
            api_base_url: None,
            api_model: None,
            api_key: None,
            gguf_path: None,
            tokenizer_path: None,
            model_dir: None,
            hub_repo: default_hub_repo(),
            hub_gguf: default_hub_gguf(),
            hub_tokenizer: default_hub_tokenizer(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            max_context_tokens: default_max_context_tokens(),
        }
    }
}

impl LLMConfig {
    /// Returns true if this backend uses an external API (not in-process).
    pub fn is_api_backend(&self) -> bool {
        !matches!(self.backend.as_str(), "candle" | "llamacpp")
    }

    /// Resolve the API base URL from the backend/provider name.
    /// Uses `api_base_url` if set, otherwise returns the provider's default.
    pub fn resolve_api_base_url(&self) -> Option<String> {
        if let Some(ref url) = self.api_base_url {
            return Some(url.clone());
        }
        match self.backend.as_str() {
            "ollama" => Some("http://localhost:11434/v1".to_string()),
            "openai" => Some("https://api.openai.com/v1".to_string()),
            "deepseek" => Some("https://api.deepseek.com".to_string()),
            "claude" => Some("https://api.anthropic.com/v1".to_string()),
            "api" | "vllm" => None, // must be set explicitly
            _ => None,
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
    /// Maximum tool permission level: "safe", "standard", "sensitive", "dangerous".
    #[serde(default = "default_max_permission")]
    pub max_permission: String,
}

fn default_true() -> bool {
    // Disabled by default — 0.5B model can't do tool calling reliably.
    // Enable for larger models (7B+) via config.
    false
}
fn default_max_tool_rounds() -> usize {
    3
}
fn default_max_permission() -> String {
    "sensitive".to_string()
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            max_tool_rounds: default_max_tool_rounds(),
            max_permission: default_max_permission(),
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
    0.4
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
    /// Enable memory weaver instinct (proactive graph building during idle).
    #[serde(default = "default_evolution_enabled")]
    pub memory_weaver_enabled: bool,
    /// Minutes of idle time before weaver urges start firing.
    #[serde(default = "default_weaver_idle_minutes")]
    pub memory_weaver_idle_minutes: f64,
    /// Minimum memories before weaving is worthwhile.
    #[serde(default = "default_weaver_min_memories")]
    pub memory_weaver_min_memories: i64,
}

fn default_check_in_hours() -> f64 {
    2.0
}
fn default_follow_up_hours() -> f64 {
    4.0
}
fn default_conflict_threshold() -> usize {
    5
}
fn default_weaver_idle_minutes() -> f64 {
    15.0
}
fn default_weaver_min_memories() -> i64 {
    10
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
            memory_weaver_enabled: true,
            memory_weaver_idle_minutes: default_weaver_idle_minutes(),
            memory_weaver_min_memories: default_weaver_min_memories(),
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HomeAssistantConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProactiveConfig {
    /// Whether proactive message delivery is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Urgency threshold above which to auto-deliver (0.0–1.0).
    #[serde(default = "default_delivery_threshold")]
    pub delivery_threshold: f64,
    /// Minimum minutes between proactive messages.
    #[serde(default = "default_cooldown_minutes")]
    pub cooldown_minutes: u64,
}

fn default_delivery_threshold() -> f64 {
    0.4
}
fn default_cooldown_minutes() -> u64 {
    10
}

impl Default for ProactiveConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            delivery_threshold: default_delivery_threshold(),
            cooldown_minutes: default_cooldown_minutes(),
        }
    }
}

// ── Telegram Config ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub bot_token: Option<String>,
    #[serde(default)]
    pub chat_id: Option<String>,
    #[serde(default = "default_tg_poll_secs")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_true")]
    pub forward_proactive: bool,
}

fn default_tg_poll_secs() -> u64 {
    3
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: None,
            chat_id: None,
            poll_interval_secs: default_tg_poll_secs(),
            forward_proactive: true,
        }
    }
}

// ── Memory Evolution Config ─────────────────────────────────────────────────

/// Configuration for memory evolution features (V23).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEvolutionConfig {
    /// Enable smart multi-signal recall (Gap 1).
    #[serde(default = "default_evolution_enabled")]
    pub smart_recall_enabled: bool,
    /// Max extra recall calls per message (Gap 1). Default 2.
    #[serde(default = "default_max_extra_recalls")]
    pub max_extra_recall_calls: usize,
    /// Max total memories to inject from smart recall.
    #[serde(default = "default_max_recall_memories")]
    pub max_recall_memories: usize,
    /// Enable cross-domain entity bridging (Gap 2).
    #[serde(default = "default_evolution_enabled")]
    pub cross_domain_enabled: bool,
    /// Enable semantic drift correction / consolidation (Gap 3).
    #[serde(default = "default_evolution_enabled")]
    pub consolidation_enabled: bool,
    /// Hours between consolidation runs. Default 6.
    #[serde(default = "default_consolidation_hours")]
    pub consolidation_interval_hours: f64,
    /// Similarity threshold to consider two memories as duplicates.
    #[serde(default = "default_dup_threshold")]
    pub duplicate_similarity_threshold: f64,
    /// Enable shared reference freshness tracking (Gap 4).
    #[serde(default = "default_evolution_enabled")]
    pub reference_freshness_enabled: bool,
    /// Half-life for reference freshness decay in days.
    #[serde(default = "default_ref_freshness_days")]
    pub reference_freshness_half_life_days: f64,
    /// Enable variable half-life / pruning (Gap 5).
    #[serde(default = "default_evolution_enabled")]
    pub variable_halflife_enabled: bool,
    /// Hours between pruning runs. Default 24.
    #[serde(default = "default_pruning_hours")]
    pub pruning_interval_hours: f64,
    /// Half-life in seconds for each importance tier.
    #[serde(default)]
    pub tier_half_lives: TierHalfLives,
    /// Enable idle-time memory graph weaving.
    #[serde(default = "default_evolution_enabled")]
    pub weaving_enabled: bool,
    /// Hours between weaving cycles. Default 2.
    #[serde(default = "default_weaving_hours")]
    pub weaving_interval_hours: f64,
    /// Number of memories to process per weaving cycle.
    #[serde(default = "default_weaving_batch")]
    pub weaving_batch_size: usize,
}

/// Half-life durations (in seconds) for each importance tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierHalfLives {
    /// Core facts (identity, key relationships) — default 365 days.
    #[serde(default = "default_tier_core")]
    pub core: f64,
    /// Important events, emotional memories — default 60 days.
    #[serde(default = "default_tier_significant")]
    pub significant: f64,
    /// Default bucket — 7 days.
    #[serde(default = "default_tier_routine")]
    pub routine: f64,
    /// Low-importance, system-generated — 1 day.
    #[serde(default = "default_tier_ephemeral")]
    pub ephemeral: f64,
}

fn default_evolution_enabled() -> bool { true }
fn default_max_extra_recalls() -> usize { 2 }
fn default_max_recall_memories() -> usize { 8 }
fn default_consolidation_hours() -> f64 { 6.0 }
fn default_dup_threshold() -> f64 { 0.85 }
fn default_ref_freshness_days() -> f64 { 14.0 }
fn default_pruning_hours() -> f64 { 24.0 }
fn default_weaving_hours() -> f64 { 2.0 }
fn default_weaving_batch() -> usize { 15 }
fn default_tier_core() -> f64 { 31_536_000.0 }      // 365 days
fn default_tier_significant() -> f64 { 5_184_000.0 } // 60 days
fn default_tier_routine() -> f64 { 604_800.0 }       // 7 days
fn default_tier_ephemeral() -> f64 { 86_400.0 }      // 1 day

impl Default for TierHalfLives {
    fn default() -> Self {
        Self {
            core: default_tier_core(),
            significant: default_tier_significant(),
            routine: default_tier_routine(),
            ephemeral: default_tier_ephemeral(),
        }
    }
}

impl Default for MemoryEvolutionConfig {
    fn default() -> Self {
        Self {
            smart_recall_enabled: true,
            max_extra_recall_calls: default_max_extra_recalls(),
            max_recall_memories: default_max_recall_memories(),
            cross_domain_enabled: true,
            consolidation_enabled: true,
            consolidation_interval_hours: default_consolidation_hours(),
            duplicate_similarity_threshold: default_dup_threshold(),
            reference_freshness_enabled: true,
            reference_freshness_half_life_days: default_ref_freshness_days(),
            variable_halflife_enabled: true,
            pruning_interval_hours: default_pruning_hours(),
            tier_half_lives: TierHalfLives::default(),
            weaving_enabled: true,
            weaving_interval_hours: default_weaving_hours(),
            weaving_batch_size: default_weaving_batch(),
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
    #[serde(default)]
    pub home_assistant: HomeAssistantConfig,
    #[serde(default)]
    pub proactive: ProactiveConfig,
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub memory_evolution: MemoryEvolutionConfig,
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
            home_assistant: HomeAssistantConfig::default(),
            proactive: ProactiveConfig::default(),
            telegram: TelegramConfig::default(),
            memory_evolution: MemoryEvolutionConfig::default(),
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
