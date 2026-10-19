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
            if model.contains("claude") || model.contains("gpt-") || model.contains("gemini")
                || model.contains("MiniMax") || model.contains("minimax") {
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

// ── Model Family ─────────────────────────────────────────────────────

/// Model family determines the chat template and tool calling format.
///
/// Different model families use different formats for tool definitions,
/// tool calls, and tool results. This enum drives the `ChatTemplate` trait
/// selection so the right format is applied per model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelFamily {
    /// Qwen 2.5/3.5 and Yantrik fine-tuned models.
    /// Tool calls: `<tool_call>{"name":..., "arguments":...}</tool_call>`
    /// Tool results: `role: tool`
    Qwen,
    /// NVIDIA Nemotron-3-Nano and variants.
    /// Tool calls: `<tool_call><function=name><parameter=p>v</parameter></function></tool_call>`
    /// Tool results: `role: user` with `<tool_response>` wrapper
    Nemotron,
    /// Llama 3.x and CodeLlama.
    /// Uses OpenAI-compatible function calling format.
    Llama,
    /// Google Gemma 2/3.
    /// Text-based tool calling.
    Gemma,
    /// Microsoft Phi-3/4.
    /// OpenAI-compatible function calling.
    Phi,
    /// OpenAI cloud models (GPT-4, o1, o3).
    /// Native API tool calling — no template needed.
    OpenAI,
    /// Anthropic Claude models.
    /// Native API tool_use/tool_result format.
    Anthropic,
    /// Unknown model — falls back to Qwen ChatML as the most common open format.
    Generic,
}

impl ModelFamily {
    /// Detect model family from a model name string.
    pub fn from_model_name(model: &str) -> Self {
        let lower = model.to_lowercase();

        if lower.contains("qwen") || lower.starts_with("yantrik") {
            ModelFamily::Qwen
        } else if lower.contains("nemotron") {
            ModelFamily::Nemotron
        } else if lower.contains("llama") || lower.contains("codellama") {
            ModelFamily::Llama
        } else if lower.contains("gemma") {
            ModelFamily::Gemma
        } else if lower.contains("phi") {
            ModelFamily::Phi
        } else if lower.contains("gpt-") || lower.contains("o1") || lower.contains("o3") {
            ModelFamily::OpenAI
        } else if lower.contains("claude") {
            ModelFamily::Anthropic
        } else {
            ModelFamily::Generic
        }
    }

    /// Whether this family supports native tool calling through the API provider.
    /// When true, tools are sent via the API's `tools` parameter.
    /// When false, tools must be text-injected into the system prompt.
    pub fn supports_native_tools(&self) -> bool {
        matches!(self, ModelFamily::Qwen | ModelFamily::Nemotron | ModelFamily::Llama
            | ModelFamily::Phi | ModelFamily::OpenAI | ModelFamily::Anthropic)
    }
}

impl std::fmt::Display for ModelFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelFamily::Qwen => write!(f, "qwen"),
            ModelFamily::Nemotron => write!(f, "nemotron"),
            ModelFamily::Llama => write!(f, "llama"),
            ModelFamily::Gemma => write!(f, "gemma"),
            ModelFamily::Phi => write!(f, "phi"),
            ModelFamily::OpenAI => write!(f, "openai"),
            ModelFamily::Anthropic => write!(f, "anthropic"),
            ModelFamily::Generic => write!(f, "generic"),
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
    /// Detected model family (determines chat template format).
    pub family: ModelFamily,
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
    /// assert_eq!(p.max_tools_per_prompt, 10);
    ///
    /// let p = ModelCapabilityProfile::from_model_name("qwen3.5:9b");
    /// assert_eq!(p.tier.to_string(), "medium");
    /// assert_eq!(p.max_tools_per_prompt, 25);
    ///
    /// let p = ModelCapabilityProfile::from_model_name("qwen3.5:27b-nothink");
    /// assert_eq!(p.tier.to_string(), "large");
    /// ```
    pub fn from_model_name(model: &str) -> Self {
        // Yantrik fine-tuned models get a specialized profile — we control the
        // training so we know exactly what capabilities to enable (native tool
        // calling via Qwen3.5 Jinja format, higher tool limits, etc.).
        let lower = model.to_lowercase();
        if lower.starts_with("yantrik-") || lower.starts_with("yantrik:") {
            let params = ModelTier::extract_param_count(model).unwrap_or(9.0);
            return Self::yantrik_trained(model, params);
        }

        let tier = ModelTier::from_model_name(model);
        let params = ModelTier::extract_param_count(model).unwrap_or(match tier {
            ModelTier::Tiny => 0.8,
            ModelTier::Small => 3.0,
            ModelTier::Medium => 9.0,
            ModelTier::Large => 27.0,
        });

        let mut profile = match tier {
            ModelTier::Tiny => Self::tiny(model, params),
            ModelTier::Small => Self::small(model, params),
            ModelTier::Medium => Self::medium(model, params),
            ModelTier::Large => Self::large(model, params),
        };

        // Models with native tool calling support via their API provider.
        // Override StructuredJSON → NativeFunctionCall so Ollama handles the tool template.
        //
        // Only StructuredJSON is promoted. 8d4a1de wrote this override without a
        // guard, so it also caught the Tiny tier: every Qwen/Llama/Phi model — a
        // 0.6B included — came out of `from_model_name` in NativeFunctionCall mode
        // and `uses_mcq()` was false for the whole fleet. MCQ exists precisely
        // because a sub-1.5B model cannot emit a well-formed function call; it is a
        // deliberate downgrade, not a gap waiting to be filled by the provider.
        if profile.family.supports_native_tools()
            && profile.tool_call_mode == ToolCallMode::StructuredJSON
        {
            profile.tool_call_mode = ToolCallMode::NativeFunctionCall;
        }

        profile
    }

    /// Create a profile for degraded/fallback mode (even more constrained than Tiny).
    pub fn degraded() -> Self {
        Self {
            tier: ModelTier::Tiny,
            family: ModelFamily::Generic,
            estimated_params_b: 0.5,
            model_name: "degraded".into(),

            max_tools_per_prompt: 10,
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
            family: ModelFamily::from_model_name(model),
            estimated_params_b: params,
            model_name: model.into(),

            max_tools_per_prompt: 10,
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
            family: ModelFamily::from_model_name(model),
            estimated_params_b: params,
            model_name: model.into(),

            max_tools_per_prompt: 20,
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
            family: ModelFamily::from_model_name(model),
            estimated_params_b: params,
            model_name: model.into(),

            max_tools_per_prompt: 25,
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

    /// Profile for Yantrik fine-tuned models (e.g. `yantrik-9b-v2`, `yantrik-4b`).
    ///
    /// These models are trained with Qwen3.5's native tool calling format (Jinja
    /// template with `tools` parameter), so they get `NativeFunctionCall` mode
    /// regardless of parameter count. Other capabilities scale with size but are
    /// boosted relative to generic models of the same size since the training
    /// targets our exact tool set and conversation patterns.
    fn yantrik_trained(model: &str, params: f64) -> Self {
        let base_tier = ModelTier::from_model_name(model);
        Self {
            tier: base_tier,
            family: ModelFamily::Qwen, // Yantrik models are fine-tuned from Qwen
            estimated_params_b: params,
            model_name: model.into(),

            // Native tool calling — trained on Qwen3.5 Jinja tool format
            max_tools_per_prompt: match base_tier {
                ModelTier::Tiny => 10,
                ModelTier::Small => 20,
                ModelTier::Medium => 25,
                ModelTier::Large => 30,
            },
            tool_call_mode: ToolCallMode::NativeFunctionCall,
            slot_mode: SlotMode::JSON,
            use_family_routing: params < 14.0, // still helpful for smaller models

            max_agent_steps: match base_tier {
                ModelTier::Tiny => 5,
                ModelTier::Small => 8,
                ModelTier::Medium => 12,
                ModelTier::Large => 15,
            },
            supports_repair_loop: true,
            max_repair_attempts: if params >= 4.0 { 2 } else { 1 },
            multi_step_capable: params >= 4.0,

            max_effective_context: match base_tier {
                ModelTier::Tiny => 4096,
                ModelTier::Small => 8192,
                ModelTier::Medium => 32768,
                ModelTier::Large => 65536,
            },
            ambient_context_budget: match base_tier {
                ModelTier::Tiny => 512,
                ModelTier::Small => 2048,
                ModelTier::Medium => 8192,
                ModelTier::Large => 16384,
            },
            max_history_turns: match base_tier {
                ModelTier::Tiny => 3,
                ModelTier::Small => 5,
                ModelTier::Medium => 10,
                ModelTier::Large => 20,
            },

            max_generation_tokens: match base_tier {
                ModelTier::Tiny => 512,
                ModelTier::Small => 1024,
                ModelTier::Medium => 2048,
                ModelTier::Large => 4096,
            },
            tool_temperature: 0.2,
            chat_temperature: 0.6,

            confidence_threshold: 0.7,
            llm_nudge_polish: params >= 4.0,
            can_summarize_freely: params >= 4.0,
            hallucination_firewall: params < 14.0,
        }
    }

    fn large(model: &str, params: f64) -> Self {
        Self {
            tier: ModelTier::Large,
            family: ModelFamily::from_model_name(model),
            estimated_params_b: params,
            model_name: model.into(),

            max_tools_per_prompt: 30,
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

    /// Whether this profile should use batched MCQ tool selection.
    ///
    /// For Tiny and Small models, sending many tools at once overwhelms the model.
    /// Instead, use embedding-ranked batches of 5 tools with MCQ classification.
    pub fn uses_batched_mcq_selection(&self) -> bool {
        self.tier <= ModelTier::Small
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
        // 8d4a1de added `family={}` to the format string and `self.family` as the
        // second argument, but the second placeholder is the `{:.1}B` parameter
        // count. Every argument after the first shifted by one, so the line the
        // companion logs on startup read `medium(~qwenB) family=9 tools=25` — the
        // family where the size belongs and the size where the family belongs.
        // `ModelFamily`'s Display writes a literal, so `{:.1}` silently ignored
        // the precision instead of failing to compile.
        format!(
            "{}(~{:.1}B) family={} tools={} mode={:?} ctx={}K steps={} family_routing={}",
            self.tier,
            self.estimated_params_b,
            self.family,
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
            // No bare "today"/"tomorrow" here. They name a *time*, not a family —
            // they turn up in "will it rain today" and "remind me tomorrow" just
            // as readily as in a calendar query — and with one of them present
            // Schedule tied or beat World on every weather question. A calendar
            // query still routes here on "calendar"/"event"/"meeting"/"agenda".
            //
            // No "recipe"/"automation" either: those belong to World, which owns
            // create_recipe/list_recipes/run_recipe (see `tools()` below).
            // 8d4a1de listed them in both families, so a recipe query was a coin
            // flip between World and a Schedule family holding only calendar tools.
            ToolFamily::Schedule => &[
                "calendar", "event", "meeting", "schedule", "appointment",
                "free time", "busy", "agenda", "cron",
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
                "vault", "password", "credential", "secret", "pin",
            ],
            ToolFamily::Delegate => &[
                "parallel", "simultaneously", "multiple tasks", "spawn",
                "complex", "analyze deeply", "think hard", "claude",
            ],
            ToolFamily::World => &[
                "weather", "temperature", "forecast", "rain", "news",
                "events nearby", "what's happening", "connect", "sync",
                "recipe", "automation", "workflow", "automate",
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
                "vault_store", "vault_get", "vault_list", "vault_delete",
                "vault_generate_password", "vault_set_pin",
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
    ///
    /// The score is the weight of the family terms the query actually used —
    /// multi-word phrases count double, since "look up" or "what's happening"
    /// is far stronger evidence than a single common word.
    ///
    /// It is deliberately *not* divided by `keywords.len()`. That is what this
    /// function did until now, and it meant a family got worse at its own
    /// queries every time someone taught it a new synonym: 8d4a1de added four
    /// terms to World and three to Schedule, and "will it rain today" went from
    /// World (1/9 beats 1/10) to Schedule (1/13 ties 1/13, enum order breaks the
    /// tie) — so `get_weather` stopped being offered for weather questions.
    /// Nothing about a longer vocabulary makes a match less meaningful.
    pub fn route_query(query: &str) -> Vec<(ToolFamily, f64)> {
        let query_lower = query.to_lowercase();
        let mut scores: Vec<(ToolFamily, f64)> = ToolFamily::ALL
            .iter()
            .map(|&family| {
                let score: f64 = family
                    .keywords()
                    .iter()
                    .filter(|kw| query_lower.contains(**kw))
                    .map(|kw| if kw.contains(' ') { 2.0 } else { 1.0 })
                    .sum();
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

    /// VM 542, 16 Oct 2026: a report read the swap log's `steps=15` for Claude as `steps=1`.
    /// A large-tier cloud model must be able to call a tool and then answer from its result.
    #[test]
    fn claude_is_large_native_and_has_room_to_call_a_tool_and_answer() {
        let p = ModelCapabilityProfile::from_model_name("claude-sonnet-5-5");
        assert_eq!(p.tier, ModelTier::Large);
        assert_eq!(p.family, ModelFamily::Anthropic);
        assert!(p.uses_native_tools());
        assert!(p.max_agent_steps > 1, "{}", p.summary());
        assert!(p.summary().contains("steps=15"), "{}", p.summary());
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
        assert_eq!(tiny.max_tools_per_prompt, 10);
        assert!(tiny.uses_mcq());
        assert!(!tiny.multi_step_capable);
        assert_eq!(tiny.max_agent_steps, 3);

        let medium = ModelCapabilityProfile::from_model_name("qwen3.5:9b");
        assert_eq!(medium.tier, ModelTier::Medium);
        assert_eq!(medium.max_tools_per_prompt, 25);
        // 8d4a1de: a family whose provider drives the tool template natively
        // (Qwen here) is promoted StructuredJSON → NativeFunctionCall, so Ollama
        // renders the Jinja tool block instead of us asking for JSON in prose.
        // The Medium *tier* still chooses StructuredJSON; the family overrides it.
        assert_eq!(medium.tool_call_mode, ToolCallMode::NativeFunctionCall);
        // A family without native tool support keeps the tier's own mode.
        let gemma = ModelCapabilityProfile::from_model_name("gemma2:9b");
        assert_eq!(gemma.tier, ModelTier::Medium);
        assert_eq!(gemma.tool_call_mode, ToolCallMode::StructuredJSON);
        assert!(medium.multi_step_capable);
        assert!(medium.use_family_routing);
        assert_eq!(medium.max_agent_steps, 10);

        let large = ModelCapabilityProfile::from_model_name("qwen3.5:27b-nothink");
        assert_eq!(large.tier, ModelTier::Large);
        assert_eq!(large.max_tools_per_prompt, 30);
        assert!(large.uses_native_tools());
        assert_eq!(large.max_agent_steps, 15);
    }

    #[test]
    fn degraded_profile() {
        let d = ModelCapabilityProfile::degraded();
        assert_eq!(d.tier, ModelTier::Tiny);
        assert_eq!(d.max_tools_per_prompt, 10);
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
        // This asked for System, and has since the test was written in f1908ff —
        // but nothing in the query is a System term. "script" is a Files keyword
        // and the tool that runs a saved script, `script_run` ("Run a saved
        // script from workspace"), is a Files tool. Files is where this query
        // should land; System's only run-ish keyword is the phrase "run command",
        // which no one phrases that way. System still ranks second and its tools
        // are still offered when the budget has room — see select_tools_adaptive.
        assert_eq!(ToolFamily::best_for_query("run the deploy script"), Some(ToolFamily::Files));
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
        // Generic 9B Qwen → NativeFunctionCall since 8d4a1de: Qwen's provider
        // renders the tool template itself, so the tier's StructuredJSON is
        // overridden. The summary also has to name the family and the size in
        // the right places — see the argument-order bug fixed in `summary()`.
        let p = ModelCapabilityProfile::from_model_name("qwen3.5:9b");
        let s = p.summary();
        assert!(s.contains("medium"), "{s}");
        assert!(s.contains("9.0B"), "{s}");
        assert!(s.contains("family=qwen"), "{s}");
        assert!(s.contains("NativeFunctionCall"), "{s}");

        // A family with no native tool support keeps the tier's mode.
        let g = ModelCapabilityProfile::from_model_name("gemma2:9b");
        let s = g.summary();
        assert!(s.contains("family=gemma"), "{s}");
        assert!(s.contains("StructuredJSON"), "{s}");

        // Yantrik 9B → NativeFunctionCall
        let y = ModelCapabilityProfile::from_model_name("yantrik-9b-v3");
        let s = y.summary();
        assert!(s.contains("medium"));
        assert!(s.contains("NativeFunctionCall"));
    }

    #[test]
    fn yantrik_trained_profile() {
        let y9b = ModelCapabilityProfile::from_model_name("yantrik-9b-v3");
        assert_eq!(y9b.tier, ModelTier::Medium);
        assert_eq!(y9b.estimated_params_b, 9.0);
        assert!(y9b.uses_native_tools()); // key: native tool calling enabled
        assert_eq!(y9b.max_tools_per_prompt, 25);
        assert!(y9b.multi_step_capable);
        assert!(y9b.use_family_routing);

        // `yantrik-4b` sits exactly on a tier boundary. ModelTier has classified
        // `x < 4.0` as Small since f1908ff, so 4.0B is the *bottom of Medium*,
        // not the top of Small — and yantrik_trained's own `params >= 4.0`
        // switches agree with that. This test was written expecting Small and
        // has therefore never passed. Asserting Medium here is also the safer
        // direction: under-tiering a real model is what makes the OS hand a
        // capable model a tiny model's prompt.
        let y4b = ModelCapabilityProfile::from_model_name("yantrik-4b");
        assert_eq!(y4b.tier, ModelTier::Medium);
        assert!(y4b.uses_native_tools());
        assert_eq!(y4b.max_tools_per_prompt, 25);
        assert!(y4b.multi_step_capable);

        // ...and a model genuinely below the boundary still gets the Small budget.
        let y3b = ModelCapabilityProfile::from_model_name("yantrik-3b");
        assert_eq!(y3b.tier, ModelTier::Small);
        assert!(y3b.uses_native_tools());
        assert_eq!(y3b.max_tools_per_prompt, 20);
        assert!(!y3b.multi_step_capable);

        // Ollama tag format
        let ytag = ModelCapabilityProfile::from_model_name("yantrik:9b-v2");
        assert!(ytag.uses_native_tools());
    }

    #[test]
    fn param_extraction_edge_cases() {
        // Model with version number that looks like params
        assert_eq!(ModelTier::from_model_name("qwen3.5:7b"), ModelTier::Medium);
        assert_eq!(ModelTier::from_model_name("phi-4:3.8b"), ModelTier::Small);
    }
}
