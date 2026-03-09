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
        !matches!(self.backend.as_str(), "candle" | "llamacpp" | "claude-cli")
    }

    /// Returns true if this backend uses the Claude Code CLI.
    pub fn is_claude_cli_backend(&self) -> bool {
        self.backend == "claude-cli"
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
    /// Enable predictive workflow instinct.
    #[serde(default = "default_evolution_enabled")]
    pub predictive_workflow_enabled: bool,
    /// Enable morning/evening routine instinct.
    #[serde(default = "default_evolution_enabled")]
    pub routine_enabled: bool,
    /// Enable cognitive load monitor instinct.
    #[serde(default = "default_evolution_enabled")]
    pub cognitive_load_enabled: bool,
    /// Enable smart updates instinct.
    #[serde(default = "default_evolution_enabled")]
    pub smart_updates_enabled: bool,
    /// Enable memory weaver instinct (proactive graph building during idle).
    #[serde(default = "default_evolution_enabled")]
    pub memory_weaver_enabled: bool,
    /// Minutes of idle time before weaver urges start firing.
    #[serde(default = "default_weaver_idle_minutes")]
    pub memory_weaver_idle_minutes: f64,
    /// Minimum memories before weaving is worthwhile.
    #[serde(default = "default_weaver_min_memories")]
    pub memory_weaver_min_memories: i64,
    /// Enable email monitoring instinct (requires email.enabled + accounts configured).
    #[serde(default)]
    pub email_watch_enabled: bool,
    /// Minutes between email checks.
    #[serde(default = "default_email_poll_minutes_f64")]
    pub email_poll_minutes: f64,
    /// Enable news monitoring (uses web_search).
    #[serde(default = "default_true_val")]
    pub news_watch_enabled: bool,
    /// Minutes between news checks.
    #[serde(default = "default_news_interval")]
    pub news_watch_interval_minutes: f64,
    /// Enable trend watching (searches Google/X/Reddit via browser).
    #[serde(default = "default_true_val")]
    pub trend_watch_enabled: bool,
    /// Minutes between trend checks.
    #[serde(default = "default_trend_interval")]
    pub trend_watch_interval_minutes: f64,
    /// Enable curiosity research (idle-triggered interest-based search).
    #[serde(default = "default_true_val")]
    pub curiosity_enabled: bool,
    /// Minimum idle minutes before curiosity fires.
    #[serde(default = "default_curiosity_idle")]
    pub curiosity_idle_minutes: f64,
    /// Hours between curiosity research sessions.
    #[serde(default = "default_curiosity_interval")]
    pub curiosity_interval_hours: f64,
    /// Enable interest intelligence (targeted research based on user's interests).
    #[serde(default = "default_true_val")]
    pub interest_intelligence_enabled: bool,
    /// Hours between interest intelligence research sessions.
    #[serde(default = "default_interest_interval")]
    pub interest_intelligence_interval_hours: f64,
    /// Enable deal watch (monitors deals/prices for items user wants).
    #[serde(default = "default_true_val")]
    pub deal_watch_enabled: bool,
    /// Hours between deal checks.
    #[serde(default = "default_deal_interval")]
    pub deal_watch_interval_hours: f64,
    /// Enable activity recommender (suggests activities based on weather + interests + time).
    #[serde(default = "default_true_val")]
    pub activity_recommender_enabled: bool,
    /// Hours between activity recommendations.
    #[serde(default = "default_activity_interval")]
    pub activity_recommender_interval_hours: f64,
    /// Enable connection weaver (finds surprising bridges between user's interests).
    #[serde(default = "default_true_val")]
    pub connection_weaver_enabled: bool,
    /// Hours between connection weaving attempts.
    #[serde(default = "default_connection_weaver_interval")]
    pub connection_weaver_interval_hours: f64,
    /// Enable context bridge (connects world events to the user's personal life/work).
    #[serde(default = "default_true_val")]
    pub context_bridge_enabled: bool,
    /// Hours between context bridge analyses.
    #[serde(default = "default_context_bridge_interval")]
    pub context_bridge_interval_hours: f64,
    /// Enable deep dive (researches the "why behind the why" of user-mentioned topics).
    #[serde(default = "default_true_val")]
    pub deep_dive_enabled: bool,
    /// Hours between deep dive research attempts.
    #[serde(default = "default_deep_dive_interval")]
    pub deep_dive_interval_hours: f64,
    /// Enable wonder sense (surfaces fascinating "did you know?" facts from context).
    #[serde(default = "default_true_val")]
    pub wonder_sense_enabled: bool,
    /// Hours between wonder sense research sessions.
    #[serde(default = "default_wonder_sense_interval")]
    pub wonder_sense_interval_hours: f64,
    /// Enable golden find (discovers hidden gems — obscure but genuinely useful resources).
    #[serde(default = "default_true_val")]
    pub golden_find_enabled: bool,
    /// Hours between golden find research attempts.
    #[serde(default = "default_golden_find_interval")]
    pub golden_find_interval_hours: f64,
    /// Enable growth mirror (reflects user's growth/progress back to them).
    #[serde(default = "default_true_val")]
    pub growth_mirror_enabled: bool,
    /// Hours between growth mirror reflections.
    #[serde(default = "default_growth_mirror_interval")]
    pub growth_mirror_interval_hours: f64,
    /// Enable local pulse (hyperlocal intelligence — events, openings, closures, community happenings).
    #[serde(default = "default_true_val")]
    pub local_pulse_enabled: bool,
    /// Hours between local pulse checks.
    #[serde(default = "default_local_pulse_interval")]
    pub local_pulse_interval_hours: f64,
    /// Enable tradition keeper (cultural moments, anniversaries, seasonal traditions with depth).
    #[serde(default = "default_true_val")]
    pub tradition_keeper_enabled: bool,
    /// Hours between tradition keeper research sessions.
    #[serde(default = "default_tradition_keeper_interval")]
    pub tradition_keeper_interval_hours: f64,
    /// Enable night owl (late-night intellectual companionship, 10 PM – 4 AM only).
    #[serde(default = "default_true_val")]
    pub night_owl_enabled: bool,
    /// Hours between night owl thoughts.
    #[serde(default = "default_night_owl_interval")]
    pub night_owl_interval_hours: f64,
    /// Enable legacy builder (rare zoom-out reflections on user's narrative arc).
    #[serde(default = "default_true_val")]
    pub legacy_builder_enabled: bool,
    /// Hours between legacy builder reflections (should be high — weekly cadence).
    #[serde(default = "default_legacy_builder_interval")]
    pub legacy_builder_interval_hours: f64,
    /// Enable identity thread (weaves user's values and identity markers into coherent narrative).
    #[serde(default = "default_true_val")]
    pub identity_thread_enabled: bool,
    /// Hours between identity thread observations (should be very high — weekly cadence).
    #[serde(default = "default_identity_thread_interval")]
    pub identity_thread_interval_hours: f64,
    /// Enable myth buster (catches and corrects common misconceptions related to user's interests).
    #[serde(default = "default_true_val")]
    pub myth_buster_enabled: bool,
    /// Hours between myth buster research attempts.
    #[serde(default = "default_myth_buster_interval")]
    pub myth_buster_interval_hours: f64,
    /// Enable cooking companion (contextual food intelligence for users who love cooking).
    #[serde(default = "default_true_val")]
    pub cooking_companion_enabled: bool,
    /// Hours between cooking companion research sessions.
    #[serde(default = "default_cooking_companion_interval")]
    pub cooking_companion_interval_hours: f64,
    /// Enable second brain (periodic memory analysis — patterns, contradictions, forgotten commitments).
    #[serde(default = "default_true_val")]
    pub second_brain_enabled: bool,
    /// Hours between second brain analysis sessions.
    #[serde(default = "default_second_brain_interval")]
    pub second_brain_interval_hours: f64,
    /// Enable health pulse (tracks health mentions, researches underlying science).
    #[serde(default = "default_true_val")]
    pub health_pulse_enabled: bool,
    /// Hours between health pulse evaluations.
    #[serde(default = "default_health_pulse_interval")]
    pub health_pulse_interval_hours: f64,
    /// Enable money mind (financial awareness connected to user's interests).
    #[serde(default = "default_true_val")]
    pub money_mind_enabled: bool,
    /// Hours between money mind evaluations.
    #[serde(default = "default_money_mind_interval")]
    pub money_mind_interval_hours: f64,
    /// Enable relationship radar (social graph intelligence, reach-out nudges).
    #[serde(default = "default_true_val")]
    pub relationship_radar_enabled: bool,
    /// Hours between relationship radar checks.
    #[serde(default = "default_relationship_radar_interval")]
    pub relationship_radar_interval_hours: f64,
    /// Enable goal keeper (gentle accountability for stated intentions).
    #[serde(default = "default_true_val")]
    pub goal_keeper_enabled: bool,
    /// Hours between goal keeper checks.
    #[serde(default = "default_goal_keeper_interval")]
    pub goal_keeper_interval_hours: f64,
    /// Enable decision lab (finds hidden tradeoffs in active decisions).
    #[serde(default = "default_true_val")]
    pub decision_lab_enabled: bool,
    /// Hours between decision lab evaluations.
    #[serde(default = "default_decision_lab_interval")]
    pub decision_lab_interval_hours: f64,
    /// Enable skill forge (finds the mental model that unblocks learning).
    #[serde(default = "default_true_val")]
    pub skill_forge_enabled: bool,
    /// Hours between skill forge evaluations.
    #[serde(default = "default_skill_forge_interval")]
    pub skill_forge_interval_hours: f64,
    /// Enable time capture (micro-journaling prompts at emotionally significant moments).
    #[serde(default = "default_true_val")]
    pub time_capture_enabled: bool,
    /// Hours between time capture checks.
    #[serde(default = "default_time_capture_interval")]
    pub time_capture_interval_hours: f64,
    /// Enable mentor match (finds the right teacher/resource for current learning edge).
    #[serde(default = "default_true_val")]
    pub mentor_match_enabled: bool,
    /// Hours between mentor match evaluations.
    #[serde(default = "default_mentor_match_interval")]
    pub mentor_match_interval_hours: f64,
    /// Enable debrief partner (structured reflection after significant events).
    #[serde(default = "default_true_val")]
    pub debrief_partner_enabled: bool,
    /// Hours between debrief partner checks.
    #[serde(default = "default_debrief_partner_interval")]
    pub debrief_partner_interval_hours: f64,
    /// Enable philosophy companion (connects experiences to philosophical frameworks).
    #[serde(default = "default_true_val")]
    pub philosophy_companion_enabled: bool,
    /// Hours between philosophy companion reflections.
    #[serde(default = "default_philosophy_companion_interval")]
    pub philosophy_companion_interval_hours: f64,
    /// Enable devil's advocate (challenges user's strongest convictions with credible counterarguments).
    #[serde(default = "default_true_val")]
    pub devils_advocate_enabled: bool,
    /// Hours between devil's advocate challenges.
    #[serde(default = "default_devils_advocate_interval")]
    pub devils_advocate_interval_hours: f64,
    /// Enable energy map (observes chronotype and energy patterns).
    #[serde(default = "default_true_val")]
    pub energy_map_enabled: bool,
    /// Hours between energy map observations.
    #[serde(default = "default_energy_map_interval")]
    pub energy_map_interval_hours: f64,
    /// Enable future self (projects current trajectories forward).
    #[serde(default = "default_true_val")]
    pub future_self_enabled: bool,
    /// Hours between future self projections.
    #[serde(default = "default_future_self_interval")]
    pub future_self_interval_hours: f64,
    /// Enable dream keeper (turns aspirations into actionable next steps).
    #[serde(default = "default_true_val")]
    pub dream_keeper_enabled: bool,
    /// Hours between dream keeper actionizations.
    #[serde(default = "default_dream_keeper_interval")]
    pub dream_keeper_interval_hours: f64,
    /// Enable cultural radar (discovers new content matching user's taste).
    #[serde(default = "default_true_val")]
    pub cultural_radar_enabled: bool,
    /// Hours between cultural radar discoveries.
    #[serde(default = "default_cultural_radar_interval")]
    pub cultural_radar_interval_hours: f64,
    /// Enable pattern breaker (identifies and surfaces recurring loops).
    #[serde(default = "default_true_val")]
    pub pattern_breaker_enabled: bool,
    /// Hours between pattern breaker observations.
    #[serde(default = "default_pattern_breaker_interval")]
    pub pattern_breaker_interval_hours: f64,
    /// Enable opportunity scout (finds real-world opportunities matching user's profile).
    #[serde(default = "default_true_val")]
    pub opportunity_scout_enabled: bool,
    /// Hours between opportunity scout searches.
    #[serde(default = "default_opportunity_scout_interval")]
    pub opportunity_scout_interval_hours: f64,
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
fn default_email_poll_minutes_f64() -> f64 {
    5.0
}
fn default_true_val() -> bool { true }
fn default_news_interval() -> f64 { 60.0 }
fn default_trend_interval() -> f64 { 45.0 }
fn default_curiosity_idle() -> f64 { 15.0 }
fn default_curiosity_interval() -> f64 { 4.0 }
fn default_interest_interval() -> f64 { 3.0 }
fn default_deal_interval() -> f64 { 6.0 }
fn default_activity_interval() -> f64 { 4.0 }
fn default_connection_weaver_interval() -> f64 { 5.0 }
fn default_context_bridge_interval() -> f64 { 8.0 }
fn default_deep_dive_interval() -> f64 { 6.0 }
fn default_wonder_sense_interval() -> f64 { 4.0 }
fn default_golden_find_interval() -> f64 { 6.0 }
fn default_growth_mirror_interval() -> f64 { 18.0 }
fn default_local_pulse_interval() -> f64 { 6.0 }
fn default_tradition_keeper_interval() -> f64 { 22.0 }
fn default_night_owl_interval() -> f64 { 2.5 }
fn default_legacy_builder_interval() -> f64 { 168.0 } // ~weekly
fn default_identity_thread_interval() -> f64 { 168.0 } // ~weekly
fn default_myth_buster_interval() -> f64 { 8.0 }
fn default_cooking_companion_interval() -> f64 { 6.0 }
fn default_second_brain_interval() -> f64 { 12.0 }
fn default_health_pulse_interval() -> f64 { 4.0 }
fn default_money_mind_interval() -> f64 { 12.0 }
fn default_relationship_radar_interval() -> f64 { 6.0 }
fn default_goal_keeper_interval() -> f64 { 6.0 }
fn default_decision_lab_interval() -> f64 { 8.0 }
fn default_skill_forge_interval() -> f64 { 12.0 }
fn default_time_capture_interval() -> f64 { 12.0 }
fn default_mentor_match_interval() -> f64 { 48.0 }
fn default_debrief_partner_interval() -> f64 { 6.0 }
fn default_philosophy_companion_interval() -> f64 { 48.0 }
fn default_devils_advocate_interval() -> f64 { 24.0 }
fn default_energy_map_interval() -> f64 { 48.0 }
fn default_future_self_interval() -> f64 { 72.0 }
fn default_dream_keeper_interval() -> f64 { 72.0 }
fn default_cultural_radar_interval() -> f64 { 24.0 }
fn default_pattern_breaker_interval() -> f64 { 72.0 }
fn default_opportunity_scout_interval() -> f64 { 48.0 }

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
            predictive_workflow_enabled: true,
            routine_enabled: true,
            cognitive_load_enabled: true,
            smart_updates_enabled: true,
            memory_weaver_enabled: true,
            memory_weaver_idle_minutes: default_weaver_idle_minutes(),
            memory_weaver_min_memories: default_weaver_min_memories(),
            email_watch_enabled: false,
            email_poll_minutes: default_email_poll_minutes_f64(),
            news_watch_enabled: true,
            news_watch_interval_minutes: default_news_interval(),
            trend_watch_enabled: true,
            trend_watch_interval_minutes: default_trend_interval(),
            curiosity_enabled: true,
            curiosity_idle_minutes: default_curiosity_idle(),
            curiosity_interval_hours: default_curiosity_interval(),
            interest_intelligence_enabled: true,
            interest_intelligence_interval_hours: default_interest_interval(),
            deal_watch_enabled: true,
            deal_watch_interval_hours: default_deal_interval(),
            activity_recommender_enabled: true,
            activity_recommender_interval_hours: default_activity_interval(),
            connection_weaver_enabled: true,
            connection_weaver_interval_hours: default_connection_weaver_interval(),
            context_bridge_enabled: true,
            context_bridge_interval_hours: default_context_bridge_interval(),
            deep_dive_enabled: true,
            deep_dive_interval_hours: default_deep_dive_interval(),
            wonder_sense_enabled: true,
            wonder_sense_interval_hours: default_wonder_sense_interval(),
            golden_find_enabled: true,
            golden_find_interval_hours: default_golden_find_interval(),
            growth_mirror_enabled: true,
            growth_mirror_interval_hours: default_growth_mirror_interval(),
            local_pulse_enabled: true,
            local_pulse_interval_hours: default_local_pulse_interval(),
            tradition_keeper_enabled: true,
            tradition_keeper_interval_hours: default_tradition_keeper_interval(),
            night_owl_enabled: true,
            night_owl_interval_hours: default_night_owl_interval(),
            legacy_builder_enabled: true,
            legacy_builder_interval_hours: default_legacy_builder_interval(),
            identity_thread_enabled: true,
            identity_thread_interval_hours: default_identity_thread_interval(),
            myth_buster_enabled: true,
            myth_buster_interval_hours: default_myth_buster_interval(),
            cooking_companion_enabled: true,
            cooking_companion_interval_hours: default_cooking_companion_interval(),
            second_brain_enabled: true,
            second_brain_interval_hours: default_second_brain_interval(),
            health_pulse_enabled: true,
            health_pulse_interval_hours: default_health_pulse_interval(),
            money_mind_enabled: true,
            money_mind_interval_hours: default_money_mind_interval(),
            relationship_radar_enabled: true,
            relationship_radar_interval_hours: default_relationship_radar_interval(),
            goal_keeper_enabled: true,
            goal_keeper_interval_hours: default_goal_keeper_interval(),
            decision_lab_enabled: true,
            decision_lab_interval_hours: default_decision_lab_interval(),
            skill_forge_enabled: true,
            skill_forge_interval_hours: default_skill_forge_interval(),
            time_capture_enabled: true,
            time_capture_interval_hours: default_time_capture_interval(),
            mentor_match_enabled: true,
            mentor_match_interval_hours: default_mentor_match_interval(),
            debrief_partner_enabled: true,
            debrief_partner_interval_hours: default_debrief_partner_interval(),
            philosophy_companion_enabled: true,
            philosophy_companion_interval_hours: default_philosophy_companion_interval(),
            devils_advocate_enabled: true,
            devils_advocate_interval_hours: default_devils_advocate_interval(),
            energy_map_enabled: true,
            energy_map_interval_hours: default_energy_map_interval(),
            future_self_enabled: true,
            future_self_interval_hours: default_future_self_interval(),
            dream_keeper_enabled: true,
            dream_keeper_interval_hours: default_dream_keeper_interval(),
            cultural_radar_enabled: true,
            cultural_radar_interval_hours: default_cultural_radar_interval(),
            pattern_breaker_enabled: true,
            pattern_breaker_interval_hours: default_pattern_breaker_interval(),
            opportunity_scout_enabled: true,
            opportunity_scout_interval_hours: default_opportunity_scout_interval(),
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

// ── WhatsApp Config ────────────────────────────────────────────────────────

/// WhatsApp Business API configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppConfig {
    #[serde(default)]
    pub enabled: bool,
    /// WhatsApp Business phone number ID (from Meta Business).
    #[serde(default)]
    pub phone_number_id: Option<String>,
    /// Permanent access token (from Meta Business).
    #[serde(default)]
    pub access_token: Option<String>,
    /// Default recipient phone number in international format.
    #[serde(default)]
    pub recipient: Option<String>,
}

impl Default for WhatsAppConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            phone_number_id: None,
            access_token: None,
            recipient: None,
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

// ── Email Config ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAccountConfig {
    pub name: String,               // "Personal Gmail"
    pub email: String,              // user@gmail.com
    pub provider: String,           // "gmail", "outlook", "yahoo", "imap"
    #[serde(default)]
    pub imap_server: Option<String>,  // auto-detected from provider, or custom
    #[serde(default = "default_imap_port")]
    pub imap_port: u16,
    #[serde(default)]
    pub smtp_server: Option<String>,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    #[serde(default)]
    pub password: String,           // App password (empty if OAuth)
    #[serde(default)]
    pub auth_method: Option<String>, // "password" or "oauth2" (default: password)
    #[serde(default)]
    pub oauth_access_token: Option<String>,
    #[serde(default)]
    pub oauth_refresh_token: Option<String>,
    #[serde(default)]
    pub oauth_token_expiry: Option<f64>,  // unix timestamp
}

fn default_imap_port() -> u16 { 993 }
fn default_smtp_port() -> u16 { 587 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub accounts: Vec<EmailAccountConfig>,
    #[serde(default = "default_email_poll_minutes")]
    pub poll_interval_minutes: u32,
    #[serde(default = "default_evolution_enabled")]
    pub notify_important: bool,
}

fn default_email_poll_minutes() -> u32 { 5 }

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            accounts: Vec::new(),
            poll_interval_minutes: default_email_poll_minutes(),
            notify_important: true,
        }
    }
}

// ── Calendar Config ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Which email account to use for Google Calendar API (by name or email).
    /// Defaults to first email account with OAuth2.
    #[serde(default)]
    pub account: Option<String>,
    /// Minutes between calendar sync refreshes.
    #[serde(default = "default_calendar_poll_minutes")]
    pub poll_interval_minutes: u32,
}

fn default_calendar_poll_minutes() -> u32 { 10 }

impl Default for CalendarConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            account: None,
            poll_interval_minutes: default_calendar_poll_minutes(),
        }
    }
}

// ── Agent Loop Config ────────────────────────────────────────────────────────

/// Configuration for the robust agent loop (nudge, error recovery, trace learning).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Maximum total tool-calling steps per query.
    #[serde(default = "default_agent_max_steps")]
    pub max_steps: usize,
    /// Nudge the LLM when it responds without completing the task.
    #[serde(default = "default_agent_true")]
    pub nudge_on_empty: bool,
    /// Maximum nudge messages per query before giving up.
    #[serde(default = "default_agent_max_nudges")]
    pub max_nudges: usize,
    /// Suggest alternative tools when one fails.
    #[serde(default = "default_agent_true")]
    pub error_recovery: bool,
    /// Enable tool chain trace learning.
    #[serde(default = "default_agent_true")]
    pub trace_learning: bool,
    /// Minimum similarity threshold for trace hints (0.0–1.0).
    #[serde(default = "default_trace_min_sim")]
    pub trace_min_similarity: f32,
}

fn default_agent_max_steps() -> usize { 30 }
fn default_agent_true() -> bool { true }
fn default_agent_max_nudges() -> usize { 2 }
fn default_trace_min_sim() -> f32 { 0.5 }

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_steps: default_agent_max_steps(),
            nudge_on_empty: default_agent_true(),
            max_nudges: default_agent_max_nudges(),
            error_recovery: default_agent_true(),
            trace_learning: default_agent_true(),
            trace_min_similarity: default_trace_min_sim(),
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
    pub whatsapp: WhatsAppConfig,
    #[serde(default)]
    pub memory_evolution: MemoryEvolutionConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub email: EmailConfig,
    #[serde(default)]
    pub calendar: CalendarConfig,
    #[serde(default)]
    pub connectors: ConnectorsConfig,
    #[serde(default)]
    pub vault: VaultConfig,
    /// MCP (Model Context Protocol) servers to connect to at startup.
    /// Each server exposes tools that become available to the companion.
    #[serde(default)]
    pub mcp_servers: Vec<crate::tools::mcp::McpServerEntry>,
    /// Services the user actually uses. Only cortex rules and instincts for
    /// enabled services will fire. Examples: "email", "calendar", "git", "jira".
    /// If empty, defaults to a minimal personal set.
    #[serde(default = "default_enabled_services")]
    pub enabled_services: Vec<String>,
}

/// OAuth connector configuration for external services.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorsConfig {
    /// OAuth2 callback port (default: 9876).
    #[serde(default = "default_connector_port")]
    pub callback_port: u16,
    /// Google OAuth2 client ID (from Google Cloud Console).
    #[serde(default)]
    pub google_client_id: Option<String>,
    /// Spotify OAuth2 client ID (from Spotify Developer Dashboard).
    #[serde(default)]
    pub spotify_client_id: Option<String>,
    /// Spotify OAuth2 client secret (required — Spotify doesn't support PKCE-only).
    #[serde(default)]
    pub spotify_client_secret: Option<String>,
    /// Facebook (Meta) App ID (from Meta for Developers).
    #[serde(default)]
    pub facebook_app_id: Option<String>,
    /// Facebook App Secret (required for token exchange).
    #[serde(default)]
    pub facebook_app_secret: Option<String>,
    /// Instagram uses the same Meta App ID as Facebook.
    /// Set facebook_app_id to enable both.
    #[serde(default)]
    pub instagram_app_id: Option<String>,
    /// Background sync interval in minutes (default: 30).
    #[serde(default = "default_connector_sync_interval")]
    pub sync_interval_minutes: u32,
}

fn default_connector_port() -> u16 { 9876 }
fn default_connector_sync_interval() -> u32 { 30 }

/// Vault security configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultConfig {
    /// Security PIN hash (blake3). Set via `vault_set_pin` tool.
    /// When set, vault_get requires PIN verification before returning passwords.
    /// Protects against unauthorized access over Telegram or if someone gains
    /// physical access to an unlocked session.
    #[serde(default)]
    pub pin_hash: Option<String>,
    /// Require PIN for vault_get (default: true when pin_hash is set).
    #[serde(default = "default_true")]
    pub require_pin: bool,
    /// Auto-lock vault after N seconds of inactivity (default: 300 = 5 min).
    #[serde(default = "default_vault_lock_timeout")]
    pub lock_timeout_secs: u64,
}

fn default_vault_lock_timeout() -> u64 { 300 }

impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            pin_hash: None,
            require_pin: true,
            lock_timeout_secs: default_vault_lock_timeout(),
        }
    }
}

impl Default for ConnectorsConfig {
    fn default() -> Self {
        Self {
            callback_port: default_connector_port(),
            google_client_id: None,
            spotify_client_id: None,
            spotify_client_secret: None,
            facebook_app_id: None,
            facebook_app_secret: None,
            instagram_app_id: None,
            sync_interval_minutes: default_connector_sync_interval(),
        }
    }
}

fn default_user_name() -> String {
    "User".to_string()
}

/// Default enabled services — personal use, no enterprise tools.
fn default_enabled_services() -> Vec<String> {
    vec![
        "email".to_string(),
        "calendar".to_string(),
        "browser".to_string(),
    ]
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
            whatsapp: WhatsAppConfig::default(),
            memory_evolution: MemoryEvolutionConfig::default(),
            agent: AgentConfig::default(),
            email: EmailConfig::default(),
            calendar: CalendarConfig::default(),
            connectors: ConnectorsConfig::default(),
            vault: VaultConfig::default(),
            mcp_servers: Vec::new(),
            enabled_services: default_enabled_services(),
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

    /// Check if a service is enabled by the user.
    pub fn has_service(&self, service: &str) -> bool {
        self.enabled_services.iter().any(|s| s.eq_ignore_ascii_case(service))
    }
}
