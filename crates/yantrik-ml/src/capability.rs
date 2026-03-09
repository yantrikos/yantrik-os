//! Model Capability Profile — adaptive intelligence based on model size.
//!
//! Auto-detects model parameters from the model name/metadata and creates
//! a capability profile that the companion uses to adjust its strategy:
//! - Tool exposure (how many tools per prompt)
//! - Tool call mode (MCQ vs structured JSON vs freeform function call)
//! - Slot extraction mode (key-value vs JSON)
//! - Context budget (how much ambient context to maintain)
//! - Agent loop depth (max steps, repair loops)
//! - Guardrail strictness (confidence thresholds, confirmation requirements)
//!
//! This allows ONE codebase to adapt from 0.8B fallback through 9B primary
//! to 27B+ power mode — without separate code paths.

use serde::{Deserialize, Serialize};

// ── Model Tier ────────────────────────────────────────────────────────

/// Broad capability tier derived from model parameter count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ModelTier {
    /// 0.5–1.5B params. Very constrained — MCQ routing, KV slots, 3 tools max.
    Tiny,
    /// 1.5–4B params. Limited — structured JSON, 4-5 tools, basic multi-step.
    Small,
    /// 4–14B params. Capable — structured JSON, 6-8 tools, family routing, repair loops.
    Medium,
    /// 14B+ params. Strong — freeform function calls, 12+ tools, full agent loop.
    Large,
}

impl ModelTier {
    /// Classify a model into a tier based on its name/identifier.
    ///
    /// Heuristic: parse parameter count from common naming conventions:
    /// - `qwen3.5:0.6b`, `qwen3.5:27b-nothink`, `llama3.2:3b`
    /// - `Qwen3.5-9B`, `Llama-3.2-1B`
    /// - Falls back to Medium if undetectable (safe default).
    pub fn from_model_name(model: &str) -> Self {
        if let Some(params_b) = Self::extract_param_count(model) {
            match params_b {
                x if x < 1.5 => ModelTier::Tiny,
                x if x < 4.0 => ModelTier::Small,
                x if x < 14.0 => ModelTier::Medium,
                _ => ModelTier::Large,
            }
        } else {
            // Cloud models or unrecognizable → treat as Large
            if model.contains("claude") || model.contains("gpt-") || model.contains("gemini") {
                ModelTier::Large
            } else {
                // Safe default for unknown local models
                ModelTier::Medium
            }
        }
    }

    /// Extract parameter count in billions from model name.
    ///
    /// Handles formats:
    /// - `qwen3.5:27b-nothink` → 27.0
    /// - `qwen3.5:0.6b` → 0.6
    /// - `Qwen3.5-9B` → 9.0
    /// - `llama3.2:3b-instruct` → 3.0
    /// - `phi-3-mini-4k-3.8b` → 3.8
    fn extract_param_count(model: &str) -> Option<f64> {
        let lower = model.to_lowercase();

        // Pattern 1: `:Xb` (Ollama tag format) — e.g., `qwen3.5:27b-nothink`
        if let Some(colon_idx) = lower.rfind(':') {
            let after_colon = &lower[colon_idx + 1..];
            if let Some(b_idx) = after_colon.find('b') {
                if let Ok(val) = after_colon[..b_idx].parse::<f64>() {
                    if val > 0.0 && val < 1000.0 {
                        return Some(val);
                    }
                }
            }
        }

        // Pattern 2: `-XB` or `_XB` (HuggingFace format) — e.g., `Qwen3.5-9B`
        for sep in ['-', '_'] {
            for part in lower.split(sep) {
                if part.ends_with('b') && part.len() > 1 {
                    let num_part = &part[..part.len() - 1];
                    if let Ok(val) = num_part.parse::<f64>() {
                        if val > 0.0 && val < 1000.0 {
                            return Some(val);
                        }
                    }
                }
            }
        }

        None
    }
}

impl std::fmt::Display for ModelTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelTier::Tiny => write!(f, "tiny"),
            ModelTier::Small => write!(f, "small"),
            ModelTier::Medium => write!(f, "medium"),
            ModelTier::Large => write!(f, "large"),
        }
    }
}

// ── Tool Call Mode ────────────────────────────────────────────────────

/// How the model should express tool calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolCallMode {
    /// Multiple-choice question: "Which tool? A) recall B) web_search C) remember"
    /// Best for tiny models (0.5-1.5B) — reduces decision to A/B/C selection.
    MCQ,
    /// Model outputs structured JSON: `{"tool": "recall", "args": {"query": "..."}}`
    /// Good for medium models (4-14B) with strong IFEval but moderate BFCL.
    StructuredJSON,
    /// Standard OpenAI function-calling format.
    /// For large models (14B+) with strong BFCL scores.
    NativeFunctionCall,
}

// ── Slot Extraction Mode ──────────────────────────────────────────────

/// How the model extracts structured parameters from user queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotMode {
    /// Key-value pairs: `TASK: call mom\nWHEN: tomorrow 6pm`
    /// Simplest format, best for tiny models.
    KeyValue,
    /// JSON object with defined schema.
    /// Good for medium+ models.
    JSON,
}

// ── Model Capability Profile ──────────────────────────────────────────

/// Complete capability profile for adapting system behavior to model size.
///
/// Created automatically from model name via `ModelCapabilityProfile::from_model_name()`.
/// The companion reads this to adjust tool exposure, prompt strategy, and safety gates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilityProfile {
    /// Detected model tier.
    pub tier: ModelTier,
    /// Detected or estimated parameter count (billions).
    pub estimated_params_b: f64,
    /// Original model name used for detection.
    pub model_name: String,

    // ── Tool strategy ─────────────────────────────────────────────
    /// Maximum tools to expose in a single prompt.
    pub max_tools_per_prompt: usize,
    /// How the model expresses tool calls.
    pub tool_call_mode: ToolCallMode,
    /// How the model extracts parameters.
    pub slot_mode: SlotMode,
    /// Whether to use family-based tool routing (expose only one family at a time).
    pub use_family_routing: bool,

    // ── Agent loop ────────────────────────────────────────────────
    /// Maximum agent loop iterations.
    pub max_agent_steps: usize,
    /// Whether the model can handle repair/retry loops on tool call failures.
    pub supports_repair_loop: bool,
    /// Maximum repair attempts per tool call.
    pub max_repair_attempts: usize,
    /// Whether the model can be used for multi-step workflows.
    pub multi_step_capable: bool,

    // ── Context strategy ──────────────────────────────────────────
    /// Maximum effective context window (tokens) to use in practice.
    /// Not the model's theoretical max — the practical sweet spot for latency.
    pub max_effective_context: usize,
    /// How many tokens of ambient "Active Day Context" to maintain.
    pub ambient_context_budget: usize,
    /// Maximum conversation history turns to include.
    pub max_history_turns: usize,

    // ── Generation config overrides ───────────────────────────────
    /// Maximum tokens to generate per response.
    pub max_generation_tokens: usize,
    /// Recommended temperature for tool-calling tasks.
    pub tool_temperature: f64,
    /// Recommended temperature for free-text responses.
    pub chat_temperature: f64,

    // ── Safety & guardrails ───────────────────────────────────────
    /// Minimum confidence threshold before executing a tool (0.0 – 1.0).
    /// Lower-capability models need higher thresholds.
    pub confidence_threshold: f64,
    /// Whether proactive nudge generation uses the LLM (vs pure templates).
    pub llm_nudge_polish: bool,
    /// Whether the model can handle open-ended summarization safely.
    pub can_summarize_freely: bool,
    /// Whether to run hallucination firewall on all factual responses.
    pub hallucination_firewall: bool,
}

impl ModelCapabilityProfile {
    /// Create a capability profile from a model name string.
    ///
    /// # Examples
    /// ```
    /// use yantrik_ml::ModelCapabilityProfile;
    ///
    /// let p = ModelCapabilityProfile::from_model_name("qwen3.5:0.6b");
    /// assert_eq!(p.tier.to_string(), "tiny");
    /// assert_eq!(p.max_tools_per_prompt, 3);
    ///
    /// let p = ModelCapabilityProfile::from_model_name("qwen3.5:9b");
    /// assert_eq!(p.tier.to_string(), "medium");
    /// assert_eq!(p.max_tools_per_prompt, 8);
    ///
    /// let p = ModelCapabilityProfile::from_model_name("qwen3.5:27b-nothink");
    /// assert_eq!(p.tier.to_string(), "large");
    /// ```
    pub fn from_model_name(model: &str) -> Self {
        let tier = ModelTier::from_model_name(model);
        let params = ModelTier::extract_param_count(model).unwrap_or(match tier {
            ModelTier::Tiny => 0.8,
            ModelTier::Small => 3.0,
            ModelTier::Medium => 9.0,
            ModelTier::Large => 27.0,
        });

        match tier {
            ModelTier::Tiny => Self::tiny(model, params),
            ModelTier::Small => Self::small(model, params),
            ModelTier::Medium => Self::medium(model, params),
            ModelTier::Large => Self::large(model, params),
        }
    }

    /// Create a profile for degraded/fallback mode (even more constrained than Tiny).
    pub fn degraded() -> Self {
        Self {
            tier: ModelTier::Tiny,
            estimated_params_b: 0.5,
            model_name: "degraded".into(),

            max_tools_per_prompt: 3,
            tool_call_mode: ToolCallMode::MCQ,
            slot_mode: SlotMode::KeyValue,
            use_family_routing: false, // too few tools to bother

            max_agent_steps: 3,
            supports_repair_loop: false,
            max_repair_attempts: 0,
            multi_step_capable: false,

            max_effective_context: 2048,
            ambient_context_budget: 0,
            max_history_turns: 2,

            max_generation_tokens: 512,
            tool_temperature: 0.0,
            chat_temperature: 0.3,

            confidence_threshold: 0.95,
            llm_nudge_polish: false,
            can_summarize_freely: false,
            hallucination_firewall: true,
        }
    }

    fn tiny(model: &str, params: f64) -> Self {
        Self {
            tier: ModelTier::Tiny,
            estimated_params_b: params,
            model_name: model.into(),

            max_tools_per_prompt: 3,
            tool_call_mode: ToolCallMode::MCQ,
            slot_mode: SlotMode::KeyValue,
            use_family_routing: false, // MCQ already narrows choices

            max_agent_steps: 3,
            supports_repair_loop: false,
            max_repair_attempts: 0,
            multi_step_capable: false,

            max_effective_context: 4096,
            ambient_context_budget: 512,
            max_history_turns: 3,

            max_generation_tokens: 512,
            tool_temperature: 0.0,
            chat_temperature: 0.5,

            confidence_threshold: 0.9,
            llm_nudge_polish: false,
            can_summarize_freely: false,
            hallucination_firewall: true,
        }
    }

    fn small(model: &str, params: f64) -> Self {
        Self {
            tier: ModelTier::Small,
            estimated_params_b: params,
            model_name: model.into(),

            max_tools_per_prompt: 5,
            tool_call_mode: ToolCallMode::StructuredJSON,
            slot_mode: SlotMode::JSON,
            use_family_routing: true,

            max_agent_steps: 5,
            supports_repair_loop: true,
            max_repair_attempts: 1,
            multi_step_capable: false,

            max_effective_context: 8192,
            ambient_context_budget: 1024,
            max_history_turns: 5,

            max_generation_tokens: 1024,
            tool_temperature: 0.1,
            chat_temperature: 0.6,

            confidence_threshold: 0.85,
            llm_nudge_polish: false,
            can_summarize_freely: false,
            hallucination_firewall: true,
        }
    }

    fn medium(model: &str, params: f64) -> Self {
        Self {
            tier: ModelTier::Medium,
            estimated_params_b: params,
            model_name: model.into(),

            max_tools_per_prompt: 8,
            tool_call_mode: ToolCallMode::StructuredJSON,
            slot_mode: SlotMode::JSON,
            use_family_routing: true,

            max_agent_steps: 10,
            supports_repair_loop: true,
            max_repair_attempts: 2,
            multi_step_capable: true,

            max_effective_context: 32768,
            ambient_context_budget: 8192,
            max_history_turns: 10,

            max_generation_tokens: 2048,
            tool_temperature: 0.2,
            chat_temperature: 0.7,

            confidence_threshold: 0.75,
            llm_nudge_polish: true,
            can_summarize_freely: true,
            hallucination_firewall: true,
        }
    }

    fn large(model: &str, params: f64) -> Self {
        Self {
            tier: ModelTier::Large,
            estimated_params_b: params,
            model_name: model.into(),

            max_tools_per_prompt: 15,
            tool_call_mode: ToolCallMode::NativeFunctionCall,
            slot_mode: SlotMode::JSON,
            use_family_routing: true, // still beneficial even for large models

            max_agent_steps: 15,
            supports_repair_loop: true,
            max_repair_attempts: 3,
            multi_step_capable: true,

            max_effective_context: 65536,
            ambient_context_budget: 16384,
            max_history_turns: 20,

            max_generation_tokens: 4096,
            tool_temperature: 0.3,
            chat_temperature: 0.7,

            confidence_threshold: 0.6,
            llm_nudge_polish: true,
            can_summarize_freely: true,
            hallucination_firewall: false, // large models hallucinate less
        }
    }

    /// Whether this profile should use native OpenAI-format function calling.
    pub fn uses_native_tools(&self) -> bool {
        self.tool_call_mode == ToolCallMode::NativeFunctionCall
    }

    /// Whether this profile should use MCQ-style tool selection.
    pub fn uses_mcq(&self) -> bool {
        self.tool_call_mode == ToolCallMode::MCQ
    }

    /// Get a GenerationConfig tuned for tool-calling tasks.
    pub fn tool_gen_config(&self) -> crate::types::GenerationConfig {
        crate::types::GenerationConfig {
            max_tokens: self.max_generation_tokens,
            temperature: self.tool_temperature,
            top_p: if self.tool_temperature < 0.1 { None } else { Some(0.9) },
            ..Default::default()
        }
    }

    /// Get a GenerationConfig tuned for free-text chat responses.
    pub fn chat_gen_config(&self) -> crate::types::GenerationConfig {
        crate::types::GenerationConfig {
            max_tokens: self.max_generation_tokens,
            temperature: self.chat_temperature,
            top_p: Some(0.9),
            ..Default::default()
        }
    }

    /// Summary string for logging.
    pub fn summary(&self) -> String {
        format!(
            "{}(~{:.1}B) tools={} mode={:?} ctx={}K steps={} family_routing={}",
            self.tier,
            self.estimated_params_b,
            self.max_tools_per_prompt,
            self.tool_call_mode,
            self.max_effective_context / 1024,
            self.max_agent_steps,
            self.use_family_routing,
        )
    }
}

// ── Tool Family ───────────────────────────────────────────────────────

/// Semantic tool families for capability-family routing.
///
/// Instead of exposing 30+ tools, route the query to a family first,
/// then expose only that family's tools. This dramatically improves
/// tool selection accuracy for smaller models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolFamily {
    /// Email, messaging, notifications, WhatsApp.
    Communicate,
    /// Calendar events, scheduling, free time.
    Schedule,
    /// Memory recall, store, search, notes.
    Remember,
    /// Web search, fetch, browse, extract.
    Browse,
    /// Read, write, edit, search files.
    Files,
    /// System commands, reminders, timers, info.
    System,
    /// Sub-agents, cloud escalation, complex delegation.
    Delegate,
    /// Weather, news, local events, world state.
    World,
}

impl ToolFamily {
    /// All families as a slice.
    pub const ALL: &[ToolFamily] = &[
        ToolFamily::Communicate,
        ToolFamily::Schedule,
        ToolFamily::Remember,
        ToolFamily::Browse,
        ToolFamily::Files,
        ToolFamily::System,
        ToolFamily::Delegate,
        ToolFamily::World,
    ];

    /// Keywords that map to this family (for lightweight routing).
    pub fn keywords(&self) -> &'static [&'static str] {
        match self {
            ToolFamily::Communicate => &[
                "email", "mail", "inbox", "send", "reply", "message", "whatsapp",
                "telegram", "notify", "notification", "draft",
            ],
            ToolFamily::Schedule => &[
                "calendar", "event", "meeting", "schedule", "appointment",
                "today", "tomorrow", "free time", "busy", "agenda",
            ],
            ToolFamily::Remember => &[
                "remember", "recall", "memory", "memories", "forget",
                "note", "notes", "what did", "preference",
            ],
            ToolFamily::Browse => &[
                "search", "browse", "website", "web", "url", "http", "fetch",
                "download", "look up", "find online", "google",
            ],
            ToolFamily::Files => &[
                "file", "read", "write", "directory", "folder", "edit",
                "grep", "glob", "code", "script", "save file",
            ],
            ToolFamily::System => &[
                "system", "process", "disk", "cpu", "reminder", "timer",
                "alarm", "uptime", "run command", "execute", "screenshot",
            ],
            ToolFamily::Delegate => &[
                "parallel", "simultaneously", "multiple tasks", "spawn",
                "complex", "analyze deeply", "think hard", "claude",
            ],
            ToolFamily::World => &[
                "weather", "temperature", "forecast", "rain", "news",
                "events nearby", "what's happening", "connect", "sync",
            ],
        }
    }

    /// Tool names that belong to this family.
    pub fn tools(&self) -> &'static [&'static str] {
        match self {
            ToolFamily::Communicate => &[
                "email_check", "email_list", "email_read", "email_send",
                "email_reply", "email_search", "telegram_send", "send_notification",
                "whatsapp_send", "whatsapp_read",
            ],
            ToolFamily::Schedule => &[
                "calendar_today", "calendar_list_events", "calendar_create_event",
                "calendar_update_event", "calendar_delete_event",
                "set_reminder", "create_schedule", "list_schedules", "date_calc",
            ],
            ToolFamily::Remember => &[
                "recall", "remember", "memory_stats", "resolve_conflicts",
                "review_memories", "forget_topic",
            ],
            ToolFamily::Browse => &[
                "web_search", "web_fetch", "http_fetch",
                "launch_browser", "browse", "browser_snapshot", "browser_scroll",
                "browser_click_element", "browser_type_element", "browser_search",
                "browser_see", "browser_click_xy", "browser_type_xy",
                "browser_cleanup", "browser_status",
            ],
            ToolFamily::Files => &[
                "read_file", "write_file", "list_files", "search_files",
                "edit_file", "grep", "glob",
                "code_execute", "script_write", "script_run",
                "script_patch", "script_list", "script_read",
            ],
            ToolFamily::System => &[
                "run_command", "system_info", "disk_usage",
                "list_processes", "diagnose_process",
                "calculate", "screenshot",
            ],
            ToolFamily::Delegate => &[
                "spawn_agents", "claude_think", "claude_code",
            ],
            ToolFamily::World => &[
                "get_weather", "life_search", "recall_preferences",
                "save_user_fact", "search_sources", "extract_search_results",
                "rank_results", "list_connections", "connect_service",
                "sync_service", "disconnect_service",
                "queue_task", "list_tasks", "update_task", "complete_task",
                "create_recipe", "list_recipes", "run_recipe",
                "check_bond",
            ],
        }
    }

    /// Route a query to the best-matching family using keyword matching.
    /// Returns families sorted by match score (best first).
    pub fn route_query(query: &str) -> Vec<(ToolFamily, f64)> {
        let query_lower = query.to_lowercase();
        let mut scores: Vec<(ToolFamily, f64)> = ToolFamily::ALL
            .iter()
            .map(|&family| {
                let keywords = family.keywords();
                let matches = keywords.iter()
                    .filter(|kw| query_lower.contains(**kw))
                    .count();
                let score = matches as f64 / keywords.len() as f64;
                (family, score)
            })
            .filter(|(_, score)| *score > 0.0)
            .collect();

        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scores
    }

    /// Get the single best family for a query, or None if no keywords match.
    pub fn best_for_query(query: &str) -> Option<ToolFamily> {
        Self::route_query(query).first().map(|(f, _)| *f)
    }
}

impl std::fmt::Display for ToolFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolFamily::Communicate => write!(f, "COMMUNICATE"),
            ToolFamily::Schedule => write!(f, "SCHEDULE"),
            ToolFamily::Remember => write!(f, "REMEMBER"),
            ToolFamily::Browse => write!(f, "BROWSE"),
            ToolFamily::Files => write!(f, "FILES"),
            ToolFamily::System => write!(f, "SYSTEM"),
            ToolFamily::Delegate => write!(f, "DELEGATE"),
            ToolFamily::World => write!(f, "WORLD"),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_detection_ollama_format() {
        assert_eq!(ModelTier::from_model_name("qwen3.5:0.6b"), ModelTier::Tiny);
        assert_eq!(ModelTier::from_model_name("qwen3.5:1b"), ModelTier::Tiny);
        assert_eq!(ModelTier::from_model_name("qwen3.5:3b"), ModelTier::Small);
        assert_eq!(ModelTier::from_model_name("qwen3.5:9b"), ModelTier::Medium);
        assert_eq!(ModelTier::from_model_name("qwen3.5:14b"), ModelTier::Large);
        assert_eq!(ModelTier::from_model_name("qwen3.5:27b-nothink"), ModelTier::Large);
        assert_eq!(ModelTier::from_model_name("qwen3.5:35b"), ModelTier::Large);
    }

    #[test]
    fn tier_detection_huggingface_format() {
        assert_eq!(ModelTier::from_model_name("Qwen3.5-0.6B"), ModelTier::Tiny);
        assert_eq!(ModelTier::from_model_name("Qwen3.5-9B"), ModelTier::Medium);
        assert_eq!(ModelTier::from_model_name("Llama-3.2-3B-Instruct"), ModelTier::Small);
        assert_eq!(ModelTier::from_model_name("Llama-3.2-70B"), ModelTier::Large);
    }

    #[test]
    fn tier_detection_cloud_models() {
        assert_eq!(ModelTier::from_model_name("claude-3-5-sonnet"), ModelTier::Large);
        assert_eq!(ModelTier::from_model_name("gpt-4o"), ModelTier::Large);
        assert_eq!(ModelTier::from_model_name("gemini-pro"), ModelTier::Large);
    }

    #[test]
    fn tier_detection_unknown_defaults_medium() {
        assert_eq!(ModelTier::from_model_name("some-random-model"), ModelTier::Medium);
    }

    #[test]
    fn profile_from_model_name() {
        let tiny = ModelCapabilityProfile::from_model_name("qwen3.5:0.6b");
        assert_eq!(tiny.tier, ModelTier::Tiny);
        assert_eq!(tiny.max_tools_per_prompt, 3);
        assert!(tiny.uses_mcq());
        assert!(!tiny.multi_step_capable);
        assert_eq!(tiny.max_agent_steps, 3);

        let medium = ModelCapabilityProfile::from_model_name("qwen3.5:9b");
        assert_eq!(medium.tier, ModelTier::Medium);
        assert_eq!(medium.max_tools_per_prompt, 8);
        assert_eq!(medium.tool_call_mode, ToolCallMode::StructuredJSON);
        assert!(medium.multi_step_capable);
        assert!(medium.use_family_routing);
        assert_eq!(medium.max_agent_steps, 10);

        let large = ModelCapabilityProfile::from_model_name("qwen3.5:27b-nothink");
        assert_eq!(large.tier, ModelTier::Large);
        assert_eq!(large.max_tools_per_prompt, 15);
        assert!(large.uses_native_tools());
        assert_eq!(large.max_agent_steps, 15);
    }

    #[test]
    fn degraded_profile() {
        let d = ModelCapabilityProfile::degraded();
        assert_eq!(d.tier, ModelTier::Tiny);
        assert_eq!(d.max_tools_per_prompt, 3);
        assert_eq!(d.max_agent_steps, 3);
        assert_eq!(d.max_effective_context, 2048);
        assert!(!d.supports_repair_loop);
    }

    #[test]
    fn tool_family_routing() {
        let families = ToolFamily::route_query("check my email inbox");
        assert!(!families.is_empty());
        assert_eq!(families[0].0, ToolFamily::Communicate);

        let families = ToolFamily::route_query("what's the weather tomorrow");
        assert!(!families.is_empty());
        assert_eq!(families[0].0, ToolFamily::World);

        let families = ToolFamily::route_query("schedule a meeting with Alice");
        assert!(!families.is_empty());
        assert_eq!(families[0].0, ToolFamily::Schedule);

        let families = ToolFamily::route_query("read the config file");
        assert!(!families.is_empty());
        assert_eq!(families[0].0, ToolFamily::Files);
    }

    #[test]
    fn tool_family_best_for_query() {
        assert_eq!(ToolFamily::best_for_query("send email to Bob"), Some(ToolFamily::Communicate));
        assert_eq!(ToolFamily::best_for_query("browse hacker news"), Some(ToolFamily::Browse));
        assert_eq!(ToolFamily::best_for_query("what did I decide about the trip"), Some(ToolFamily::Remember));
        assert_eq!(ToolFamily::best_for_query("run the deploy script"), Some(ToolFamily::System));
    }

    #[test]
    fn tool_family_no_match() {
        // Very generic query — no keywords match
        let families = ToolFamily::route_query("hello how are you");
        assert!(families.is_empty());
    }

    #[test]
    fn gen_config_generation() {
        let p = ModelCapabilityProfile::from_model_name("qwen3.5:9b");
        let tool_cfg = p.tool_gen_config();
        assert_eq!(tool_cfg.max_tokens, 2048);
        assert!((tool_cfg.temperature - 0.2).abs() < 0.01);

        let chat_cfg = p.chat_gen_config();
        assert!((chat_cfg.temperature - 0.7).abs() < 0.01);
    }

    #[test]
    fn profile_summary() {
        let p = ModelCapabilityProfile::from_model_name("qwen3.5:9b");
        let s = p.summary();
        assert!(s.contains("medium"));
        assert!(s.contains("9.0B"));
        assert!(s.contains("StructuredJSON"));
    }

    #[test]
    fn param_extraction_edge_cases() {
        // Model with version number that looks like params
        assert_eq!(ModelTier::from_model_name("qwen3.5:7b"), ModelTier::Medium);
        assert_eq!(ModelTier::from_model_name("phi-4:3.8b"), ModelTier::Small);
    }
}
