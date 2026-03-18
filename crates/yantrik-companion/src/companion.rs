//! CompanionService — the main agent brain.
//!
//! Ties together YantrikDB (memory), LLMEngine (inference), instincts (drives),
//! urges (action queue), learning (memory extraction), bond tracking,
//! personality evolution, and self-narrative into a single 9-step pipeline.

use yantrikdb_core::YantrikDB;
use yantrik_ml::{
    ChatMessage, GenerationConfig, LLMBackend, ToolCall, ToolCallMode,
    ModelCapabilityProfile, ModelFamily, ToolFamily,
    parse_tool_calls, extract_text_content,
    ChatTemplate, template_for_family,
};

use crate::active_context::ActiveDayContext;
use crate::hallucination_firewall::{
    HallucinationFirewall, FirewallConfig, FirewallAction, GroundTruth,
};
use crate::structured_output::{
    StructuredDecisionParser, StructuredDecisionValidator, ValidationResult, RepairPrompt,
};

use crate::agent_loop::AgentLoop;
use crate::bond::{BondLevel, BondTracker};
use crate::config::CompanionConfig;
use crate::context::{self, ContextSignals};
use crate::evolution::Evolution;
use crate::instincts::{self, Instinct};
use crate::learning;
use crate::memory_evolution;
use crate::narrative::Narrative;
use crate::offline::OfflineResponder;
use crate::proactive::ProactiveEngine;
use crate::sanitize;
use crate::security::SecurityGuard;
use crate::tool_cache::ToolCache;
use crate::tool_traces::ToolTraces;
use crate::tools::{self, PermissionLevel, ToolContext, ToolRegistry, parse_permission};
use crate::types::{AgentResponse, CompanionState, ProactiveMessage};
use crate::resonance::ResonanceEngine;
use crate::query_planner::{self, PlanDecision};
use crate::urges::UrgeQueue;

/// Core tools always included in the LLM prompt — no discover_tools needed for these.
/// These cover the most common user needs. Everything else is discoverable.
/// Tools ALWAYS sent on every request (tiny set — the model's core abilities).
pub const ALWAYS_TOOLS: &[&str] = &[
    "remember", "recall", "discover_tools",
    "run_command", "web_search", "calculate",
    "create_schedule", "create_recipe",
];

/// Minimal tools for fallback/degraded mode — tiny models can't handle many tools.
pub const FALLBACK_TOOLS: &[&str] = &[
    "recall", "remember", "run_command",
];

/// Tool categories for keyword-based routing.
/// Each entry: (category_name, keyword_patterns, tool_names).
pub const TOOL_CATEGORIES: &[(&str, &[&str], &[&str])] = &[
    ("files", &["file", "read", "write", "directory", "folder", "list files", "search file", "find file",
                "create file", "edit file", "delete file", "save", "grep", "glob", "pattern", "regex",
                "replace", "modify file", "change file", "update file", "patch"],
     &["read_file", "write_file", "list_files", "search_files", "edit_file", "grep", "glob"]),
    ("system", &["system", "process", "disk", "cpu", "memory usage", "uptime", "diagnose", "kill"],
     &["system_info", "disk_usage", "list_processes", "diagnose_process", "date_calc"]),
    ("browser", &["browse", "website", "click", "navigate", "open page", "web page", "url", "browser", "screenshot"],
     &["launch_browser", "browse", "browser_snapshot", "browser_scroll",
       "browser_click_element", "browser_type_element", "browser_search",
       "browser_see", "browser_click_xy", "browser_type_xy",
       "browser_cleanup", "browser_status"]),
    ("network", &["fetch", "http", "api", "download", "curl", "request", "network"],
     &["http_fetch", "web_fetch", "network_diagnose"]),
    ("vault", &["vault", "password", "credential", "secret", "login", "pin", "store password", "generate password"],
     &["vault_store", "vault_get", "vault_list", "vault_delete", "vault_generate_password", "vault_set_pin"]),
    ("coder", &["code", "script", "execute", "python", "javascript", "run code", "program", "coding"],
     &["code_execute", "script_write", "script_run", "script_patch", "script_list", "script_read"]),
    ("email", &["email", "inbox", "mail", "send email", "check email", "reply", "forward",
                "unread", "mark read", "archive", "flag", "star", "delete email", "trash", "move email", "folders"],
     &["email_check", "email_list", "email_read", "email_send", "email_reply", "email_reply_all",
       "email_forward", "email_search", "email_mark_read", "email_mark_unread",
       "email_flag", "email_unflag", "email_delete", "email_move", "email_archive", "email_list_folders"]),
    ("calendar", &["calendar", "event", "meeting", "schedule", "appointment", "today"],
     &["calendar_today", "calendar_list_events", "calendar_create_event", "calendar_update_event", "calendar_delete_event"]),
    ("scheduling", &["reminder", "alarm", "schedule", "timer", "cron"],
     &["set_reminder", "create_schedule", "list_schedules"]),
    ("communication", &["telegram", "notify", "send message", "notification"],
     &["telegram_send", "send_notification"]),
    ("life", &["recommend", "suggestion", "preference", "find me", "search for", "best", "top", "nearby", "restaurant", "hotel"],
     &["life_search", "recall_preferences", "save_user_fact", "search_sources", "extract_search_results", "rank_results"]),
    ("memory", &["memory", "memories", "forget", "conflict", "review memory", "stop talking", "don't bring up", "stop tracking", "drop topic"],
     &["memory_stats", "resolve_conflicts", "review_memories", "forget_topic"]),
    ("tasks", &["task", "queue", "todo", "backlog"],
     &["queue_task", "list_tasks", "update_task", "complete_task"]),
    ("recipes", &["recipe", "automation", "workflow", "automate"],
     &["create_recipe", "list_recipes", "run_recipe"]),
    ("weather", &["weather", "temperature", "forecast", "rain", "sunny"],
     &["get_weather"]),
    ("connectors", &["connect", "oauth", "google", "spotify", "sync service"],
     &["list_connections", "connect_service", "sync_service", "disconnect_service"]),
    ("delegation", &["think hard", "complex", "analyze", "deep think", "claude",
                      "parallel", "simultaneously", "at the same time", "agents", "multiple tasks",
                      "spawn", "concurrent"],
     &["claude_think", "claude_code", "spawn_agents"]),
    ("github", &["github", "repo", "repos", "repository", "stars", "github profile", "starred",
                  "open source", "contributions", "forks"],
     &["github_repos", "github_stars", "github_profile"]),
    ("bond", &["bond", "relationship", "trust level"],
     &["check_bond"]),
    ("screenshot", &["screenshot", "capture screen", "screen"],
     &["screenshot"]),
    ("life_management", &["open loops", "commitments", "pending", "unanswered", "overdue",
                           "what's waiting", "what am I forgetting", "unresolved", "snooze",
                           "resolve", "attention", "follow up"],
     &["show_open_loops", "resolve_loop", "snooze_loop"]),
];

/// Flat list of ALL tools that were formerly in CORE_TOOLS (for backwards compat).
pub const CORE_TOOLS: &[&str] = &[
    "remember", "recall", "discover_tools",
    "run_command", "read_file", "write_file", "list_files", "search_files",
    "system_info", "date_calc", "disk_usage",
    "list_processes", "diagnose_process",
    "set_reminder", "create_schedule", "list_schedules",
    "telegram_send", "send_notification",
    "launch_browser", "browse", "browser_snapshot", "browser_scroll",
    "browser_click_element", "browser_type_element", "browser_search",
    "browser_see", "browser_click_xy", "browser_type_xy",
    "web_search",
    "life_search", "recall_preferences", "save_user_fact",
    "search_sources", "extract_search_results", "rank_results",
    "browser_cleanup", "browser_status",
    "http_fetch", "web_fetch", "network_diagnose",
    "calculate", "screenshot",
    "email_check", "email_list", "email_read", "email_send", "email_reply", "email_reply_all",
    "email_forward", "email_search", "email_mark_read", "email_mark_unread",
    "email_flag", "email_unflag", "email_delete", "email_move", "email_archive", "email_list_folders",
    "calendar_today", "calendar_list_events", "calendar_create_event", "calendar_update_event", "calendar_delete_event",
    "memory_stats", "resolve_conflicts", "review_memories", "forget_topic",
    "queue_task", "list_tasks", "update_task", "complete_task",
    "create_recipe", "list_recipes", "run_recipe",
    "claude_think", "claude_code", "spawn_agents",
    "get_weather", "check_bond",
    "list_connections", "connect_service", "sync_service", "disconnect_service",
    "vault_store", "vault_get", "vault_list", "vault_delete", "vault_generate_password", "vault_set_pin",
    "code_execute", "script_write", "script_run", "script_patch", "script_list", "script_read",
];

/// Select tools for a specific query using keyword routing + embedding fallback.
/// Returns a deduplicated list of tool names (ALWAYS_TOOLS + routed + embedding top-K).
/// Target: 8-15 tools per query instead of 83+.
/// Select tools for a query using YantrikDB embedding similarity as the primary
/// method, with keyword categories as a fast boost layer.
///
/// Flow: ALWAYS_TOOLS → keyword boost → YantrikDB cosine similarity (top-K).
/// The embedding layer handles paraphrases and novel phrasing that keywords miss.
pub fn select_tools_for_query(query: &str, db: &YantrikDB, max_extra: usize) -> Vec<&'static str> {
    let mut selected: Vec<&'static str> = ALWAYS_TOOLS.to_vec();
    let budget = max_extra.saturating_sub(selected.len());

    // Layer 1: keyword boost — fast, catches obvious matches
    let query_lower = query.to_lowercase();
    let mut keyword_hits = 0u32;
    for &(_cat_name, keywords, tools) in TOOL_CATEGORIES {
        let hit = keywords.iter().any(|kw| query_lower.contains(kw));
        if hit {
            for tool in tools {
                if !selected.contains(tool) {
                    selected.push(tool);
                }
            }
            keyword_hits += 1;
        }
    }

    // Layer 2: YantrikDB embedding similarity — always runs, fills remaining budget
    // This catches tools that keywords missed (paraphrases, novel phrasing)
    if selected.len() < max_extra {
        let remaining = max_extra.saturating_sub(selected.len());
        let relevant = crate::tool_cache::ToolCache::select_relevant(
            db.conn(), db, query, remaining + 10, // fetch extra to filter dupes
        );
        for def in &relevant {
            if selected.len() >= max_extra {
                break;
            }
            if let Some(name) = def["function"]["name"].as_str() {
                if let Some(&static_name) = CORE_TOOLS.iter().find(|&&t| t == name) {
                    if !selected.contains(&static_name) {
                        selected.push(static_name);
                    }
                }
            }
        }
    }

    tracing::info!(
        query_short = &query[..query.len().min(60)],
        keyword_hits,
        total = selected.len(),
        "Dynamic tool selection (keywords + embeddings)"
    );

    selected
}

/// Adaptive tool selection — embeddings-first with ToolFamily as a boost layer.
///
/// For all tiers: ALWAYS_TOOLS → YantrikDB embedding search → ToolFamily keyword boost.
/// The embedding search in YantrikDB is the primary selector — it handles paraphrases
/// and novel phrasing that keyword routing misses.
pub fn select_tools_adaptive(
    query: &str,
    db: &YantrikDB,
    profile: &ModelCapabilityProfile,
) -> Vec<&'static str> {
    let mut selected: Vec<&'static str> = ALWAYS_TOOLS.to_vec();
    let budget = profile.max_tools_per_prompt;

    // Layer 1: YantrikDB embedding similarity — primary selector
    // Use pure similarity search (no built-in ALWAYS_INCLUDE) so the most relevant
    // tools surface regardless of hardcoded lists. ALWAYS_TOOLS are already in `selected`.
    let embed_limit = budget.saturating_sub(selected.len()) + 5;
    let relevant = crate::tool_cache::ToolCache::select_by_similarity(
        db.conn(), db, query, embed_limit,
    );
    let mut embed_added = 0usize;
    for def in &relevant {
        if selected.len() >= budget {
            break;
        }
        if let Some(name) = def["function"]["name"].as_str() {
            if !selected.iter().any(|&s| s == name) {
                // Find the static &str reference from CORE_TOOLS or TOOL_CATEGORIES
                if let Some(&static_name) = CORE_TOOLS.iter().find(|&&t| t == name) {
                    if !selected.contains(&static_name) {
                        selected.push(static_name);
                        embed_added += 1;
                    }
                }
            }
        }
    }

    // Layer 2: ToolFamily keyword boost — fills gaps that embeddings might miss
    if profile.use_family_routing && selected.len() < budget {
        let families = ToolFamily::route_query(query);
        for (family, _score) in &families {
            if selected.len() >= budget {
                break;
            }
            for &tool_name in family.tools() {
                if selected.len() >= budget {
                    break;
                }
                let is_registered = CORE_TOOLS.contains(&tool_name);
                if is_registered && !selected.contains(&tool_name) {
                    selected.push(tool_name);
                }
            }
        }
    }

    // Log selected tool names for debugging
    let selected_str: Vec<&str> = selected.iter().copied().collect();
    tracing::info!(
        query_short = &query[..query.len().min(60)],
        embed_matched = embed_added,
        total = selected.len(),
        tier = %profile.tier,
        max_budget = budget,
        tools = ?selected_str,
        "Adaptive tool selection (embeddings-first)"
    );

    selected
}

// ── Batched MCQ Tool Selection (for Tiny/Small models) ───────────────

/// MCQ labels for batch tool selection.
const MCQ_LABELS: &[char] = &['A', 'B', 'C', 'D', 'E'];

/// Check if embedding scores are confident enough to auto-select without LLM.
///
/// Returns the tool name if top-1 score is high and margin over top-2 is large.
fn embedding_auto_select(ranked: &[(f32, String, String)]) -> Option<&str> {
    if ranked.len() < 2 {
        return None;
    }
    let (score1, ref name1, _) = ranked[0];
    let (score2, _, _) = ranked[1];
    let margin = score1 - score2;

    if score1 > 0.85 && margin > 0.10 {
        tracing::info!(
            tool = name1.as_str(),
            score = score1,
            margin,
            "Embedding auto-select — skipping LLM"
        );
        Some(name1.as_str())
    } else {
        None
    }
}

/// Build an MCQ prompt for a batch of tools.
///
/// Returns a compact prompt (~300-500 tokens) asking the model to pick A-E or NO_TOOL.
fn build_mcq_prompt(query: &str, batch: &[(f32, String, String)]) -> String {
    let mut prompt = String::with_capacity(512);
    prompt.push_str(
        "Select the best tool for the user's request.\n\
         Output exactly one of: A, B, C, D, E, NO_TOOL\n\n"
    );

    for (i, (_score, _name, card)) in batch.iter().enumerate() {
        if i >= MCQ_LABELS.len() {
            break;
        }
        prompt.push(MCQ_LABELS[i]);
        prompt.push_str(". ");
        prompt.push_str(card);
        prompt.push('\n');
    }

    prompt.push_str("\nUser request: \"");
    prompt.push_str(query);
    prompt.push_str("\"\n\nAnswer:");
    prompt
}

/// Parse an MCQ response to extract the selected label (A-E) or NO_TOOL.
///
/// Returns the index (0-4) of the selected tool, or None for NO_TOOL / invalid.
fn parse_mcq_response(response: &str) -> Option<usize> {
    let trimmed = response.trim();

    // Check for NO_TOOL first
    if trimmed.contains("NO_TOOL") || trimmed.contains("no_tool") || trimmed.contains("NONE") {
        return None;
    }

    // Look for a single letter A-E (possibly with period or parenthesis)
    for ch in trimmed.chars() {
        match ch {
            'A' | 'a' => return Some(0),
            'B' | 'b' => return Some(1),
            'C' | 'c' => return Some(2),
            'D' | 'd' => return Some(3),
            'E' | 'e' => return Some(4),
            _ => continue,
        }
    }

    None
}

/// Run batched MCQ tool selection for small models.
///
/// Sends batches of 5 tools to the LLM as an MCQ prompt. If the model picks a tool,
/// returns its name. If NO_TOOL, advances to the next batch. Max `max_rounds` rounds.
fn mcq_batch_select(
    llm: &dyn LLMBackend,
    query: &str,
    ranked_tools: &[(f32, String, String)],
    gen_config: &GenerationConfig,
    batch_size: usize,
    max_rounds: usize,
) -> Option<String> {
    for round in 0..max_rounds {
        let start = round * batch_size;
        if start >= ranked_tools.len() {
            break;
        }
        let end = (start + batch_size).min(ranked_tools.len());
        let batch = &ranked_tools[start..end];

        if batch.is_empty() {
            break;
        }

        let prompt = build_mcq_prompt(query, batch);

        // Use a tight config for MCQ — we just need 1-2 tokens
        let mcq_config = GenerationConfig {
            max_tokens: 16, // Just need "A" or "NO_TOOL"
            temperature: 0.1, // Low temperature for deterministic selection
            max_context: gen_config.max_context,
            ..Default::default()
        };

        let messages = vec![ChatMessage::user(&prompt)];
        match llm.chat(&messages, &mcq_config, None) {
            Ok(response) => {
                let text = response.text.trim().to_string();
                tracing::info!(
                    round = round + 1,
                    batch_start = start,
                    batch_end = end,
                    response = text.as_str(),
                    "MCQ batch selection round"
                );

                if let Some(idx) = parse_mcq_response(&text) {
                    if idx < batch.len() {
                        let tool_name = batch[idx].1.clone();
                        tracing::info!(
                            tool = tool_name.as_str(),
                            round = round + 1,
                            label = %MCQ_LABELS[idx],
                            "MCQ selected tool"
                        );
                        return Some(tool_name);
                    }
                }
                // NO_TOOL or invalid — continue to next batch
                tracing::debug!(round = round + 1, "MCQ round returned NO_TOOL, trying next batch");
            }
            Err(e) => {
                tracing::warn!(round = round + 1, error = %e, "MCQ batch LLM call failed");
                break;
            }
        }
    }

    tracing::info!(rounds = max_rounds, "MCQ batch selection exhausted — no tool selected");
    None
}

/// Extract tool arguments from a user query for a specific tool.
///
/// For tools with no required parameters, returns empty object.
/// For tools with parameters, uses a simple LLM prompt to extract values.
fn extract_tool_arguments(
    llm: &dyn LLMBackend,
    query: &str,
    tool_name: &str,
    tool_def: &serde_json::Value,
    gen_config: &GenerationConfig,
) -> serde_json::Value {
    let params = &tool_def["function"]["parameters"]["properties"];
    if params.is_null() || params.as_object().map_or(true, |o| o.is_empty()) {
        return serde_json::json!({});
    }

    // Build a compact parameter extraction prompt
    let required = tool_def["function"]["parameters"]["required"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    let params_desc = if let Some(obj) = params.as_object() {
        obj.iter()
            .map(|(k, v)| {
                let desc = v["description"].as_str().unwrap_or("");
                let typ = v["type"].as_str().unwrap_or("string");
                format!("  \"{}\": ({}) {}", k, typ, desc)
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        String::new()
    };

    let prompt = format!(
        "Extract arguments for the tool \"{}\" from the user's request.\n\
         Output ONLY a JSON object with the required fields.\n\n\
         Parameters:\n{}\n\
         Required: {}\n\n\
         User request: \"{}\"\n\n\
         JSON:",
        tool_name, params_desc, required, query
    );

    let extract_config = GenerationConfig {
        max_tokens: 256,
        temperature: 0.1,
        max_context: gen_config.max_context,
        ..Default::default()
    };

    let messages = vec![ChatMessage::user(&prompt)];
    match llm.chat(&messages, &extract_config, None) {
        Ok(response) => {
            // Try to parse JSON from response
            let text = response.text.trim();
            // Find JSON object in response
            if let Some(start) = text.find('{') {
                if let Some(end) = text.rfind('}') {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text[start..=end]) {
                        return v;
                    }
                }
            }
            tracing::warn!(
                tool = tool_name,
                response = text,
                "Failed to parse tool arguments from LLM response"
            );
            serde_json::json!({})
        }
        Err(e) => {
            tracing::warn!(tool = tool_name, error = %e, "Argument extraction LLM call failed");
            serde_json::json!({})
        }
    }
}

/// The companion agent — memory + inference + instincts + bond + evolution in one struct.
pub struct CompanionService {
    pub db: YantrikDB,
    pub llm: std::sync::Arc<dyn LLMBackend>,
    pub config: CompanionConfig,
    pub urge_queue: UrgeQueue,
    instincts: Vec<Box<dyn Instinct>>,

    // Conversation state
    conversation_history: Vec<ChatMessage>,
    last_interaction_ts: f64,
    session_turn_count: usize,
    proactive_message: Option<ProactiveMessage>,

    // Cached from last think()
    pending_triggers: Vec<serde_json::Value>,
    active_patterns: Vec<serde_json::Value>,
    open_conflicts_count: usize,
    recent_valence_avg: Option<f64>,

    // Soul state — cached from DB per interaction
    bond_level: BondLevel,
    bond_score: f64,
    bond_level_changed: bool,

    // Desktop system state — set from SystemObserver via bridge
    system_context: String,

    // Tool registry — modular tool store
    pub(crate) registry: ToolRegistry,

    // Security — self-evolving adaptive defense
    guard: SecurityGuard,

    // Proactive conversation engine — template-based message delivery
    proactive_engine: ProactiveEngine,

    // Cached stable tools system message — prefix-cached by llama.cpp / Ollama.
    // Contains ALL tool definitions. Stays identical across calls so the
    // server's KV cache can reuse it.
    // Used ONLY for non-API backends (candle, llama.cpp in-process).
    tools_system_message: String,

    // Native tools JSON array — ALWAYS_TOOLS definitions (small, stable set).
    // Additional tools selected dynamically per query via select_tools_for_query().
    native_core_tools: Vec<serde_json::Value>,

    // Extra tool names added by Skill Store (always included in per-query selection).
    skill_extra_tools: Vec<String>,

    // Whether to use native OpenAI tool calling format (API backend with --jinja).
    use_native_tools: bool,

    // Model family for family-aware chat templates (tool format, tool results).
    model_family: ModelFamily,

    // Background task manager for long-running processes.
    pub(crate) task_manager: std::sync::Mutex<crate::task_manager::TaskManager>,

    // Event buffer for automation matching (drained during think cycles).
    pub recent_events: Vec<(String, serde_json::Value)>,

    // Incognito mode — when true, no data is persisted from interactions.
    pub incognito: bool,

    // Natural Communication state
    /// Significant events for aftermath instinct: (description, timestamp, reflected)
    pub natural_events: Vec<(String, f64, bool)>,
    /// Running average of user message lengths (for conversational metabolism)
    user_msg_lengths: Vec<usize>,
    /// Proactive messages sent today (reset daily)
    pub daily_proactive_count: u32,
    daily_proactive_reset_ts: f64,
    /// Last N messages sent by companion (for anti-repetition / negative examples)
    pub recent_sent_messages: Vec<String>,
    /// Suppressed urges: (key, reason, timestamp) — for strategic silence reveal
    pub suppressed_urges: Vec<(String, String, f64)>,
    /// Last proactive message context: (text, urge_ids, timestamp)
    /// Used for threading — if user replies within 5 minutes, inject context.
    pub last_proactive_context: Option<(String, Vec<String>, f64)>,

    /// User's known interests — loaded from memory on startup, updated on interaction.
    pub user_interests: Vec<String>,
    /// User's location for local relevance.
    pub user_location: String,

    /// Resonance Engine — mathematical communication priority scoring.
    /// Uses Kuramoto phase dynamics + information theory + social penetration theory.
    pub resonance: ResonanceEngine,

    /// Adaptive User Model — learns interaction patterns to adjust proactive behavior.
    pub user_model: crate::user_model::UserModel,

    /// Context Cortex — cross-system intelligence engine.
    pub cortex: Option<crate::cortex::ContextCortex>,

    /// Playbook Engine — deterministic anticipatory workflows.
    pub playbook_engine: crate::cortex::playbook::PlaybookEngine,

    /// Connector state — OAuth connector manager for external services.
    pub connector_state: Option<std::sync::Arc<std::sync::Mutex<tools::connector::ConnectorState>>>,

    /// Cognitive Event Bus — typed, causal, replayable event system.
    pub event_bus: Option<yantrik_os::EventBus>,

    /// Model Capability Profile — auto-detected from LLM model name.
    /// Controls tool exposure, routing strategy, context budgets, and guardrails.
    pub capability_profile: ModelCapabilityProfile,

    /// Provider Registry — multi-provider LLM management with routing and failover.
    /// When populated, routes requests through registered providers instead of `self.llm`.
    pub provider_registry: Option<std::sync::Arc<yantrik_ml::ProviderRegistry>>,

    /// Active Day Context — ambient awareness buffer (calendar, weather, email, etc.).
    /// Refreshed by stewardship loop, injected into system prompt per token budget.
    pub active_context: ActiveDayContext,

    /// 3-axis trust model (action, personal, taste) — gates autonomous execution.
    pub trust_state: crate::trust_model::TrustState,

    /// Contextual policy engine — predicate-based permission decisions.
    pub policy_engine: crate::policy_engine::PolicyEngine,

    /// Silence policy — learns when to shut up from dismissal patterns.
    pub silence_policy: crate::silence_policy::SilencePolicy,
}

impl CompanionService {
    /// Create a new companion from pre-built YantrikDB and LLM backend.
    pub fn new(db: YantrikDB, llm: std::sync::Arc<dyn LLMBackend>, config: CompanionConfig) -> Self {
        // Ensure soul tables exist
        BondTracker::ensure_tables(db.conn());
        Evolution::ensure_tables(db.conn());
        Narrative::ensure_table(db.conn());

        let urge_queue = UrgeQueue::new(db.conn(), config.urges.clone());
        let instincts = instincts::load_instincts(&config.instincts);
        let mut registry = tools::build_registry(&config);
        let guard = SecurityGuard::new(&db);
        let proactive_engine =
            ProactiveEngine::new(config.proactive.clone(), &config.user_name);

        // Scheduler table
        crate::scheduler::Scheduler::ensure_table(db.conn());

        // Automation table
        crate::automation::AutomationStore::ensure_table(db.conn());

        // Phase 2: Proactive intelligence tables
        ensure_workflow_table(db.conn());
        ensure_maintenance_table(db.conn());

        // Tool reliability metrics table
        crate::tool_metrics::ToolMetrics::ensure_table(db.conn());

        // World model tables (commitments, preferences, routines)
        crate::world_model::WorldModel::ensure_tables(db.conn());

        // Trust model tables (3-axis trust + event log)
        crate::trust_model::TrustModel::ensure_table(db.conn());

        // Silence policy tables (intervention outcomes)
        crate::silence_policy::SilencePolicy::ensure_table(db.conn());

        // Offline NLP + cognitive router tables
        crate::cognitive_router::ensure_tables(db.conn());

        // Tool trace learning table
        ToolTraces::ensure_table(db.conn());

        // Persistent task queue for multi-cycle autonomous work
        crate::task_queue::TaskQueue::ensure_table(db.conn());
        // Recipe engine tables + built-in templates
        crate::recipe::RecipeStore::ensure_tables(db.conn());
        crate::recipe_templates::register_all(db.conn());
        // Calendar local cache
        crate::calendar::ensure_table(db.conn());
        // Vault tables (encrypted credential storage)
        yantrikdb_core::vault::init_tables(db.conn());

        // Memory evolution tables + backfill existing memories
        memory_evolution::ensure_tables(db.conn());
        memory_evolution::ensure_weaving_tables(db.conn());
        memory_evolution::backfill_tiers(db.conn(), &config.memory_evolution);

        // Memory lifecycle + repair tables (contradiction detection, scoping, exclusions)
        crate::memory_lifecycle::MemoryLifecycle::ensure_table(db.conn());
        crate::memory_repair::MemoryRepair::ensure_table(db.conn());

        // Brain loop tables (detectors, curiosity, expectations, baselines)
        yantrikdb_core::cognition::detectors::ensure_detector_tables(db.conn());
        yantrikdb_core::cognition::curiosity::ensure_curiosity_tables(db.conn());
        crate::brain_loop::seed_from_existing_data(db.conn());
        // Curiosity sources are seeded after user_interests/location are loaded (see below)

        // Connector manager — OAuth flows for external services
        // Must be registered before native_core_tools computation.
        let connector_state = {
            let mut mgr = crate::connectors::ConnectorManager::new();
            mgr.register(Box::new(crate::connectors::google::GoogleConnector::new()));
            mgr.register(Box::new(crate::connectors::spotify::SpotifyConnector::new()));
            mgr.register(Box::new(crate::connectors::facebook::FacebookConnector::new()));
            mgr.register(Box::new(crate::connectors::instagram::InstagramConnector::new()));

            let state = tools::connector::ConnectorState {
                manager: mgr,
                config: config.connectors.clone(),
                db_path: config.yantrikdb.db_path.clone(),
                pending_auth: None,
            };
            let arc = std::sync::Arc::new(std::sync::Mutex::new(state));

            tools::connector::register(&mut registry, arc.clone());
            tracing::info!("Connector tools registered (Google, Spotify, Facebook, Instagram)");

            Some(arc)
        };

        // Auto-detect model capability profile from LLM model identifier.
        let capability_profile = if llm.is_degraded() {
            ModelCapabilityProfile::degraded()
        } else {
            ModelCapabilityProfile::from_model_name(llm.model_id())
        };
        tracing::info!(
            profile = %capability_profile.summary(),
            model_id = llm.model_id(),
            "Model capability profile detected"
        );

        // Build stable tools prefix — ALWAYS_TOOLS only (small set for KV caching).
        // Additional tools selected dynamically per query via select_tools_for_query().
        // Full tool set discoverable via discover_tools meta-tool.
        let max_perm = parse_permission(&config.tools.max_permission);
        let use_native_tools = llm.backend_name() == "api" && capability_profile.uses_native_tools();

        // Native tools: only ALWAYS_TOOLS (6 tools) — rest added dynamically per query
        tracing::debug!(always_on = ALWAYS_TOOLS.len(), "Dynamic tool selection initialized");
        let native_core_tools = if config.tools.enabled {
            registry.definitions_for(ALWAYS_TOOLS, max_perm)
        } else {
            Vec::new()
        };

        // Text-injected tools: only for non-API backends (also uses ALWAYS_TOOLS)
        let tools_system_message = if config.tools.enabled && !use_native_tools {
            let core_defs = registry.definitions_for(ALWAYS_TOOLS, max_perm);
            tracing::info!(
                always = core_defs.len(),
                total = registry.definitions(max_perm).len(),
                "Tools prefix: text-injected for {} backend (dynamic selection active)",
                llm.backend_name(),
            );
            template_for_family(capability_profile.family).format_tools(&core_defs)
        } else {
            if config.tools.enabled {
                tracing::info!(
                    always = native_core_tools.len(),
                    total = registry.definitions(max_perm).len(),
                    "Native tool calling: {} always-on + dynamic per-query selection",
                    native_core_tools.len(),
                );
            }
            String::new()
        };

        // Sync tool cache — still used by discover_tools for category metadata
        ToolCache::ensure_table(db.conn());
        if config.tools.enabled {
            let defs = registry.definitions(max_perm);
            ToolCache::sync(db.conn(), &db, &defs);
        }

        // Background task manager
        let mut task_mgr = crate::task_manager::TaskManager::new();
        crate::task_manager::TaskManager::ensure_table(db.conn());
        task_mgr.recover_stale(db.conn());

        // Context Cortex — cross-system intelligence
        let cortex = match crate::cortex::ContextCortex::init_with_services(db.conn(), &config.enabled_services) {
            Ok(c) => {
                tracing::info!(
                    services = ?config.enabled_services,
                    "Context Cortex initialized with enabled services"
                );
                Some(c)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to initialize Context Cortex, continuing without it");
                None
            }
        };

        // Adaptive User Model — init tables and load saved state
        crate::user_model::UserModel::init_db(db.conn());
        let user_model = crate::user_model::UserModel::load(db.conn());

        // Playbook Engine — deterministic anticipatory workflows
        crate::cortex::playbook::PlaybookEngine::init_db(db.conn());
        let mut playbook_engine = crate::cortex::playbook::PlaybookEngine::new();
        crate::cortex::playbook::register_default_playbooks(&mut playbook_engine);
        playbook_engine.load(db.conn());

        // Load current bond state
        let bond_state = BondTracker::get_state(db.conn());

        // Load user interests and location from memory (before db moves)
        let user_interests = load_user_interests(db.conn());
        let user_location = load_user_location(db.conn());

        // Seed curiosity sources from user interests and location
        yantrikdb_core::cognition::curiosity::seed_default_sources(
            db.conn(), &user_interests, &user_location,
        );

        // V25: Load trust state + silence policy history
        let trust_state = crate::trust_model::TrustModel::get_state(db.conn());
        let mut silence_policy = crate::silence_policy::SilencePolicy::new();
        silence_policy.load_history(db.conn());

        Self {
            db,
            llm,
            config,
            urge_queue,
            instincts,
            conversation_history: Vec::new(),
            last_interaction_ts: now_ts(),
            session_turn_count: 0,
            proactive_message: None,
            pending_triggers: Vec::new(),
            active_patterns: Vec::new(),
            open_conflicts_count: 0,
            recent_valence_avg: None,
            bond_level: bond_state.bond_level,
            bond_score: bond_state.bond_score,
            bond_level_changed: false,
            system_context: String::new(),
            registry,
            guard,
            proactive_engine,
            tools_system_message,
            native_core_tools,
            skill_extra_tools: Vec::new(),
            use_native_tools,
            model_family: capability_profile.family,
            task_manager: std::sync::Mutex::new(task_mgr),
            recent_events: Vec::new(),
            incognito: false,
            natural_events: Vec::new(),
            user_msg_lengths: Vec::new(),
            daily_proactive_count: 0,
            daily_proactive_reset_ts: now_ts(),
            recent_sent_messages: Vec::new(),
            suppressed_urges: Vec::new(),
            last_proactive_context: None,
            resonance: ResonanceEngine::new(),
            user_model,
            cortex,
            playbook_engine,
            connector_state,
            user_interests,
            user_location,
            event_bus: None,
            capability_profile,
            active_context: ActiveDayContext::new(),
            trust_state,
            policy_engine: crate::policy_engine::PolicyEngine::new(),
            silence_policy,
            provider_registry: None,
        }
    }

    /// Attach a cognitive event bus for tool execution tracing.
    pub fn set_event_bus(&mut self, bus: yantrik_os::EventBus) {
        self.event_bus = Some(bus);
    }

    /// Get the family-aware chat template for this model.
    fn template(&self) -> Box<dyn ChatTemplate> {
        template_for_family(self.model_family)
    }

    /// Build and attach a ProviderRegistry from the config's `providers` list.
    ///
    /// When a provider registry is active, `get_backend()` routes through it
    /// instead of using `self.llm` directly.
    pub fn init_provider_registry(&mut self) {
        if self.config.llm.providers.is_empty() {
            tracing::debug!("No providers configured, skipping registry init");
            return;
        }

        let registry = yantrik_ml::ProviderRegistry::new();
        let mut primary_id = None;
        let mut fallback_chain = Vec::new();

        for provider_cfg in &self.config.llm.providers {
            let model = provider_cfg.model.as_deref().unwrap_or("default");
            let backend = yantrik_ml::ProviderRegistry::create_backend(
                &provider_cfg.provider_type,
                &provider_cfg.base_url,
                provider_cfg.api_key.as_deref(),
                model,
            );

            let entry = yantrik_ml::RegisteredProvider::new(
                backend,
                &provider_cfg.provider_type,
                &provider_cfg.name,
            );

            registry.register(&provider_cfg.id, entry);

            if provider_cfg.is_primary && primary_id.is_none() {
                primary_id = Some(provider_cfg.id.clone());
            }
            if provider_cfg.is_fallback {
                fallback_chain.push(provider_cfg.id.clone());
            }
        }

        if let Some(ref pid) = primary_id {
            registry.set_primary(pid);
        }
        if !fallback_chain.is_empty() {
            registry.set_fallback_chain(fallback_chain);
        }

        tracing::info!(
            providers = registry.len(),
            primary = ?primary_id,
            "Provider registry initialized"
        );

        self.provider_registry = Some(std::sync::Arc::new(registry));
    }

    /// Get the active LLM backend — either from the provider registry or the direct llm field.
    ///
    /// When a provider registry is active, returns it as an Arc<dyn LLMBackend>
    /// (ProviderRegistry implements LLMBackend). Otherwise returns self.llm.
    pub fn get_backend(&self) -> std::sync::Arc<dyn LLMBackend> {
        if let Some(ref registry) = self.provider_registry {
            registry.clone() as std::sync::Arc<dyn LLMBackend>
        } else {
            self.llm.clone()
        }
    }

    /// Hot-swap a provider in the registry at runtime.
    ///
    /// Used for runtime reconfiguration (e.g. user changes API key in settings).
    pub fn hot_swap_provider(
        &self,
        provider_id: &str,
        provider_type: &str,
        base_url: &str,
        api_key: Option<&str>,
        model: &str,
        display_name: &str,
    ) {
        if let Some(ref registry) = self.provider_registry {
            let backend = yantrik_ml::ProviderRegistry::create_backend(
                provider_type,
                base_url,
                api_key,
                model,
            );
            registry.hot_swap(provider_id, backend, display_name, provider_type);
            tracing::info!(
                provider = %provider_id,
                model = %model,
                "Provider hot-swapped"
            );
        }
    }

    /// Apply a Skill Store snapshot — merges skill-derived services with config,
    /// filters instincts, extends core tools based on enabled skills.
    pub fn apply_skill_snapshot(&mut self, snapshot: &crate::skills::SkillSnapshot) {
        // 1. Merge cortex services: config services + skill-derived services
        if let Some(ref mut cortex) = self.cortex {
            let mut merged: Vec<String> = self.config.enabled_services.clone();
            for svc in &snapshot.enabled_services {
                if !merged.iter().any(|s| s.eq_ignore_ascii_case(svc)) {
                    merged.push(svc.clone());
                }
            }
            cortex.set_services(&merged);
            tracing::info!(
                config_services = ?self.config.enabled_services,
                skill_services = ?snapshot.enabled_services,
                merged = ?merged,
                "Cortex services merged from config + Skill Store"
            );
        }

        // 2. Filter instincts — keep only those whose name matches an enabled instinct ID
        //    (or that are not gated by skills at all, i.e., core instincts)
        if !snapshot.enabled_instincts.is_empty() {
            let before = self.instincts.len();
            self.instincts.retain(|inst| {
                let name = inst.name();
                // Core instincts that are always on (scheduler, automation, bond, etc.)
                let is_core = matches!(
                    name,
                    // Exact name() values from each instinct impl
                    "scheduler" | "automation" | "BondMilestone" | "SelfAwareness"
                    | "morning_brief" | "activity_reflector" | "serendipity"
                    | "Aftermath" | "SocraticSpark" | "EveningReflection"
                    | "ConversationalCallback" | "SilenceReveal"
                    | "check_in" | "emotional_awareness" | "follow_up"
                    | "reminder" | "pattern_surfacing" | "conflict_alerting"
                    | "MemoryWeaver"
                    | "predictive_workflow" | "routine" | "cognitive_load" | "smart_updates"
                );
                is_core || snapshot.enabled_instincts.contains(name)
            });
            tracing::info!(
                before,
                after = self.instincts.len(),
                enabled = ?snapshot.enabled_instincts,
                "Instincts filtered by Skill Store"
            );
        }

        // 3. Store extra tool names from skills — included in per-query dynamic selection
        if !snapshot.extra_core_tools.is_empty() {
            self.skill_extra_tools = snapshot.extra_core_tools.clone();
            tracing::info!(
                extra = ?snapshot.extra_core_tools,
                "Skill Store extra tools registered for dynamic selection"
            );
        }
    }

    /// Toggle incognito mode (no data persistence).
    pub fn set_incognito(&mut self, enabled: bool) {
        self.incognito = enabled;
        tracing::info!(enabled, "Incognito mode toggled");
    }

    /// Whether incognito mode is active.
    pub fn is_incognito(&self) -> bool {
        self.incognito
    }

    /// Buffer a system event for automation matching during think cycles.
    pub fn push_event(&mut self, event_type: &str, event_data: serde_json::Value) {
        // Keep buffer bounded (last 50 events)
        if self.recent_events.len() >= 50 {
            self.recent_events.drain(0..25);
        }
        self.recent_events.push((event_type.to_string(), event_data));
    }

    /// Drain buffered events (called during think cycle).
    pub fn drain_events(&mut self) -> Vec<(String, serde_json::Value)> {
        std::mem::take(&mut self.recent_events)
    }

    /// Execute a tool directly (bypassing LLM). Used by the recipe engine for Tool steps.
    pub fn execute_tool_direct(&self, tool_name: &str, args: &serde_json::Value) -> String {
        let ctx = ToolContext {
            db: &self.db,
            max_permission: PermissionLevel::Standard,
            registry_metadata: None,
            task_manager: Some(&self.task_manager),
            incognito: self.incognito,
            agent_spawner: None,
        };
        self.registry.execute(&ctx, tool_name, args)
    }

    /// Execute a recipe synchronously and return the final answer.
    /// Used by the query planner when it decides a complex query needs a recipe.
    fn execute_recipe_sync(
        &mut self,
        user_text: &str,
        steps: &[crate::recipe::RecipeStep],
        goal: &str,
    ) -> Option<AgentResponse> {
        use crate::recipe::RecipeStep;
        use std::collections::HashMap;

        let mut vars: HashMap<String, String> = HashMap::new();
        let mut tool_calls_made: Vec<String> = Vec::new();
        let mut notify_messages: Vec<String> = Vec::new();

        for (i, step) in steps.iter().enumerate() {
            match step {
                RecipeStep::Tool { tool_name, args, store_as, .. } => {
                    // Resolve {{var}} references in args
                    let args_str = serde_json::to_string(args).unwrap_or_default();
                    let resolved_args_str = resolve_template_vars(&args_str, &vars);
                    let resolved_args: serde_json::Value =
                        serde_json::from_str(&resolved_args_str).unwrap_or(args.clone());

                    tracing::info!(
                        step = i, tool = tool_name.as_str(),
                        "QueryPlanner: executing tool step"
                    );
                    let result = self.execute_tool_direct(tool_name, &resolved_args);
                    tool_calls_made.push(tool_name.clone());

                    // Truncate large results to save token budget
                    let truncated = if result.len() > 2000 {
                        format!("{}...(truncated)", &result[..result.floor_char_boundary(2000)])
                    } else {
                        result
                    };
                    vars.insert(store_as.clone(), truncated);
                }
                RecipeStep::Think { prompt, store_as } => {
                    // Resolve variables in prompt
                    let resolved_prompt = resolve_template_vars(prompt, &vars);

                    // Build a focused synthesis prompt
                    let messages = vec![
                        ChatMessage::system(&format!(
                            "You are {}, a personal AI companion. Answer based ONLY on the \
                             provided data. Never invent prices, ratings, or availability. \
                             If data is missing, say so.",
                            self.config.personality.name
                        )),
                        ChatMessage::user(&format!(
                            "User asked: \"{}\"\n\n{}", user_text, resolved_prompt
                        )),
                    ];

                    let gen_config = GenerationConfig {
                        max_tokens: self.config.llm.max_tokens,
                        temperature: self.config.llm.temperature,
                        ..Default::default()
                    };

                    match self.llm.chat(&messages, &gen_config, None) {
                        Ok(r) => {
                            let text = extract_text_content(&r.text);
                            let text = if text.is_empty() { r.text } else { text };
                            let text = strip_think_tags(&text);
                            vars.insert(store_as.clone(), text);
                        }
                        Err(e) => {
                            tracing::warn!("QueryPlanner Think step failed: {e}");
                            vars.insert(store_as.clone(), String::new());
                        }
                    }
                }
                RecipeStep::Notify { message } => {
                    let resolved = resolve_template_vars(message, &vars);
                    notify_messages.push(resolved);
                }
                _ => {} // JumpIf/WaitFor not used in planner recipes
            }
        }

        // Get the final answer from the last Think step
        let final_answer = vars.get("final_answer").cloned().unwrap_or_default();
        if final_answer.is_empty() {
            tracing::warn!("QueryPlanner: recipe produced no final answer, falling back");
            return None;
        }

        // Update conversation history
        self.conversation_history.push(ChatMessage::user(user_text));
        self.conversation_history.push(ChatMessage::assistant(&final_answer));
        self.compress_history_if_needed();
        self.last_interaction_ts = now_ts();
        self.session_turn_count += 1;

        // Learning + bond
        if !self.incognito {
            let smart = if self.config.memory_evolution.smart_recall_enabled {
                memory_evolution::smart_recall(&self.db, user_text, &self.config.memory_evolution)
            } else {
                let mems = self.db.recall_text(user_text, 5).unwrap_or_default();
                memory_evolution::SmartRecallResult::from_primary(mems)
            };
            let memories = smart.all_unique();

            let clean = sanitize::clean_response_for_learning(&final_answer, &tool_calls_made);
            learning::extract_and_learn(
                &self.db, &*self.llm, user_text, &clean, &self.config.memory_evolution,
            );
            memory_evolution::update_conversation_context(self.db.conn(), user_text, &memories);
            if self.config.bond.enabled {
                let (new_level, level_changed) = BondTracker::score_interaction(
                    self.db.conn(), user_text, &final_answer, memories.len(),
                );
                self.bond_level = new_level;
                self.bond_level_changed = level_changed;
            }
        }

        Some(AgentResponse {
            message: final_answer,
            memories_recalled: 0,
            urges_delivered: vec![],
            tool_calls_made,
            offline_mode: false,
        })
    }

    /// The 9-step message pipeline.
    pub fn handle_message(&mut self, user_text: &str) -> AgentResponse {
        // Step 0: SecurityGuard — check user input for injection
        if let Some(warning) = self.guard.check_input(user_text, &self.db) {
            return AgentResponse {
                message: warning,
                memories_recalled: 0,
                urges_delivered: vec![],
                tool_calls_made: vec![],
                offline_mode: false,
            };
        }

        // Step 0.5: Query Planner — ask LLM if this needs a recipe
        // Only for medium+ tier models with tools enabled and non-trivial queries
        if self.config.tools.enabled
            && !self.llm.is_degraded()
            && self.capability_profile.tier >= yantrik_ml::ModelTier::Medium
            && user_text.split_whitespace().count() > 5
        {
            // Get a small set of tool names for the planner prompt
            let planner_tools: Vec<&str> = select_tools_adaptive(
                user_text, &self.db, &self.capability_profile,
            ).into_iter().take(12).collect();

            let decision = query_planner::plan_or_direct(
                &*self.llm, user_text, &planner_tools,
            );

            if let PlanDecision::Recipe { ref goal, ref steps } = decision {
                tracing::info!(
                    goal = goal.as_str(),
                    steps = steps.len(),
                    "QueryPlanner: routing to recipe execution"
                );
                if let Some(response) = self.execute_recipe_sync(user_text, steps, goal) {
                    return response;
                }
                // If recipe execution failed, fall through to normal pipeline
                tracing::warn!("QueryPlanner: recipe failed, falling back to normal pipeline");
            }
        }

        // Step 1: Check session timeout
        self.check_session_timeout();

        // Step 2: Smart multi-signal recall (Gap 1+2)
        let smart = if self.config.memory_evolution.smart_recall_enabled {
            memory_evolution::smart_recall(&self.db, user_text, &self.config.memory_evolution)
        } else {
            let mems = self.db.recall_text(user_text, 5).unwrap_or_default();
            memory_evolution::SmartRecallResult::from_primary(mems)
        };
        let mut memories = smart.all_unique();
        let (recall_confidence, recall_hint) = (smart.confidence, smart.hint);

        // Step 2b: Always include identity anchor memories (name, website, GitHub, etc.)
        // These are high-importance facts that should always be available regardless of query.
        {
            let existing_rids: std::collections::HashSet<String> =
                memories.iter().map(|m| m.rid.clone()).collect();
            let identity_facts = self.recall_identity_facts();
            for fact in identity_facts {
                if !existing_rids.contains(&fact.rid) {
                    memories.push(fact);
                }
            }
        }

        // Step 3: Recall self-memories (reflections about the companion itself)
        let self_memories = self
            .db
            .recall_text(&format!("self: {user_text}"), 10)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| {
                r.source == "self" || r.domain == "self-reflection"
            })
            .take(3)
            .collect::<Vec<_>>();

        // Step 4: Pop urges for this interaction
        let urges = self
            .urge_queue
            .pop_for_interaction(self.db.conn(), 2);
        let urge_ids: Vec<String> = urges.iter().map(|u| u.urge_id.clone()).collect();

        // Detect humor reaction from previous exchange
        if !self.incognito {
            learning::detect_humor_reaction(self.db.conn(), user_text);
        }

        // Step 5: Evaluate instincts on interaction
        let state = self.build_state();
        for instinct in &self.instincts {
            let specs = instinct.on_interaction(&state, user_text);
            for spec in specs {
                self.urge_queue.push(self.db.conn(), &spec);
            }
        }

        // Step 6: Build LLM context — lightweight for degraded mode, full otherwise
        let degraded = self.llm.is_degraded();
        if degraded {
            tracing::info!("LLM degraded — using lightweight prompt and minimal tools");
        }

        let context_messages = if degraded {
            context::build_messages_lightweight(
                user_text, &self.config, &memories, &self.conversation_history,
            )
        } else {
            let personality = self.db.get_personality().ok();
            let patterns_json: Vec<serde_json::Value> = self
                .active_patterns.iter().cloned().collect();
            let narrative_text = Narrative::get(self.db.conn());
            let style = Evolution::get_style(self.db.conn());
            let opinions = Evolution::get_opinions(self.db.conn(), 3);
            let shared_refs = if self.config.memory_evolution.reference_freshness_enabled {
                memory_evolution::get_fresh_references(self.db.conn(), 3)
            } else {
                Evolution::get_shared_references(self.db.conn(), 3)
            };
            // CK-5 cognitive awareness injection
            let ck5_text = if self.config.ck5.enabled {
                let snippet = crate::ck5_integration::build_context_snippet(&self.db);
                if snippet.is_empty() {
                    tracing::debug!("CK-5 context snippet: empty");
                    None
                } else {
                    let formatted = snippet.format_for_prompt(300);
                    tracing::debug!(ck5_prompt = %formatted, "CK-5 context injected into system prompt");
                    Some(formatted)
                }
            } else {
                None
            };
            let signals = ContextSignals {
                self_memories: &self_memories,
                narrative: &narrative_text,
                style: &style,
                opinions: &opinions,
                shared_refs: &shared_refs,
                system_state: &self.system_context,
                recall_confidence,
                recall_hint: recall_hint.as_deref(),
                ck5_awareness: ck5_text,
            };
            context::build_messages(
                user_text, &self.config, &state, &memories, &urges,
                &patterns_json, &self.conversation_history,
                personality.as_ref(), Some(&signals), self.use_native_tools,
            )
        };

        // Build message array — single system message (Qwen3.5 requires it):
        // [0] system: context (+ text-injected tools for non-API backends)
        // [1..N-1] conversation history
        // [N] user query
        let max_perm = parse_permission(&self.config.tools.max_permission);
        let mut messages = Vec::with_capacity(context_messages.len() + 1);

        // Dynamic tool selection — adaptive based on model capability profile
        let active_profile = if degraded {
            ModelCapabilityProfile::degraded()
        } else {
            self.capability_profile.clone()
        };

        let word_count = user_text.split_whitespace().count();
        let needs_tools = self.config.tools.enabled && word_count > 2;
        tracing::info!(
            tools_enabled = self.config.tools.enabled,
            word_count,
            needs_tools,
            degraded,
            tier = %active_profile.tier,
            max_tools = active_profile.max_tools_per_prompt,
            "Tool selection gate"
        );

        // ── Batched MCQ tool selection for Tiny/Small models ──────────────
        // Instead of sending 20+ tools to the LLM, use embedding-ranked batches
        // of 5 tools with MCQ classification. Much more reliable for small models.
        let mcq_result: Option<(String, serde_json::Value, String)> = // (tool_name, args, tool_result)
            if !degraded && needs_tools && active_profile.uses_batched_mcq_selection() {
                let gen_config_mcq = GenerationConfig {
                    max_tokens: self.config.llm.max_tokens.min(active_profile.max_generation_tokens),
                    temperature: self.config.llm.temperature,
                    max_context: Some(self.config.llm.max_context_tokens),
                    ..Default::default()
                };

                let ranked = ToolCache::select_ranked_with_scores(
                    self.db.conn(), &self.db, user_text, 15,
                );

                tracing::info!(
                    ranked_count = ranked.len(),
                    top_score = ranked.first().map(|r| r.0).unwrap_or(0.0),
                    tier = %active_profile.tier,
                    "MCQ batched tool selection — ranking tools"
                );

                // Tier 1: Auto-select if embeddings are highly confident
                let selected_tool = if let Some(auto_name) = embedding_auto_select(&ranked) {
                    Some(auto_name.to_string())
                } else {
                    // Tier 2: MCQ batch selection
                    mcq_batch_select(&*self.llm, user_text, &ranked, &gen_config_mcq, 5, 3)
                };

                if let Some(tool_name) = selected_tool {
                    // Get the full tool definition for argument extraction
                    let tool_defs = self.registry.definitions_for(&[tool_name.as_str()], max_perm);
                    let tool_def = tool_defs.first().cloned().unwrap_or(serde_json::json!({}));

                    // Extract arguments (separate LLM call for tools with params)
                    let args = extract_tool_arguments(
                        &*self.llm, user_text, &tool_name, &tool_def, &gen_config_mcq,
                    );

                    // Execute the tool
                    let ctx = ToolContext {
                        db: &self.db,
                        max_permission: max_perm,
                        registry_metadata: None,
                        task_manager: Some(&self.task_manager),
                        incognito: self.incognito,
                        agent_spawner: None,
                    };
                    let result = self.registry.execute(&ctx, &tool_name, &args);
                    tracing::info!(
                        tool = tool_name.as_str(),
                        result_len = result.len(),
                        "MCQ tool executed successfully"
                    );
                    Some((tool_name, args, result))
                } else {
                    None
                }
            } else {
                None
            };

        // If MCQ selected and executed a tool, generate final response with tool result
        if let Some((ref tool_name, ref _args, ref tool_result)) = mcq_result {
            let mut tool_calls_made = vec![tool_name.clone()];

            // Build context messages + tool result for final LLM response
            let mut final_messages = context_messages.clone();
            final_messages.push(ChatMessage::assistant(&format!(
                "I'll use the {} tool to help with that.", tool_name
            )));
            final_messages.push(ChatMessage::user(&format!(
                "Tool {} returned:\n{}\n\nBased on this result, respond to the user's request: \"{}\"",
                tool_name, tool_result, user_text
            )));

            let gen_config = GenerationConfig {
                max_tokens: self.config.llm.max_tokens.min(active_profile.max_generation_tokens),
                temperature: self.config.llm.temperature,
                max_context: Some(self.config.llm.max_context_tokens),
                ..Default::default()
            };

            let mut response_text = match self.llm.chat(&final_messages, &gen_config, None) {
                Ok(r) => {
                    let text = extract_text_content(&r.text);
                    if text.is_empty() { r.text } else { text }
                }
                Err(_) => format!("Here's what I found: {}", &tool_result[..tool_result.len().min(500)]),
            };

            // Clean up response
            response_text = extract_text_content(&response_text);
            response_text = strip_think_tags(&response_text);
            response_text = self.guard.check_response(&response_text, &self.db);

            if response_text.is_empty() {
                response_text = "I'm here. How can I help?".to_string();
            }

            // Update conversation history
            self.conversation_history.push(ChatMessage::user(user_text));
            self.conversation_history.push(ChatMessage::assistant(&response_text));
            self.compress_history_if_needed();
            self.last_interaction_ts = now_ts();
            self.session_turn_count += 1;

            // Learning + bond (skip in incognito)
            if !self.incognito {
                let clean_response = sanitize::clean_response_for_learning(
                    &response_text, &tool_calls_made,
                );
                learning::extract_and_learn(
                    &self.db, &*self.llm, user_text, &clean_response,
                    &self.config.memory_evolution,
                );
                memory_evolution::update_conversation_context(self.db.conn(), user_text, &memories);
                if self.config.bond.enabled {
                    let (new_level, level_changed) = BondTracker::score_interaction(
                        self.db.conn(), user_text, &response_text, memories.len(),
                    );
                    self.bond_level = new_level;
                    self.bond_level_changed = level_changed;
                }
                // Record tool trace for learning
                let trace_chain = vec![serde_json::json!({"tool": tool_name, "status": "success"})];
                ToolTraces::record(
                    self.db.conn(), &self.db, user_text,
                    &trace_chain, "success",
                );
            }

            return AgentResponse {
                message: response_text,
                memories_recalled: memories.len(),
                urges_delivered: urge_ids,
                tool_calls_made,
                offline_mode: false,
            };
        }

        // ── Standard tool selection path (Medium/Large models) ────────────
        let mut selected_tool_names: Vec<&str> = if degraded {
            FALLBACK_TOOLS.to_vec()
        } else if needs_tools {
            select_tools_adaptive(user_text, &self.db, &active_profile)
        } else {
            ALWAYS_TOOLS.to_vec()
        };

        // Build native tools array from dynamically selected + skill extra tools
        let mut native_tools: Vec<serde_json::Value> = if self.use_native_tools {
            let mut defs = self.registry.definitions_for(&selected_tool_names, max_perm);
            // Add skill extra tools by name (String-based lookup)
            if !self.skill_extra_tools.is_empty() {
                let extra_refs: Vec<&str> = self.skill_extra_tools.iter()
                    .filter(|s| !selected_tool_names.contains(&s.as_str()))
                    .map(|s| s.as_str())
                    .collect();
                defs.extend(self.registry.definitions_for(&extra_refs, max_perm));
            }
            defs
        } else {
            Vec::new()
        };

        // For non-API backends: text-inject selected tools into system message
        if !self.use_native_tools && self.config.tools.enabled {
            let selected_defs = self.registry.definitions_for(&selected_tool_names, max_perm);
            let tools_text = self.template().format_tools(&selected_defs);
            if !tools_text.is_empty() {
                if let Some(first) = context_messages.first() {
                    let combined = format!("{}\n\n{}", tools_text, first.content);
                    messages.push(ChatMessage::system(&combined));
                    messages.extend_from_slice(&context_messages[1..]);
                } else {
                    messages.push(ChatMessage::system(&tools_text));
                }
            } else {
                messages.extend(context_messages.clone());
            }
        }

        // For API backend or when no text injection happened: use context messages directly
        if messages.is_empty() {
            messages.extend(context_messages);
        }

        // Tool chain learning: inject trace hints into system prompt (skip in degraded mode)
        if !degraded && self.config.agent.trace_learning && self.config.tools.enabled {
            let hints = ToolTraces::find_similar(
                self.db.conn(), &self.db, user_text, 3,
                self.config.agent.trace_min_similarity,
            );
            if !hints.is_empty() {
                let hint_text = ToolTraces::format_hints(&hints);
                if let Some(sys_msg) = messages.first_mut() {
                    sys_msg.content.push_str(&hint_text);
                }
                for hint in &hints {
                    ToolTraces::mark_used(self.db.conn(), &hint.trace_id);
                }
            }
        }

        // Inject Active Day Context into system prompt (budget from capability profile)
        if active_profile.ambient_context_budget > 0 {
            self.active_context.prune_stale();
            if let Some(context_block) = self.active_context.build_context_block(
                active_profile.ambient_context_budget,
            ) {
                if let Some(sys_msg) = messages.first_mut() {
                    sys_msg.content.push_str("\n\n");
                    sys_msg.content.push_str(&context_block);
                }
                tracing::debug!(
                    sections = self.active_context.section_count(),
                    budget = active_profile.ambient_context_budget,
                    "Injected active day context"
                );
            }
        }

        // Step 7: Call LLM with robust agent loop
        // Generation config adapts to model capability profile
        let gen_config = if degraded {
            active_profile.tool_gen_config()
        } else {
            // Use profile-recommended config, but respect user overrides from config.yaml
            GenerationConfig {
                max_tokens: self.config.llm.max_tokens.min(active_profile.max_generation_tokens),
                temperature: self.config.llm.temperature,
                top_p: Some(0.9),
                max_context: Some(self.config.llm.max_context_tokens),
                ..Default::default()
            }
        };

        let mut tool_calls_made = Vec::new();
        let mut injected_tool_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut response_text = String::new();
        let mut is_offline = false;
        let max_nudges = if degraded { 0 } else { self.config.agent.max_nudges };
        let mut agent_loop = AgentLoop::new(user_text, max_nudges);

        // Build AgentSpawnerContext for parallel sub-agent tool
        let agent_spawner = Some(tools::AgentSpawnerContext {
            llm: self.llm.clone(),
            db_path: self.config.yantrikdb.db_path.clone(),
            embedding_dim: self.config.yantrikdb.embedding_dim,
            max_steps: 10,
            max_tokens: self.config.llm.max_tokens,
            temperature: self.config.llm.temperature,
            user_name: self.config.user_name.clone(),
            config: self.config.clone(),
        });

        // Emit UserMessage event and capture trace for tool call linking
        let msg_trace = self.event_bus.as_ref().map(|bus| {
            bus.emit(
                yantrik_os::EventKind::UserMessage {
                    text: user_text.chars().take(500).collect(),
                    source: "handle_message".into(),
                },
                yantrik_os::EventSource::UserInterface,
            )
        });

        // Discovery rounds are limited; actual tool rounds reset the counter.
        // Max rounds adapt to model capability — smaller models get fewer steps.
        let mut discovery_budget = if degraded { 0 } else { self.config.tools.max_tool_rounds };
        let max_total_rounds = if degraded { 3 } else {
            // Use the minimum of config max_steps and profile max_agent_steps
            self.config.agent.max_steps.min(active_profile.max_agent_steps).max(3)
        };

        for _round in 0..max_total_rounds {
            // Compute tools_param each iteration — native_tools may grow via discover_tools
            let tools_param: Option<&[serde_json::Value]> = if self.use_native_tools && !native_tools.is_empty() {
                Some(&native_tools)
            } else {
                None
            };
            let llm_response = match self.llm.chat(&messages, &gen_config, tools_param) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("LLM offline: {e:#}");
                    response_text = OfflineResponder::respond(
                        &self.db,
                        user_text,
                        &self.system_context,
                        &memories,
                        &urges,
                        &self.config.user_name,
                    );
                    is_offline = true;
                    agent_loop.fail("LLM offline");
                    break;
                }
            };

            // Use native tool_calls if available, fall back to text parsing.
            // For StructuredJSON mode: parse via StructuredDecisionParser first.
            let tool_calls: Vec<ToolCall> = if !llm_response.tool_calls.is_empty() {
                llm_response.tool_calls.clone()
            } else if active_profile.tool_call_mode == ToolCallMode::StructuredJSON {
                // Structured Decision Protocol: parse JSON decision from LLM output
                match StructuredDecisionParser::parse(&llm_response.text) {
                    Ok(decision) => {
                        // Validate the decision
                        let known_tools: Vec<&str> = selected_tool_names.iter().copied().collect();
                        let validation = StructuredDecisionValidator::validate(
                            &decision,
                            Some(&known_tools),
                            active_profile.confidence_threshold,
                        );
                        match validation {
                            ValidationResult::Valid | ValidationResult::Repairable(_) => {
                                if let Some(tc) = decision.to_tool_call() {
                                    tracing::debug!(
                                        tool = %tc.name,
                                        confidence = decision.confidence,
                                        family = %decision.family,
                                        "Structured decision → tool call"
                                    );
                                    vec![tc]
                                } else if decision.has_answer() {
                                    // Direct answer — no tool calls needed
                                    response_text = decision.answer.clone();
                                    Vec::new()
                                } else {
                                    Vec::new()
                                }
                            }
                            ValidationResult::Invalid(issues) => {
                                // Repair attempt: inject repair prompt and retry
                                if active_profile.supports_repair_loop {
                                    let repair = RepairPrompt::build(&decision, &issues);
                                    tracing::debug!(
                                        issues = issues.len(),
                                        "Structured decision invalid — injecting repair prompt"
                                    );
                                    messages.push(ChatMessage::assistant(&llm_response.text));
                                    messages.push(ChatMessage::user(&repair));
                                    continue; // Retry with repair prompt
                                }
                                // No repair loop — fall back to text parsing
                                parse_tool_calls(&llm_response.text)
                            }
                        }
                    }
                    Err(_) => {
                        // Structured parse failed — fall back to standard tool call parsing
                        parse_tool_calls(&llm_response.text)
                    }
                }
            } else {
                parse_tool_calls(&llm_response.text)
            };

            if tool_calls.is_empty() {
                // Nudge on empty: if response is weak and we have budget, push LLM to try harder
                if self.config.agent.nudge_on_empty && self.config.tools.enabled {
                    if let Some(nudge) = agent_loop.maybe_nudge(&llm_response.text) {
                        messages.push(ChatMessage::assistant(&llm_response.text));
                        messages.push(ChatMessage::user(&nudge));
                        tracing::debug!(
                            nudge_count = agent_loop.nudge_count,
                            "Nudging LLM to complete task"
                        );
                        continue;
                    }
                }
                response_text = llm_response.text;
                agent_loop.complete();
                break;
            }

            // Check if this round has actual tools (not just discover_tools)
            let has_real_tool = tool_calls.iter().any(|c| c.name != "discover_tools");
            if !has_real_tool {
                if discovery_budget == 0 {
                    response_text = extract_text_content(&llm_response.text);
                    agent_loop.complete();
                    break;
                }
                discovery_budget -= 1;
            }

            let text_part = extract_text_content(&llm_response.text);

            // Add assistant message with proper format
            if self.use_native_tools && !llm_response.api_tool_calls.is_empty() {
                messages.push(ChatMessage::assistant_with_tool_calls(
                    &llm_response.text,
                    llm_response.api_tool_calls.clone(),
                ));
            } else {
                messages.push(ChatMessage::assistant(&llm_response.text));
            }

            let tc_pairs: Vec<(String, serde_json::Value)> = tool_calls
                .iter()
                .map(|c| (c.name.clone(), c.arguments.clone()))
                .collect();
            execute_tool_round_tracked(
                &self.registry, &mut self.guard, &self.db,
                &tc_pairs, &mut messages, &mut tool_calls_made,
                &mut injected_tool_names, max_perm, &self.task_manager,
                self.use_native_tools,
                &llm_response.api_tool_calls,
                &mut native_tools,
                &mut agent_loop,
                self.config.agent.error_recovery,
                self.incognito,
                self.cortex.as_mut(),
                agent_spawner.as_ref(),
                self.event_bus.as_ref(),
                msg_trace,
                &self.policy_engine,
                &self.trust_state,
                self.model_family,
            );

            if !text_part.is_empty() {
                response_text = text_part;
            }
        }

        // If we exhausted rounds without a clean response, request summary
        if (response_text.is_empty() || response_text.contains("[Using")) && !tool_calls_made.is_empty() {
            tracing::info!(
                tools = %tool_calls_made.join(", "),
                "Non-streaming tool loop exhausted — requesting summary"
            );
            agent_loop.status = crate::agent_loop::LoopStatus::MaxSteps;
            messages.push(ChatMessage::user(
                "Summarize what you accomplished in 1-2 sentences. Do NOT call any more tools."
            ));
            if let Ok(summary) = self.llm.chat(&messages, &gen_config, None) {
                let text = extract_text_content(&summary.text);
                if !text.is_empty() {
                    response_text = text;
                }
            }
        }
        if response_text.is_empty() {
            response_text = "I'm here. How can I help?".to_string();
        }

        // Record tool chain trace for learning (only if tools were actually called)
        if !self.incognito && self.config.agent.trace_learning && agent_loop.any_success() && !tool_calls_made.is_empty() {
            let outcome = match &agent_loop.status {
                crate::agent_loop::LoopStatus::Completed => "success",
                crate::agent_loop::LoopStatus::MaxSteps => "partial",
                crate::agent_loop::LoopStatus::Failed(_) => "failed",
                crate::agent_loop::LoopStatus::Running => "partial",
            };
            ToolTraces::record(
                self.db.conn(), &self.db, user_text,
                &agent_loop.chain_summary(), outcome,
            );
            // Trace learning flywheel: record for motif distillation
            crate::cognitive_router::record_trace(
                self.db.conn(), user_text, &tool_calls_made,
                outcome == "success",
                agent_loop.elapsed_ms(),
            );
        }

        // Clean up tool call XML and Qwen3.5 thinking blocks from final response
        response_text = extract_text_content(&response_text);
        response_text = strip_think_tags(&response_text);

        // Hallucination Firewall — verify factual claims against ground truth
        // Only active for Medium-tier models (where hallucination_firewall = true)
        if active_profile.hallucination_firewall && !is_offline {
            let mut ground_truth = GroundTruth::new();

            // Populate ground truth from active day context
            for (source_id, content, ts) in self.active_context.to_ground_truth() {
                ground_truth.add_tool_result(&source_id, &content, ts);
            }

            // Populate from tool results gathered during this conversation
            for tool_name in &tool_calls_made {
                // Tool results are already in the message history — extract from last tool messages
                for msg in messages.iter().rev().take(20) {
                    if msg.role == "tool" && msg.content.len() < 2000 {
                        ground_truth.add_tool_result(tool_name, &msg.content, now_ts() as u64);
                        break;
                    }
                }
            }

            if !ground_truth.is_empty() {
                let firewall = HallucinationFirewall::new(FirewallConfig {
                    enabled: true,
                    auto_correct: true,
                    ..Default::default()
                });
                let verdict = firewall.check(&response_text, &ground_truth);

                match verdict.action {
                    FirewallAction::PassThrough => {}
                    FirewallAction::Correct | FirewallAction::Annotate => {
                        if let Some(corrected) = verdict.corrected_response {
                            tracing::info!(
                                action = %verdict.action,
                                claims = verdict.claims.len(),
                                trust = format!("{:.2}", verdict.aggregate_trust),
                                "Hallucination firewall corrected response"
                            );
                            response_text = corrected;
                        }
                    }
                    FirewallAction::Block => {
                        tracing::warn!(
                            claims = verdict.claims.len(),
                            trust = format!("{:.2}", verdict.aggregate_trust),
                            "Hallucination firewall blocked response"
                        );
                        // Don't block entirely — downgrade to annotation with warning
                        if let Some(corrected) = verdict.corrected_response {
                            response_text = corrected;
                        }
                    }
                }
            }
        }

        // SecurityGuard: filter output for sensitive info leaks
        response_text = self.guard.check_response(&response_text, &self.db);

        // Update conversation history
        self.conversation_history
            .push(ChatMessage::user(user_text));
        self.conversation_history
            .push(ChatMessage::assistant(&response_text));

        // Compress conversation history when it grows too long
        self.compress_history_if_needed();

        self.last_interaction_ts = now_ts();
        self.session_turn_count += 1;

        // Steps 8-9: Skip all persistence in incognito mode
        if !self.incognito {
            // Step 8: Learn from this exchange (skip if offline — LLM needed)
            //   V25: Clean tool artifacts from response before learning
            if !is_offline {
                let clean_response = sanitize::clean_response_for_learning(
                    &response_text, &tool_calls_made,
                );
                learning::extract_and_learn(
                    &self.db, &*self.llm, user_text, &clean_response,
                    &self.config.memory_evolution,
                );
            }

            // Step 8b: Update conversation context for smart recall (Gap 1)
            memory_evolution::update_conversation_context(self.db.conn(), user_text, &memories);

            // Step 9: Score bond + tick evolution (always runs — tracks interaction count)
            if self.config.bond.enabled {
                let (new_level, level_changed) = BondTracker::score_interaction(
                    self.db.conn(),
                    user_text,
                    &response_text,
                    memories.len(),
                );
                self.bond_level = new_level;
                self.bond_level_changed = level_changed;

                let bond_state = BondTracker::get_state(self.db.conn());
                self.bond_score = bond_state.bond_score;

                // Tick personality evolution
                Evolution::tick(
                    self.db.conn(),
                    new_level,
                    self.config.evolution.formality_alpha,
                );

                // Check if narrative needs updating (skip if offline)
                if !is_offline {
                    let needs_narrative = Narrative::tick_interaction(
                        self.db.conn(),
                        self.config.narrative.update_interval_interactions,
                    );
                    if needs_narrative {
                        let self_texts: Vec<String> = self_memories
                            .iter()
                            .map(|m| m.text.clone())
                            .collect();
                        Narrative::update(
                            self.db.conn(),
                            &*self.llm,
                            &self.config.user_name,
                            new_level,
                            bond_state.bond_score,
                            &self_texts,
                            self.config.narrative.max_tokens,
                        );
                    }
                }
            }
        }

        // Step 10: CK-5 interaction recording — feed into schemas, narrative, replay
        if self.config.ck5.enabled && !self.incognito {
            let interaction = crate::ck5_integration::InteractionSummary {
                summary: if response_text.len() > 200 {
                    response_text[..response_text.floor_char_boundary(200)].to_string()
                } else {
                    response_text.clone()
                },
                tools_used: tool_calls_made.clone(),
                domains: classify_interaction_domains(user_text),
                sentiment: if self.bond_level_changed { 0.8 } else { 0.3 },
                outcome_positive: !is_offline,
                involved_nodes: Vec::new(),
            };

            crate::ck5_integration::run_ck5_cycle(
                &self.db,
                0.0, // not idle — active interaction
                &self.config.ck5,
                Some(&interaction),
            );
        }

        AgentResponse {
            message: response_text,
            memories_recalled: memories.len(),
            urges_delivered: urge_ids,
            tool_calls_made,
            offline_mode: is_offline,
        }
    }

    /// Streaming version of handle_message — calls `on_token` for each text fragment.
    pub fn handle_message_streaming<F>(
        &mut self,
        user_text: &str,
        mut on_token: F,
    ) -> AgentResponse
    where
        F: FnMut(&str),
    {
        // Step 0: SecurityGuard — check user input for injection
        if let Some(warning) = self.guard.check_input(user_text, &self.db) {
            on_token(&warning);
            return AgentResponse {
                message: warning,
                memories_recalled: 0,
                urges_delivered: vec![],
                tool_calls_made: vec![],
                offline_mode: false,
            };
        }

        // Steps 1-6 are identical to handle_message
        self.check_session_timeout();

        // Step 2: Smart multi-signal recall (Gap 1+2)
        let smart = if self.config.memory_evolution.smart_recall_enabled {
            memory_evolution::smart_recall(&self.db, user_text, &self.config.memory_evolution)
        } else {
            let mems = self.db.recall_text(user_text, 5).unwrap_or_default();
            memory_evolution::SmartRecallResult::from_primary(mems)
        };
        let mut memories = smart.all_unique();
        let (recall_confidence, recall_hint) = (smart.confidence, smart.hint);

        // Always include identity anchor memories
        {
            let existing_rids: std::collections::HashSet<String> =
                memories.iter().map(|m| m.rid.clone()).collect();
            let identity_facts = self.recall_identity_facts();
            for fact in identity_facts {
                if !existing_rids.contains(&fact.rid) {
                    memories.push(fact);
                }
            }
        }

        let self_memories = self
            .db
            .recall_text(&format!("self: {user_text}"), 10)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| {
                r.source == "self" || r.domain == "self-reflection"
            })
            .take(3)
            .collect::<Vec<_>>();

        let urges = self.urge_queue.pop_for_interaction(self.db.conn(), 2);
        let urge_ids: Vec<String> = urges.iter().map(|u| u.urge_id.clone()).collect();

        if !self.incognito {
            learning::detect_humor_reaction(self.db.conn(), user_text);
        }

        let state = self.build_state();
        for instinct in &self.instincts {
            let specs = instinct.on_interaction(&state, user_text);
            for spec in specs {
                self.urge_queue.push(self.db.conn(), &spec);
            }
        }

        // Build LLM context — lightweight for degraded mode, full otherwise
        let degraded = self.llm.is_degraded();
        if degraded {
            tracing::info!("LLM degraded (streaming) — lightweight prompt and minimal tools");
        }

        let context_messages = if degraded {
            context::build_messages_lightweight(
                user_text, &self.config, &memories, &self.conversation_history,
            )
        } else {
            let personality = self.db.get_personality().ok();
            let patterns_json: Vec<serde_json::Value> =
                self.active_patterns.iter().cloned().collect();
            let narrative_text = Narrative::get(self.db.conn());
            let style = Evolution::get_style(self.db.conn());
            let opinions = Evolution::get_opinions(self.db.conn(), 3);
            let shared_refs = if self.config.memory_evolution.reference_freshness_enabled {
                memory_evolution::get_fresh_references(self.db.conn(), 3)
            } else {
                Evolution::get_shared_references(self.db.conn(), 3)
            };
            // CK-5 cognitive awareness injection
            let ck5_text = if self.config.ck5.enabled {
                let snippet = crate::ck5_integration::build_context_snippet(&self.db);
                if snippet.is_empty() {
                    tracing::debug!("CK-5 context snippet: empty");
                    None
                } else {
                    let formatted = snippet.format_for_prompt(300);
                    tracing::debug!(ck5_prompt = %formatted, "CK-5 context injected into system prompt");
                    Some(formatted)
                }
            } else {
                None
            };
            let signals = ContextSignals {
                self_memories: &self_memories,
                narrative: &narrative_text,
                style: &style,
                opinions: &opinions,
                shared_refs: &shared_refs,
                system_state: &self.system_context,
                recall_confidence,
                recall_hint: recall_hint.as_deref(),
                ck5_awareness: ck5_text,
            };
            context::build_messages(
                user_text, &self.config, &state, &memories, &urges,
                &patterns_json, &self.conversation_history,
                personality.as_ref(), Some(&signals), self.use_native_tools,
            )
        };

        // Build message array — single system message (Qwen3.5 requires it):
        let max_perm = parse_permission(&self.config.tools.max_permission);
        let mut messages = Vec::with_capacity(context_messages.len() + 1);

        // Dynamic tool selection — adaptive based on model capability profile
        let active_profile = if degraded {
            ModelCapabilityProfile::degraded()
        } else {
            self.capability_profile.clone()
        };

        let word_count = user_text.split_whitespace().count();
        let needs_tools = self.config.tools.enabled && word_count > 2;
        tracing::debug!(
            tools_enabled = self.config.tools.enabled,
            word_count,
            needs_tools,
            degraded,
            tier = %active_profile.tier,
            "Tool selection gate (streaming)"
        );

        // ── Batched MCQ tool selection for Tiny/Small models (streaming) ──
        if !degraded && needs_tools && active_profile.uses_batched_mcq_selection() {
            let gen_config_mcq = GenerationConfig {
                max_tokens: self.config.llm.max_tokens.min(active_profile.max_generation_tokens),
                temperature: self.config.llm.temperature,
                max_context: Some(self.config.llm.max_context_tokens),
                ..Default::default()
            };

            let ranked = ToolCache::select_ranked_with_scores(
                self.db.conn(), &self.db, user_text, 15,
            );

            tracing::info!(
                ranked_count = ranked.len(),
                top_score = ranked.first().map(|r| r.0).unwrap_or(0.0),
                tier = %active_profile.tier,
                "MCQ batched tool selection (streaming) — ranking tools"
            );

            let selected_tool = if let Some(auto_name) = embedding_auto_select(&ranked) {
                Some(auto_name.to_string())
            } else {
                mcq_batch_select(&*self.llm, user_text, &ranked, &gen_config_mcq, 5, 3)
            };

            if let Some(tool_name) = selected_tool {
                let tool_defs = self.registry.definitions_for(&[tool_name.as_str()], max_perm);
                let tool_def = tool_defs.first().cloned().unwrap_or(serde_json::json!({}));
                let args = extract_tool_arguments(
                    &*self.llm, user_text, &tool_name, &tool_def, &gen_config_mcq,
                );

                let ctx = ToolContext {
                    db: &self.db,
                    max_permission: max_perm,
                    registry_metadata: None,
                    task_manager: Some(&self.task_manager),
                    incognito: self.incognito,
                    agent_spawner: None,
                };

                let tool_result = self.registry.execute(&ctx, &tool_name, &args);
                tracing::info!(tool = tool_name.as_str(), "MCQ tool executed (streaming)");

                // Generate final response with streaming
                let mut final_messages = context_messages.clone();
                final_messages.push(ChatMessage::assistant(&format!(
                    "I'll use the {} tool to help with that.", tool_name
                )));
                final_messages.push(ChatMessage::user(&format!(
                    "Tool {} returned:\n{}\n\nBased on this result, respond to the user's request: \"{}\"",
                    tool_name, tool_result, user_text
                )));

                let mut response_text = String::new();
                match self.llm.chat_streaming(&final_messages, &gen_config_mcq, None, &mut |token| {
                    on_token(token);
                }) {
                    Ok(r) => {
                        response_text = extract_text_content(&r.text);
                        if response_text.is_empty() { response_text = r.text; }
                    }
                    Err(_) => {
                        let fallback = format!("Here's what I found: {}", &tool_result[..tool_result.len().min(500)]);
                        on_token(&fallback);
                        response_text = fallback;
                    }
                }

                response_text = strip_think_tags(&response_text);
                response_text = self.guard.check_response(&response_text, &self.db);
                if response_text.is_empty() {
                    response_text = "I'm here. How can I help?".to_string();
                }

                self.conversation_history.push(ChatMessage::user(user_text));
                self.conversation_history.push(ChatMessage::assistant(&response_text));
                self.compress_history_if_needed();
                self.last_interaction_ts = now_ts();
                self.session_turn_count += 1;

                if !self.incognito {
                    let tool_calls_made = vec![tool_name.clone()];
                    let clean_response = sanitize::clean_response_for_learning(
                        &response_text, &tool_calls_made,
                    );
                    learning::extract_and_learn(
                        &self.db, &*self.llm, user_text, &clean_response,
                        &self.config.memory_evolution,
                    );
                    memory_evolution::update_conversation_context(self.db.conn(), user_text, &memories);
                    if self.config.bond.enabled {
                        let (new_level, level_changed) = BondTracker::score_interaction(
                            self.db.conn(), user_text, &response_text, memories.len(),
                        );
                        self.bond_level = new_level;
                        self.bond_level_changed = level_changed;
                    }
                    let trace_chain = vec![serde_json::json!({"tool": &tool_name, "status": "success"})];
                    ToolTraces::record(
                        self.db.conn(), &self.db, user_text,
                        &trace_chain, "success",
                    );
                }

                return AgentResponse {
                    message: response_text,
                    memories_recalled: memories.len(),
                    urges_delivered: urge_ids,
                    tool_calls_made: vec![tool_name],
                    offline_mode: false,
                };
            }
            // MCQ didn't select a tool — fall through to standard path
        }

        // ── Standard tool selection path (Medium/Large models) ────────────
        let selected_tool_names: Vec<&str> = if degraded {
            FALLBACK_TOOLS.to_vec()
        } else if needs_tools {
            select_tools_adaptive(user_text, &self.db, &active_profile)
        } else {
            ALWAYS_TOOLS.to_vec()
        };

        let mut native_tools: Vec<serde_json::Value> = if self.use_native_tools {
            let mut defs = self.registry.definitions_for(&selected_tool_names, max_perm);
            if !self.skill_extra_tools.is_empty() {
                let extra_refs: Vec<&str> = self.skill_extra_tools.iter()
                    .filter(|s| !selected_tool_names.contains(&s.as_str()))
                    .map(|s| s.as_str())
                    .collect();
                defs.extend(self.registry.definitions_for(&extra_refs, max_perm));
            }
            defs
        } else {
            Vec::new()
        };

        if !self.use_native_tools && self.config.tools.enabled {
            let selected_defs = self.registry.definitions_for(&selected_tool_names, max_perm);
            let tools_text = self.template().format_tools(&selected_defs);
            if !tools_text.is_empty() {
                if let Some(first) = context_messages.first() {
                    let combined = format!("{}\n\n{}", tools_text, first.content);
                    messages.push(ChatMessage::system(&combined));
                    messages.extend_from_slice(&context_messages[1..]);
                } else {
                    messages.push(ChatMessage::system(&tools_text));
                }
            } else {
                messages.extend(context_messages.clone());
            }
        }

        if messages.is_empty() {
            messages.extend(context_messages);
        }

        // Tool chain learning: inject trace hints (skip in degraded mode)
        if !degraded && self.config.agent.trace_learning && self.config.tools.enabled {
            let hints = ToolTraces::find_similar(
                self.db.conn(), &self.db, user_text, 3,
                self.config.agent.trace_min_similarity,
            );
            if !hints.is_empty() {
                let hint_text = ToolTraces::format_hints(&hints);
                if let Some(sys_msg) = messages.first_mut() {
                    sys_msg.content.push_str(&hint_text);
                }
                for hint in &hints {
                    ToolTraces::mark_used(self.db.conn(), &hint.trace_id);
                }
            }
        }

        // Inject Active Day Context into system prompt (budget from capability profile)
        if active_profile.ambient_context_budget > 0 {
            self.active_context.prune_stale();
            if let Some(context_block) = self.active_context.build_context_block(
                active_profile.ambient_context_budget,
            ) {
                if let Some(sys_msg) = messages.first_mut() {
                    sys_msg.content.push_str("\n\n");
                    sys_msg.content.push_str(&context_block);
                }
            }
        }

        // Step 7: Call LLM with streaming + robust agent loop
        // Generation config adapts to model capability profile
        let gen_config = if degraded {
            active_profile.tool_gen_config()
        } else {
            GenerationConfig {
                max_tokens: self.config.llm.max_tokens.min(active_profile.max_generation_tokens),
                temperature: self.config.llm.temperature,
                top_p: Some(0.9),
                max_context: Some(self.config.llm.max_context_tokens),
                ..Default::default()
            }
        };

        let mut tool_calls_made = Vec::new();
        let mut injected_tool_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut response_text: String;
        let mut is_offline = false;
        let max_nudges = if degraded { 0 } else { self.config.agent.max_nudges };
        let mut agent_loop = AgentLoop::new(user_text, max_nudges);

        // Build AgentSpawnerContext for parallel sub-agent tool
        let agent_spawner = Some(tools::AgentSpawnerContext {
            llm: self.llm.clone(),
            db_path: self.config.yantrikdb.db_path.clone(),
            embedding_dim: self.config.yantrikdb.embedding_dim,
            max_steps: 10,
            max_tokens: self.config.llm.max_tokens,
            temperature: self.config.llm.temperature,
            user_name: self.config.user_name.clone(),
            config: self.config.clone(),
        });

        // Emit UserMessage event and capture trace for tool call linking
        let msg_trace = self.event_bus.as_ref().map(|bus| {
            bus.emit(
                yantrik_os::EventKind::UserMessage {
                    text: user_text.chars().take(500).collect(),
                    source: "handle_message_streaming".into(),
                },
                yantrik_os::EventSource::UserInterface,
            )
        });

        // Round 1: streaming
        let mut streamed_text = String::new();
        // Compute tools_param in a temporary scope so it drops before any mutable borrow of native_tools
        let llm_response = {
            let tools_param: Option<&[serde_json::Value]> = if self.use_native_tools && !native_tools.is_empty() {
                Some(&native_tools)
            } else {
                None
            };
            self.llm.chat_streaming(&messages, &gen_config, tools_param, &mut |token| {
                streamed_text.push_str(token);
                on_token(token);
            })
        };

        match llm_response {
            Ok(r) => {
                let full_text = if !streamed_text.is_empty() { &streamed_text } else { &r.text };

                // Use native tool_calls if available, fall back to text parsing.
                // For StructuredJSON mode: parse via StructuredDecisionParser first.
                let tool_calls: Vec<ToolCall> = if !r.tool_calls.is_empty() {
                    r.tool_calls.clone()
                } else if active_profile.tool_call_mode == ToolCallMode::StructuredJSON {
                    match StructuredDecisionParser::parse(full_text) {
                        Ok(decision) => {
                            let known_tools: Vec<&str> = selected_tool_names.iter().copied().collect();
                            let validation = StructuredDecisionValidator::validate(
                                &decision, Some(&known_tools), active_profile.confidence_threshold,
                            );
                            match validation {
                                ValidationResult::Valid | ValidationResult::Repairable(_) => {
                                    if let Some(tc) = decision.to_tool_call() {
                                        vec![tc]
                                    } else if decision.has_answer() {
                                        response_text = decision.answer.clone();
                                        Vec::new()
                                    } else {
                                        Vec::new()
                                    }
                                }
                                ValidationResult::Invalid(_issues) => {
                                    // In streaming path, repair loop defers to the non-streaming
                                    // follow-up rounds. Fall back to standard text parsing.
                                    parse_tool_calls(full_text)
                                }
                            }
                        }
                        Err(_) => parse_tool_calls(full_text),
                    }
                } else {
                    parse_tool_calls(full_text)
                };

                if !tool_calls.is_empty() {
                    tracing::info!(
                        count = tool_calls.len(),
                        names = %tool_calls.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", "),
                        native = !r.api_tool_calls.is_empty(),
                        "Tool calls detected in streaming response"
                    );
                }
                if tool_calls.is_empty() {
                    response_text = if !streamed_text.is_empty() {
                        streamed_text.clone()
                    } else {
                        r.text
                    };
                } else {
                    // Tool calls found — enter multi-round tool loop (non-streaming)
                    let last_text_part = extract_text_content(full_text);

                    let display_text = if last_text_part.len() > 120 {
                        String::new()
                    } else {
                        last_text_part.trim().to_string()
                    };

                    on_token("__REPLACE__");
                    let tool_names: Vec<&str> = tool_calls.iter().map(|c| c.name.as_str()).collect();
                    let progress_msg = format_tool_progress(&tool_names, 1);
                    if display_text.is_empty() {
                        on_token(&format!("{}\n", progress_msg));
                    } else {
                        on_token(&format!("{}\n{}\n", display_text, progress_msg));
                    }

                    // Add assistant message with proper format
                    if self.use_native_tools && !r.api_tool_calls.is_empty() {
                        messages.push(ChatMessage::assistant_with_tool_calls(
                            full_text, r.api_tool_calls.clone(),
                        ));
                    } else {
                        messages.push(ChatMessage::assistant(full_text));
                    }

                    // Execute first round of tool calls
                    let tc_pairs: Vec<(String, serde_json::Value)> = tool_calls
                        .iter()
                        .map(|c| (c.name.clone(), c.arguments.clone()))
                        .collect();
                    execute_tool_round_tracked(
                        &self.registry, &mut self.guard, &self.db,
                        &tc_pairs, &mut messages, &mut tool_calls_made,
                        &mut injected_tool_names, max_perm, &self.task_manager,
                        self.use_native_tools,
                        &r.api_tool_calls,
                        &mut native_tools,
                        &mut agent_loop,
                        self.config.agent.error_recovery,
                        self.incognito,
                        self.cortex.as_mut(),
                        agent_spawner.as_ref(),
                        self.event_bus.as_ref(),
                        msg_trace,
                        &self.policy_engine,
                        &self.trust_state,
                        self.model_family,
                    );

                    // Remaining rounds: discovery rounds are budget-limited,
                    // actual tool rounds run until the hard cap.
                    response_text = display_text.clone();
                    let mut discovery_budget = if degraded { 0 } else { self.config.tools.max_tool_rounds.saturating_sub(1) };
                    let max_total_rounds = if degraded { 2 } else {
                        self.config.agent.max_steps.min(active_profile.max_agent_steps).max(3)
                    };

                    for _round in 0..max_total_rounds {
                        let tools_param: Option<&[serde_json::Value]> = if self.use_native_tools && !native_tools.is_empty() {
                            Some(&native_tools)
                        } else {
                            None
                        };
                        match self.llm.chat(&messages, &gen_config, tools_param) {
                            Ok(r2) => {
                                let tc2: Vec<ToolCall> = if !r2.tool_calls.is_empty() {
                                    r2.tool_calls.clone()
                                } else {
                                    parse_tool_calls(&r2.text)
                                };
                                if tc2.is_empty() {
                                    on_token("__REPLACE__");
                                    on_token(&r2.text);
                                    response_text = r2.text;
                                    agent_loop.complete();
                                    break;
                                }

                                let has_real_tool = tc2.iter().any(|c| c.name != "discover_tools");
                                if !has_real_tool {
                                    if discovery_budget == 0 {
                                        let fallback = extract_text_content(&r2.text);
                                        if !fallback.is_empty() {
                                            on_token("__REPLACE__");
                                            on_token(&fallback);
                                            response_text = fallback;
                                        }
                                        agent_loop.complete();
                                        break;
                                    }
                                    discovery_budget -= 1;
                                }

                                let round_text = extract_text_content(&r2.text);
                                let names2: Vec<&str> = tc2.iter().map(|c| c.name.as_str()).collect();
                                tracing::info!(
                                    count = tc2.len(),
                                    names = %names2.join(", "),
                                    "Tool calls detected in follow-up round"
                                );
                                let round_display = if round_text.len() > 120 {
                                    String::new()
                                } else {
                                    round_text.trim().to_string()
                                };
                                let step_num = tool_calls_made.len() + 1;
                                let progress_msg2 = format_tool_progress(&names2, step_num);
                                on_token("__REPLACE__");
                                if round_display.is_empty() {
                                    on_token(&format!("{}\n", progress_msg2));
                                } else {
                                    on_token(&format!("{}\n{}\n", round_display, progress_msg2));
                                }

                                if self.use_native_tools && !r2.api_tool_calls.is_empty() {
                                    messages.push(ChatMessage::assistant_with_tool_calls(
                                        &r2.text, r2.api_tool_calls.clone(),
                                    ));
                                } else {
                                    messages.push(ChatMessage::assistant(&r2.text));
                                }

                                let tc2_pairs: Vec<(String, serde_json::Value)> = tc2
                                    .iter()
                                    .map(|c| (c.name.clone(), c.arguments.clone()))
                                    .collect();

                                // Show progress before tool execution
                                on_token("__REPLACE__");
                                let run_progress = format_tool_progress(&names2, tool_calls_made.len());
                                on_token(&format!("{}\n", run_progress));

                                execute_tool_round_tracked(
                                    &self.registry, &mut self.guard, &self.db,
                                    &tc2_pairs, &mut messages, &mut tool_calls_made,
                                    &mut injected_tool_names, max_perm, &self.task_manager,
                                    self.use_native_tools,
                                    &r2.api_tool_calls,
                                    &mut native_tools,
                                    &mut agent_loop,
                                    self.config.agent.error_recovery,
                                    self.incognito,
                                    self.cortex.as_mut(),
                                    agent_spawner.as_ref(),
                                    self.event_bus.as_ref(),
                                    msg_trace,
                                    &self.policy_engine,
                                    &self.trust_state,
                                    self.model_family,
                                );

                                if !round_text.is_empty() {
                                    response_text = round_text;
                                }
                            }
                            Err(_) if !response_text.is_empty() => break,
                            Err(e) => {
                                tracing::warn!("LLM offline during tool follow-up: {e:#}");
                                response_text = OfflineResponder::respond(
                                    &self.db, user_text, &self.system_context,
                                    &memories, &urges, &self.config.user_name,
                                );
                                on_token("__REPLACE__");
                                on_token(&response_text);
                                is_offline = true;
                                break;
                            }
                        }
                    }

                    // If loop exhausted without a clean text response,
                    // make one final LLM call asking for a summary.
                    if response_text.is_empty() || response_text.contains("[Using") {
                        agent_loop.status = crate::agent_loop::LoopStatus::MaxSteps;
                        tracing::info!(
                            rounds = max_total_rounds,
                            tools = %tool_calls_made.join(", "),
                            "Tool loop exhausted — requesting summary"
                        );
                        messages.push(ChatMessage::user(
                            "Summarize what you accomplished in 1-2 sentences. Do NOT call any more tools."
                        ));
                        match self.llm.chat(&messages, &gen_config, None) {
                            Ok(summary) => {
                                let text = extract_text_content(&summary.text);
                                if !text.is_empty() {
                                    on_token("__REPLACE__");
                                    on_token(&text);
                                    response_text = text;
                                }
                            }
                            Err(_) => {
                                let fallback = format!(
                                    "I used {} tools to work on that. The task may still be in progress.",
                                    tool_calls_made.len()
                                );
                                on_token("__REPLACE__");
                                on_token(&fallback);
                                response_text = fallback;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("LLM offline: {e:#}");
                response_text = OfflineResponder::respond(
                    &self.db,
                    user_text,
                    &self.system_context,
                    &memories,
                    &urges,
                    &self.config.user_name,
                );
                on_token(&response_text);
                is_offline = true;
            }
        }

        if response_text.is_empty() {
            response_text = "I'm here. How can I help?".to_string();
        }

        // Record tool chain trace for learning (streaming path)
        if !self.incognito && self.config.agent.trace_learning && agent_loop.any_success() && !tool_calls_made.is_empty() {
            let outcome = match &agent_loop.status {
                crate::agent_loop::LoopStatus::Completed => "success",
                crate::agent_loop::LoopStatus::MaxSteps => "partial",
                crate::agent_loop::LoopStatus::Failed(_) => "failed",
                crate::agent_loop::LoopStatus::Running => "partial",
            };
            ToolTraces::record(
                self.db.conn(), &self.db, user_text,
                &agent_loop.chain_summary(), outcome,
            );
            // Trace learning flywheel: record for motif distillation
            crate::cognitive_router::record_trace(
                self.db.conn(), user_text, &tool_calls_made,
                outcome == "success",
                agent_loop.elapsed_ms(),
            );
        }

        response_text = extract_text_content(&response_text);
        response_text = strip_think_tags(&response_text);

        // Hallucination Firewall — verify factual claims against ground truth (streaming path)
        if active_profile.hallucination_firewall && !is_offline {
            let mut ground_truth = GroundTruth::new();
            for (source_id, content, ts) in self.active_context.to_ground_truth() {
                ground_truth.add_tool_result(&source_id, &content, ts);
            }
            for tool_name in &tool_calls_made {
                for msg in messages.iter().rev().take(20) {
                    if msg.role == "tool" && msg.content.len() < 2000 {
                        ground_truth.add_tool_result(tool_name, &msg.content, now_ts() as u64);
                        break;
                    }
                }
            }
            if !ground_truth.is_empty() {
                let firewall = HallucinationFirewall::new(FirewallConfig {
                    enabled: true,
                    auto_correct: true,
                    ..Default::default()
                });
                let verdict = firewall.check(&response_text, &ground_truth);
                match verdict.action {
                    FirewallAction::PassThrough => {}
                    FirewallAction::Correct | FirewallAction::Annotate | FirewallAction::Block => {
                        if let Some(corrected) = verdict.corrected_response {
                            tracing::info!(
                                action = %verdict.action,
                                trust = format!("{:.2}", verdict.aggregate_trust),
                                "Hallucination firewall corrected streaming response"
                            );
                            response_text = corrected;
                        }
                    }
                }
            }
        }

        // SecurityGuard: filter output for sensitive info leaks
        response_text = self.guard.check_response(&response_text, &self.db);

        // Update conversation history
        self.conversation_history.push(ChatMessage::user(user_text));
        self.conversation_history
            .push(ChatMessage::assistant(&response_text));

        // Compress conversation history when it grows too long
        self.compress_history_if_needed();

        self.last_interaction_ts = now_ts();
        self.session_turn_count += 1;

        // Steps 8-9: Skip all persistence in incognito mode
        if !self.incognito {
            // Step 8: Learn from this exchange (skip if offline — LLM needed)
            //   V25: Clean tool artifacts from response before learning
            if !is_offline {
                let clean_response = sanitize::clean_response_for_learning(
                    &response_text, &tool_calls_made,
                );
                learning::extract_and_learn(
                    &self.db, &*self.llm, user_text, &clean_response,
                    &self.config.memory_evolution,
                );
            }

            // Step 8b: Update conversation context for smart recall (Gap 1)
            memory_evolution::update_conversation_context(self.db.conn(), user_text, &memories);

            // Step 9: Score bond + tick evolution (always runs — tracks interaction count)
            if self.config.bond.enabled {
                let (new_level, level_changed) = BondTracker::score_interaction(
                    self.db.conn(),
                    user_text,
                    &response_text,
                    memories.len(),
                );
                self.bond_level = new_level;
                self.bond_level_changed = level_changed;

                let bond_state = BondTracker::get_state(self.db.conn());
                self.bond_score = bond_state.bond_score;

                Evolution::tick(
                    self.db.conn(),
                    new_level,
                    self.config.evolution.formality_alpha,
                );

                // Check if narrative needs updating (skip if offline)
                if !is_offline {
                    let needs_narrative = Narrative::tick_interaction(
                        self.db.conn(),
                        self.config.narrative.update_interval_interactions,
                    );
                    if needs_narrative {
                        let self_texts: Vec<String> = self_memories
                            .iter()
                            .map(|m| m.text.clone())
                            .collect();
                        Narrative::update(
                            self.db.conn(),
                            &*self.llm,
                            &self.config.user_name,
                            new_level,
                            bond_state.bond_score,
                            &self_texts,
                            self.config.narrative.max_tokens,
                        );
                    }
                }
            }
        }

        // Step 10: CK-5 interaction recording — feed into schemas, narrative, replay
        if self.config.ck5.enabled && !self.incognito {
            let interaction = crate::ck5_integration::InteractionSummary {
                summary: if response_text.len() > 200 {
                    response_text[..response_text.floor_char_boundary(200)].to_string()
                } else {
                    response_text.clone()
                },
                tools_used: tool_calls_made.clone(),
                domains: classify_interaction_domains(user_text),
                sentiment: if self.bond_level_changed { 0.8 } else { 0.3 },
                outcome_positive: !is_offline,
                involved_nodes: Vec::new(),
            };

            crate::ck5_integration::run_ck5_cycle(
                &self.db,
                0.0,
                &self.config.ck5,
                Some(&interaction),
            );
        }

        AgentResponse {
            message: response_text,
            memories_recalled: memories.len(),
            urges_delivered: urge_ids,
            tool_calls_made,
            offline_mode: is_offline,
        }
    }

    /// Build a state snapshot for instinct evaluation.
    pub fn build_state(&self) -> CompanionState {
        let memory_count = self
            .db
            .stats(None)
            .map(|s| s.active_memories)
            .unwrap_or(0);

        // Time fields
        let now = now_ts();
        let dt = chrono_from_ts(now);
        let current_hour = dt.0;
        let current_day_of_week = dt.1;
        let idle_seconds = now - self.last_interaction_ts;

        // Interaction density (last hour)
        let interactions_last_hour = count_recent_interactions(self.db.conn(), now - 3600.0);

        // Workflow hints for current hour
        let workflow_hints = query_workflow_hints(self.db.conn());

        // Maintenance report
        let maintenance_report = query_maintenance_log(self.db.conn());

        CompanionState {
            last_interaction_ts: self.last_interaction_ts,
            current_ts: now,
            session_active: self.session_turn_count > 0,
            conversation_turn_count: self.session_turn_count,
            recent_valence_avg: self.recent_valence_avg,
            pending_triggers: self.pending_triggers.clone(),
            active_patterns: self.active_patterns.clone(),
            open_conflicts_count: self.open_conflicts_count,
            memory_count,
            config_user_name: self.config.user_name.clone(),
            // Soul state
            bond_level: self.bond_level,
            bond_score: self.bond_score,
            formality: Evolution::get_style(self.db.conn()).formality,
            opinions_count: Evolution::count_opinions(self.db.conn()),
            shared_references_count: Evolution::count_shared_references(self.db.conn()),
            bond_level_changed: self.bond_level_changed,
            // Phase 2: Proactive intelligence
            current_hour,
            current_day_of_week,
            idle_seconds,
            interactions_last_hour,
            workflow_hints,
            maintenance_report,
            // Natural Communication
            recent_events: self.natural_events.clone(),
            avg_user_msg_length: if self.user_msg_lengths.is_empty() {
                0.0
            } else {
                self.user_msg_lengths.iter().sum::<usize>() as f64
                    / self.user_msg_lengths.len() as f64
            },
            daily_proactive_count: self.daily_proactive_count,
            recent_sent_messages: self.recent_sent_messages.clone(),
            suppressed_urges: self.suppressed_urges.clone(),
            // Interest intelligence
            user_interests: self.user_interests.clone(),
            user_location: self.user_location.clone(),
            // Open Loops Guardian
            open_loops_count: crate::world_model::count_open_threads(self.db.conn()),
            overdue_commitment_count: crate::world_model::WorldModel::overdue_commitments(self.db.conn()).len(),
            pending_attention_count: crate::world_model::attention_summary(self.db.conn())
                .iter().map(|(_, c)| c).sum(),
            model_tier: self.capability_profile.tier,
        }
    }

    /// Evaluate all instincts against current state. Used by background cognition.
    pub fn evaluate_instincts(&self, state: &CompanionState) -> Vec<crate::types::UrgeSpec> {
        let mut all_urges = Vec::new();
        for instinct in &self.instincts {
            all_urges.extend(instinct.evaluate(state));
        }
        all_urges
    }

    /// Check proactive engine for messages to deliver. Called during think cycle.
    pub fn check_proactive(&mut self) {
        // V25: Silence policy — skip if in quiet period (frustration cooldown)
        if self.silence_policy.is_quiet_period() {
            tracing::debug!("Silence policy: quiet period active, skipping proactive check");
            return;
        }

        // V25: Taste trust gate — don't send proactive if suggestions aren't welcome
        if !self.trust_state.suggestions_welcome() {
            tracing::debug!(taste = self.trust_state.taste, "Taste trust too low, skipping proactive");
            return;
        }

        // Sync bond level so templates render with personality
        self.proactive_engine.set_bond_level(self.bond_level);
        if let Some(msg) = self.proactive_engine.check(&self.urge_queue, self.db.conn()) {
            self.set_proactive_message(msg);
        }
    }

    /// Record a proactive message outcome for the silence policy and brain feedback.
    pub fn record_proactive_outcome(&mut self, source: &str, outcome: crate::silence_policy::InterventionOutcome) {
        let is_positive = outcome.is_positive();
        let is_negative = outcome.is_negative();
        self.silence_policy.record_outcome(source, outcome, self.db.conn());

        // Brain feedback: map outcome to reward for source learning
        let reward = if is_positive { 1.0 } else if is_negative { 0.0 } else { 0.5 };
        // Infer signal type from the source instinct name
        let signal_type_str = yantrikdb_core::cognition::brain::infer_signal_type(
            source, "", &serde_json::json!({}),
        ).as_str();
        crate::brain_loop::record_brain_feedback(self.db.conn(), source, signal_type_str, reward);

        // Also update taste trust
        let trust_event = if is_positive {
            crate::trust_model::TrustEvent::SuggestionAccepted { source: source.to_string() }
        } else if is_negative {
            crate::trust_model::TrustEvent::SuggestionDismissed { source: source.to_string() }
        } else {
            return;
        };
        self.trust_state = crate::trust_model::TrustModel::apply_event(self.db.conn(), &trust_event);
    }

    /// Refresh trust state from DB (call periodically).
    pub fn refresh_trust_state(&mut self) {
        self.trust_state = crate::trust_model::TrustModel::get_state(self.db.conn());
    }

    /// Get silence policy dampening factor for a source.
    pub fn silence_dampening_for(&self, source: &str) -> f64 {
        self.silence_policy.dampening_for(source)
    }

    /// Take the pending proactive message (if any).
    pub fn take_proactive_message(&mut self) -> Option<ProactiveMessage> {
        self.proactive_message.take()
    }

    /// Set a proactive message (called by background cognition).
    pub fn set_proactive_message(&mut self, msg: ProactiveMessage) {
        self.proactive_message = Some(msg);
    }

    // ---- Natural Communication helpers ----

    /// Record a significant event for the aftermath instinct.
    pub fn record_event(&mut self, description: &str) {
        let ts = now_ts();
        self.natural_events.push((description.to_string(), ts, false));
        // Keep last 20 events
        if self.natural_events.len() > 20 {
            self.natural_events.drain(0..self.natural_events.len() - 20);
        }
    }

    /// Mark an event as reflected (aftermath instinct used it).
    pub fn mark_event_reflected(&mut self, idx: usize) {
        if let Some(ev) = self.natural_events.get_mut(idx) {
            ev.2 = true;
        }
    }

    /// Track user message length for conversational metabolism.
    pub fn track_user_msg_length(&mut self, len: usize) {
        self.user_msg_lengths.push(len);
        if self.user_msg_lengths.len() > 5 {
            self.user_msg_lengths.drain(0..self.user_msg_lengths.len() - 5);
        }
    }

    /// Record a sent proactive message for anti-repetition AND conversation history.
    pub fn record_sent_message(&mut self, text: &str) {
        self.recent_sent_messages.push(text.to_string());
        if self.recent_sent_messages.len() > 10 {
            self.recent_sent_messages.drain(0..self.recent_sent_messages.len() - 10);
        }

        // Add to conversation history so the LLM remembers what it said
        self.conversation_history.push(ChatMessage::assistant(text));
        // Compress conversation history if needed
        self.compress_history_if_needed();

        // Track daily count
        let now = now_ts();
        if now - self.daily_proactive_reset_ts > 86400.0 {
            self.daily_proactive_count = 0;
            self.daily_proactive_reset_ts = now;
        }
        self.daily_proactive_count += 1;
    }

    /// Record a suppressed urge for strategic silence.
    pub fn record_suppressed_urge(&mut self, key: &str, reason: &str) {
        let ts = now_ts();
        self.suppressed_urges.push((key.to_string(), reason.to_string(), ts));
        // Keep last 20
        if self.suppressed_urges.len() > 20 {
            self.suppressed_urges.drain(0..self.suppressed_urges.len() - 20);
        }
    }

    /// Record proactive message context for threading.
    pub fn record_proactive_context(&mut self, text: &str, urge_ids: Vec<String>) {
        self.last_proactive_context = Some((text.to_string(), urge_ids, now_ts()));
    }

    /// Get threading context if a user replies shortly after a proactive message.
    /// Returns Some(context_text) if the last proactive was within 15 minutes.
    pub fn get_threading_context(&mut self) -> Option<String> {
        if let Some((ref text, ref _urge_ids, ts)) = self.last_proactive_context {
            let elapsed = now_ts() - ts;
            if elapsed < 900.0 { // 15 minutes
                let short = if text.len() > 500 {
                    format!("{}...", &text[..text.floor_char_boundary(497)])
                } else {
                    text.clone()
                };
                let ctx = format!(
                    "[Context: You recently sent this proactive message ({:.0}s ago):\n\"{}\"\nThe user's reply may be about this.]\n",
                    elapsed, short
                );
                // Don't clear — the message is also in conversation_history now,
                // but keep this as reinforcement for multiple follow-ups
                return Some(ctx);
            }
        }
        None
    }

    /// Get the last N sent messages for anti-repetition prompting.
    pub fn last_sent_messages(&self, n: usize) -> &[String] {
        let start = self.recent_sent_messages.len().saturating_sub(n);
        &self.recent_sent_messages[start..]
    }

    /// Get daily message budget based on bond level.
    pub fn daily_message_budget(&self) -> u32 {
        let base = match self.bond_level {
            BondLevel::Stranger => 3,
            BondLevel::Acquaintance => 5,
            BondLevel::Friend => 8,
            BondLevel::Confidant => 10,
            BondLevel::PartnerInCrime => 14,
        };
        // Apply adaptive user model budget multiplier
        let mult = self.user_model.budget_multiplier();
        let adjusted = (base as f64 * mult).round() as u32;
        adjusted.clamp(2, 20) // MIN_BUDGET=2, MAX_BUDGET=20
    }

    /// Check if daily proactive budget is exceeded.
    pub fn is_over_daily_budget(&self) -> bool {
        self.daily_proactive_count >= self.daily_message_budget()
    }

    /// Update cached cognition state (called after think()).
    pub fn update_cognition_cache(
        &mut self,
        triggers: Vec<serde_json::Value>,
        patterns: Vec<serde_json::Value>,
        conflicts_count: usize,
        valence_avg: Option<f64>,
    ) {
        self.pending_triggers = triggers;
        self.active_patterns = patterns;
        self.open_conflicts_count = conflicts_count;
        self.recent_valence_avg = valence_avg;
    }

    /// Update the system context string (battery, network, etc.)
    /// that gets injected into the LLM system prompt.
    pub fn set_system_context(&mut self, ctx: String) {
        // Check for injection patterns in system context (comes from D-Bus/sysinfo)
        sanitize::check_and_warn(&ctx, "system_context");
        self.system_context = ctx;
    }

    /// Poll background tasks; record completed ones to memory.
    /// Poll background tasks. Returns descriptions of newly completed tasks (for notifications).
    pub fn poll_background_tasks(&self) -> Vec<String> {
        let mut tm = match self.task_manager.lock() {
            Ok(t) => t,
            Err(_) => return Vec::new(),
        };
        let completed = tm.poll(self.db.conn());
        let mut notifications = Vec::new();
        for task_id in &completed {
            if let Some(status) = tm.get_status(self.db.conn(), task_id) {
                let output = crate::task_manager::TaskManager::read_output(task_id, 20);
                let exit_str = status.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "?".into());
                let text = format!(
                    "Background task completed: {} ({})\nExit: {}\nOutput:\n{}",
                    status.label, status.command, exit_str, output
                );
                let _ = self.db.record_text(
                    &text, "episodic", 0.5, 0.0, 604800.0,
                    &serde_json::json!({"task_id": task_id}),
                    "default", 0.9, "system/tasks", "system", None,
                );
                crate::task_manager::TaskManager::mark_recorded(self.db.conn(), task_id);

                // Build notification text
                let outcome = if status.exit_code == Some(0) { "completed" } else { "failed" };
                let short_output = if output.len() > 200 {
                    format!("{}...", &output[..output.floor_char_boundary(197)])
                } else {
                    output
                };
                notifications.push(format!(
                    "Background task {} ({}): exit {}\n{}",
                    outcome, status.label, exit_str, short_output
                ));
            }
        }
        notifications
    }

    /// Active task summary for system context injection.
    pub fn active_tasks_summary(&self) -> String {
        match self.task_manager.lock() {
            Ok(tm) => tm.format_active_summary(self.db.conn()),
            Err(_) => String::new(),
        }
    }

    /// Get conversation history.
    pub fn history(&self) -> &[ChatMessage] {
        &self.conversation_history
    }

    /// Compress conversation history when it exceeds the configured limit.
    ///
    /// Instead of simply dropping old messages, summarizes the older half
    /// into a single context message using the LLM. This preserves key
    /// information while freeing context window space.
    fn compress_history_if_needed(&mut self) {
        let max = self.config.conversation.max_history_turns * 2;
        if self.conversation_history.len() <= max {
            return;
        }

        // Split: older half gets compressed, recent half stays verbatim
        let split = self.conversation_history.len() / 2;
        // Round to even (user+assistant pairs)
        let split = split - (split % 2);

        if split < 4 {
            // Too few messages to compress — just truncate
            let drain = self.conversation_history.len() - max;
            self.conversation_history.drain(..drain);
            return;
        }

        let old_messages = &self.conversation_history[..split];

        // Build summary text from old messages (without LLM call for speed)
        let mut summary_parts = Vec::new();
        for pair in old_messages.chunks(2) {
            if pair.len() == 2 {
                let user_text = &pair[0].content;
                let asst_text = &pair[1].content;
                // Truncate each turn to keep summary compact
                let user_short = if user_text.len() > 150 {
                    format!("{}...", &user_text[..user_text.floor_char_boundary(150)])
                } else {
                    user_text.clone()
                };
                let asst_short = if asst_text.len() > 200 {
                    format!("{}...", &asst_text[..asst_text.floor_char_boundary(200)])
                } else {
                    asst_text.clone()
                };
                summary_parts.push(format!("User: {}\nAssistant: {}", user_short, asst_short));
            }
        }

        let summary = format!(
            "[Earlier conversation summary ({} turns compressed)]\n{}",
            split / 2,
            summary_parts.join("\n---\n")
        );

        // Replace old messages with a single summary message
        self.conversation_history.drain(..split);
        self.conversation_history.insert(0, ChatMessage::system(&summary));

        tracing::debug!(
            compressed = split / 2,
            remaining = self.conversation_history.len(),
            "Conversation history compressed"
        );
    }

    /// Seconds since last interaction.
    pub fn idle_seconds(&self) -> f64 {
        now_ts() - self.last_interaction_ts
    }

    /// Get current bond level.
    pub fn bond_level(&self) -> BondLevel {
        self.bond_level
    }

    /// Get current bond score.
    pub fn bond_score(&self) -> f64 {
        self.bond_score
    }

    /// Recall high-importance identity facts (name, website, GitHub, etc.)
    /// These are always included in context regardless of query topic.
    fn recall_identity_facts(&self) -> Vec<yantrikdb_core::types::RecallResult> {
        let conn = self.db.conn();
        // Fetch identity + high-importance work facts (name, website, GitHub, etc.)
        let query = "SELECT rid, text, importance, domain FROM memories \
                     WHERE ((domain = 'identity' AND importance >= 0.7) \
                        OR (domain = 'work' AND importance >= 0.85)) \
                     AND consolidation_status = 'active' \
                     ORDER BY importance DESC LIMIT 8";
        let mut stmt = match conn.prepare(query) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |row| {
            Ok(yantrikdb_core::types::RecallResult {
                rid: row.get(0)?,
                text: row.get(1)?,
                memory_type: "semantic".to_string(),
                created_at: 0.0,
                importance: row.get::<_, f64>(2)?,
                valence: 0.0,
                score: row.get::<_, f64>(2)?, // use importance as score
                scores: yantrikdb_core::types::ScoreBreakdown {
                    similarity: 1.0,
                    decay: 1.0,
                    recency: 1.0,
                    importance: row.get::<_, f64>(2).unwrap_or(0.8),
                    graph_proximity: 0.0,
                    contributions: yantrikdb_core::types::ScoreContributions {
                        similarity: 1.0, decay: 1.0, recency: 1.0, importance: 1.0, graph_proximity: 0.0,
                    },
                    valence_multiplier: 1.0,
                },
                why_retrieved: vec!["identity anchor".to_string()],
                metadata: serde_json::Value::Null,
                namespace: "default".to_string(),
                certainty: 0.9,
                domain: row.get(3)?,
                source: "companion".to_string(),
                emotional_state: None,
            })
        })
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    fn check_session_timeout(&mut self) {
        let idle = self.idle_seconds();
        let timeout = self.config.conversation.session_timeout_minutes as f64 * 60.0;

        if idle > timeout && self.session_turn_count > 0 {
            tracing::info!(
                idle_minutes = idle / 60.0,
                turns = self.session_turn_count,
                "Session timeout — resetting history"
            );
            self.conversation_history.clear();
            self.session_turn_count = 0;
        }
    }
}

/// Compute recall confidence from result scores.
/// Returns (confidence, hint) where hint is a prompt instruction for low confidence.
fn compute_recall_confidence(
    memories: &[yantrikdb_core::types::RecallResult],
) -> (f64, Option<String>) {
    if memories.is_empty() {
        return (0.0, Some("You have no relevant memories for this topic.".into()));
    }

    let n = memories.len() as f64;

    // Signal 1: Average similarity (0.0–1.0) — how well do results match the query?
    let avg_sim = memories.iter().map(|r| r.scores.similarity).sum::<f64>() / n;

    // Signal 2: Best similarity — is there at least one strong hit?
    let best_sim = memories
        .iter()
        .map(|r| r.scores.similarity)
        .fold(0.0_f64, f64::max);

    // Signal 3: Score gap — large gap between best and worst = uncertain spread
    let worst_score = memories
        .iter()
        .map(|r| r.score)
        .fold(f64::MAX, f64::min);
    let best_score = memories.iter().map(|r| r.score).fold(0.0_f64, f64::max);
    let gap_penalty = if best_score > 0.0 {
        ((best_score - worst_score) / best_score).min(1.0)
    } else {
        1.0
    };

    // Combined: weighted average
    let confidence = (0.40 * avg_sim + 0.35 * best_sim + 0.25 * (1.0 - gap_penalty)).clamp(0.0, 1.0);

    let hint = if confidence < 0.3 {
        Some(
            "Your memory match is very weak — ask clarifying questions \
             to understand what the user means."
                .into(),
        )
    } else if confidence < 0.5 {
        Some(
            "Your memory match is uncertain — mention what you do remember \
             and ask if that's what they mean."
                .into(),
        )
    } else {
        None
    };

    (confidence, hint)
}

/// Execute a round of tool calls with discover_tools schema injection.
///
/// When `use_native_tools` is true (API backend), tool results are sent as
/// `role: "tool"` messages with `tool_call_id`, matching OpenAI format.
/// When false (candle/llamacpp), results are sent as user messages with `<data:tool_result>` tags.
fn execute_tool_round(
    registry: &ToolRegistry,
    guard: &mut SecurityGuard,
    db: &YantrikDB,
    tool_calls: &[(String, serde_json::Value)],
    messages: &mut Vec<ChatMessage>,
    tool_calls_made: &mut Vec<String>,
    injected_tool_names: &mut std::collections::HashSet<String>,
    max_perm: PermissionLevel,
    task_manager: &std::sync::Mutex<crate::task_manager::TaskManager>,
    use_native_tools: bool,
    api_tool_calls: &[yantrik_ml::ApiToolCall],
    native_tools: &mut Vec<serde_json::Value>,
    incognito: bool,
    model_family: ModelFamily,
) {
    let ctx = ToolContext {
        db,
        max_permission: max_perm,
        registry_metadata: None,
        task_manager: Some(task_manager),
        incognito,
        agent_spawner: None,
    };

    for (idx, (name, args)) in tool_calls.iter().enumerate() {
        tool_calls_made.push(name.clone());

        // For discover_tools, build a context with metadata
        let result = if name == "discover_tools" {
            let metadata = registry.list_metadata(max_perm);
            let disc_ctx = ToolContext {
                db,
                max_permission: max_perm,
                registry_metadata: Some(&metadata),
                task_manager: Some(task_manager),
                incognito,
                agent_spawner: None,
            };
            registry.execute(&disc_ctx, name, args)
        } else {
            registry.execute(&ctx, name, args)
        };

        guard.check_tool_result(name, &result, db);

        // Dynamic schema injection for discover_tools
        if name == "discover_tools" {
            let discovered = parse_discovered_tool_names(&result, injected_tool_names);
            if discovered.is_empty() {
                let override_result = "All relevant tools are already available. Use them now.".to_string();
                if use_native_tools {
                    let call_id = api_tool_calls.get(idx)
                        .map(|tc| tc.id.as_str())
                        .unwrap_or("call_discover");
                    let tmpl = template_for_family(model_family);
                    messages.push(tmpl.format_tool_result(call_id, name, &override_result));
                } else {
                    messages.push(ChatMessage::assistant(&format!(
                        "<tool_call>\n{{\"name\": \"discover_tools\", \"arguments\": {}}}\n</tool_call>",
                        serde_json::to_string(args).unwrap_or_default()
                    )));
                    messages.push(ChatMessage::user(&format!("[tool result: {}] {}", name, override_result)));
                }
                continue;
            }
            {
                let refs: Vec<&str> = discovered.iter().map(|s| s.as_str()).collect();
                let new_defs = registry.definitions_for(&refs, max_perm);
                if !new_defs.is_empty() {
                    tracing::info!(
                        tools = %discovered.join(", "),
                        "Dynamic schema injection after discover_tools"
                    );
                    if use_native_tools {
                        // For API backend: add to native tools array
                        native_tools.extend(new_defs);
                    } else {
                        // For non-API: text-inject into system message
                        let tmpl = template_for_family(model_family);
                        let new_text = tmpl.format_tools(&new_defs);
                        if let Some(sys_msg) = messages.first_mut() {
                            sys_msg.content.push_str(&new_text);
                        }
                    }
                    for n in &discovered {
                        injected_tool_names.insert(n.clone());
                    }
                }
            }
        }

        let max_len = sanitize::max_result_len_for_tool(name);
        let safe_result = sanitize::sanitize_tool_result_with_limit(&result, max_len);

        if use_native_tools {
            // Family-aware tool result format (e.g. Nemotron uses role:user + <tool_response>)
            let call_id = api_tool_calls.get(idx)
                .map(|tc| tc.id.as_str())
                .unwrap_or_else(|| "call_0");
            let tmpl = template_for_family(model_family);
            messages.push(tmpl.format_tool_result(call_id, name, &safe_result));
        } else {
            // Legacy format: user message with data tag
            messages.push(ChatMessage::user(format!(
                "<data:tool_result name=\"{}\">{}",
                sanitize::escape_for_prompt(name),
                safe_result,
            )));
        }
    }
}

/// Execute a round of tool calls with agent loop tracking + error recovery.
///
/// Delegates to `execute_tool_round` for actual execution, then records each
/// step in the `AgentLoop` tracker. On tool failure, optionally injects an
/// error recovery hint suggesting alternatives.
fn execute_tool_round_tracked(
    registry: &ToolRegistry,
    guard: &mut SecurityGuard,
    db: &YantrikDB,
    tool_calls: &[(String, serde_json::Value)],
    messages: &mut Vec<ChatMessage>,
    tool_calls_made: &mut Vec<String>,
    injected_tool_names: &mut std::collections::HashSet<String>,
    max_perm: PermissionLevel,
    task_manager: &std::sync::Mutex<crate::task_manager::TaskManager>,
    use_native_tools: bool,
    api_tool_calls: &[yantrik_ml::ApiToolCall],
    native_tools: &mut Vec<serde_json::Value>,
    agent_loop: &mut AgentLoop,
    error_recovery: bool,
    incognito: bool,
    mut cortex: Option<&mut crate::cortex::ContextCortex>,
    agent_spawner: Option<&tools::AgentSpawnerContext>,
    event_bus: Option<&yantrik_os::EventBus>,
    parent_trace: Option<yantrik_os::TraceId>,
    policy_engine: &crate::policy_engine::PolicyEngine,
    trust_state: &crate::trust_model::TrustState,
    model_family: ModelFamily,
) {
    let agent_spawner_any = agent_spawner.map(|s| s as &dyn std::any::Any);
    let ctx = ToolContext {
        db,
        max_permission: max_perm,
        registry_metadata: None,
        task_manager: Some(task_manager),
        incognito,
        agent_spawner: agent_spawner_any,
    };

    for (idx, (name, args)) in tool_calls.iter().enumerate() {
        tool_calls_made.push(name.clone());

        // Runaway detection: category-aware limits (browser=25, files=10, shell=15, etc.)
        let max_for_tool = crate::agent_loop::max_calls_for_tool(name);
        if let Some(stop_msg) = agent_loop.runaway_check(name, max_for_tool) {
            tracing::warn!(tool = name, "Runaway tool loop detected — injecting stop");
            if use_native_tools {
                let call_id = api_tool_calls.get(idx)
                    .map(|tc| tc.id.as_str())
                    .unwrap_or("call_stop");
                let tmpl = template_for_family(model_family);
                messages.push(tmpl.format_tool_result(call_id, name, &stop_msg));
            } else {
                messages.push(ChatMessage::user(&format!("[system] {}", stop_msg)));
            }
            continue;
        }

        // Emit ToolCalled event
        let tool_trace = if let Some(bus) = event_bus {
            let trace = if let Some(parent) = parent_trace {
                bus.emit_with_parent(
                    yantrik_os::EventKind::ToolCalled {
                        tool_name: name.clone(),
                        arguments: args.clone(),
                        permission: format!("{:?}", max_perm),
                    },
                    yantrik_os::EventSource::ToolExecutor,
                    parent,
                )
            } else {
                bus.emit(
                    yantrik_os::EventKind::ToolCalled {
                        tool_name: name.clone(),
                        arguments: args.clone(),
                        permission: format!("{:?}", max_perm),
                    },
                    yantrik_os::EventSource::ToolExecutor,
                )
            };
            Some(trace)
        } else {
            None
        };

        // V25: Policy engine gate — check if this action is allowed
        let action_ctx = crate::policy_engine::ActionContext::from_tool(name);
        let policy_decision = policy_engine.evaluate(&action_ctx, trust_state);
        if let crate::policy_engine::PolicyDecision::Deny { ref reason } = policy_decision {
            tracing::warn!(tool = %name, reason = %reason, "Policy engine denied tool execution");
            let result = format!("BLOCKED by policy: {}", reason);
            agent_loop.record_step(name, args, &result, false);
            if use_native_tools {
                let call_id = api_tool_calls.get(idx).map(|tc| tc.id.as_str()).unwrap_or("call_0");
                let tmpl = template_for_family(model_family);
                messages.push(tmpl.format_tool_result(call_id, name, &result));
            } else {
                messages.push(ChatMessage::user(format!(
                    "<data:tool_result name=\"{}\">{}", name, result
                )));
            }
            continue;
        }
        if !policy_decision.is_allowed() {
            tracing::info!(tool = %name, decision = %policy_decision.as_str(), "Policy engine flagged tool");
        }

        let tool_start = std::time::Instant::now();

        // Execute the tool
        let result = if name == "discover_tools" {
            let metadata = registry.list_metadata(max_perm);
            let disc_ctx = ToolContext {
                db,
                max_permission: max_perm,
                registry_metadata: Some(&metadata),
                task_manager: Some(task_manager),
                incognito,
                agent_spawner: agent_spawner_any,
            };
            registry.execute(&disc_ctx, name, args)
        } else {
            registry.execute(&ctx, name, args)
        };

        let tool_duration_ms = tool_start.elapsed().as_millis() as u64;

        guard.check_tool_result(name, &result, db);

        // MCP security: check if this tool call was potentially manipulated by
        // a previous MCP server response (follow-up action blocking)
        let last_mcp = tool_calls_made.iter().rev().skip(1)
            .find(|t| t.starts_with("mcp__"))
            .and_then(|t| t.strip_prefix("mcp__"))
            .and_then(|t| t.split('_').next())
            .map(|s| s.to_string());
        if !name.starts_with("mcp__") {
            if let Some(ref mcp_server) = last_mcp {
                if let Ok(scanner) = tools::mcp::security_scanner().lock() {
                    if let Some(block_msg) = scanner.scan_llm_follow_up(name, args, Some(mcp_server)) {
                        tracing::warn!(
                            tool = name,
                            mcp_server = %mcp_server,
                            "MCP follow-up action blocked"
                        );
                        let result = format!("Security: {}", block_msg);
                        if use_native_tools {
                            let call_id = api_tool_calls.get(idx)
                                .map(|tc| tc.id.as_str())
                                .unwrap_or("call_security");
                            let tmpl = template_for_family(model_family);
                            messages.push(tmpl.format_tool_result(call_id, name, &result));
                        } else {
                            messages.push(ChatMessage::user(&format!("[tool result: {}] {}", name, result)));
                        }
                        continue;
                    }
                }
            }
        }

        // Determine success/failure for agent loop tracking
        let is_error = result.starts_with("Error:")
            || result.starts_with("error:")
            || result.starts_with("Permission denied")
            || result.starts_with("Tool not found")
            || result.starts_with("BLOCKED")
            || result.starts_with("Security:");

        // Emit ToolCompleted event with outcome (postcondition verification)
        if let (Some(bus), Some(trace)) = (event_bus, tool_trace) {
            let outcome = if is_error {
                yantrik_os::ToolOutcome::Failed {
                    error: result.chars().take(200).collect(),
                }
            } else {
                verify_postcondition(name, args, &result)
            };
            let preview: String = result.chars().take(200).collect();
            bus.emit_with_parent(
                yantrik_os::EventKind::ToolCompleted {
                    tool_name: name.clone(),
                    outcome,
                    duration_ms: tool_duration_ms,
                    result_preview: preview,
                },
                yantrik_os::EventSource::ToolExecutor,
                trace,
            );
        }

        // Record tool reliability metrics
        let failure_reason = if is_error {
            Some(result.chars().take(200).collect::<String>())
        } else {
            None
        };
        crate::tool_metrics::ToolMetrics::record(
            db.conn(), name, !is_error, tool_duration_ms,
            failure_reason.as_deref(),
        );

        // V25: Update trust model based on tool outcome
        {
            let trust_event = if is_error {
                crate::trust_model::TrustEvent::AutonomousMistake {
                    action: name.to_string(),
                    severity: 0.5,
                }
            } else {
                crate::trust_model::TrustEvent::AutonomousSuccess {
                    action: name.to_string(),
                }
            };
            crate::trust_model::TrustModel::apply_event(db.conn(), &trust_event);
        }

        // Record step in agent loop
        agent_loop.record_step(name, args, &result, !is_error);

        // Ingest into Context Cortex pulse stream
        if let Some(ctx) = cortex.as_mut() {
            ctx.ingest_tool_result(db.conn(), name, args, &result);
        }

        // Dynamic schema injection for discover_tools
        if name == "discover_tools" {
            let discovered = parse_discovered_tool_names(&result, injected_tool_names);
            if discovered.is_empty() {
                let override_result = "All relevant tools are already available. Use them now.".to_string();
                if use_native_tools {
                    let call_id = api_tool_calls.get(idx)
                        .map(|tc| tc.id.as_str())
                        .unwrap_or("call_discover");
                    let tmpl = template_for_family(model_family);
                    messages.push(tmpl.format_tool_result(call_id, name, &override_result));
                } else {
                    messages.push(ChatMessage::assistant(&format!(
                        "<tool_call>\n{{\"name\": \"discover_tools\", \"arguments\": {}}}\n</tool_call>",
                        serde_json::to_string(args).unwrap_or_default()
                    )));
                    messages.push(ChatMessage::user(&format!("[tool result: {}] {}", name, override_result)));
                }
                continue;
            }
            {
                let refs: Vec<&str> = discovered.iter().map(|s| s.as_str()).collect();
                let new_defs = registry.definitions_for(&refs, max_perm);
                if !new_defs.is_empty() {
                    tracing::info!(
                        tools = %discovered.join(", "),
                        "Dynamic schema injection after discover_tools"
                    );
                    if use_native_tools {
                        native_tools.extend(new_defs);
                    } else {
                        let tmpl = template_for_family(model_family);
                        let new_text = tmpl.format_tools(&new_defs);
                        if let Some(sys_msg) = messages.first_mut() {
                            sys_msg.content.push_str(&new_text);
                        }
                    }
                    for n in &discovered {
                        injected_tool_names.insert(n.clone());
                    }
                }
            }
        }

        let max_len = sanitize::max_result_len_for_tool(name);
        let mut safe_result = sanitize::sanitize_tool_result_with_limit(&result, max_len);

        // Error recovery: append hint suggesting alternatives
        if is_error && error_recovery && name != "discover_tools" {
            let similar = registry.similar_tools(name, max_perm);
            let hint = AgentLoop::error_recovery_hint(name, &safe_result, &similar);
            safe_result = format!("{}\n{}", safe_result, hint);
            tracing::debug!(tool = name, "Injected error recovery hint");
        }

        if use_native_tools {
            let call_id = api_tool_calls.get(idx)
                .map(|tc| tc.id.as_str())
                .unwrap_or_else(|| "call_0");
            let tmpl = template_for_family(model_family);
            messages.push(tmpl.format_tool_result(call_id, name, &safe_result));
        } else {
            messages.push(ChatMessage::user(format!(
                "<data:tool_result name=\"{}\">{}",
                sanitize::escape_for_prompt(name),
                safe_result,
            )));
        }
    }
}

/// Parse tool names from discover_tools result (pipe-separated table).
fn parse_discovered_tool_names(
    result: &str,
    already_injected: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut names = Vec::new();
    for line in result.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() >= 4 {
            let name = parts[0].trim();
            if !name.is_empty()
                && name != "name"
                && !name.starts_with("---")
                && !name.starts_with("Found")
                && !already_injected.contains(name)
            {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// Postcondition verification for tool results.
///
/// For tools with verifiable outcomes, checks that the result contains
/// expected success markers. Returns `Verified` if postcondition is met,
/// `PartialSuccess` if ambiguous, or `Unverified` for tools without
/// postcondition definitions.
fn verify_postcondition(
    tool_name: &str,
    _args: &serde_json::Value,
    result: &str,
) -> yantrik_os::ToolOutcome {
    use yantrik_os::ToolOutcome;

    match tool_name {
        // Memory tools — verify storage confirmation
        "remember" => {
            if result.contains("Stored") || result.contains("stored")
                || result.contains("Remembered") || result.contains("saved")
            {
                ToolOutcome::Verified
            } else {
                ToolOutcome::PartialSuccess {
                    detail: "Memory store returned but no confirmation marker".into(),
                }
            }
        }
        "recall" => {
            // Recall always "succeeds" — even empty results are valid
            ToolOutcome::Verified
        }

        // File tools — verify operation markers
        "write_file" | "edit_file" => {
            if result.contains("Written") || result.contains("written")
                || result.contains("Saved") || result.contains("saved")
                || result.contains("Updated") || result.contains("updated")
                || result.contains("Edited") || result.contains("edited")
                || result.contains("Created") || result.contains("created")
            {
                ToolOutcome::Verified
            } else {
                ToolOutcome::PartialSuccess {
                    detail: "File operation returned but no write confirmation".into(),
                }
            }
        }
        "read_file" | "glob" | "grep" => {
            // Read operations succeed if they return content
            if !result.is_empty() {
                ToolOutcome::Verified
            } else {
                ToolOutcome::PartialSuccess {
                    detail: "File read returned empty content".into(),
                }
            }
        }

        // Command execution — check for exit code markers
        "run_command" | "code_execute" => {
            if result.contains("exit code: 0") || result.contains("Exit code: 0")
                || (!result.contains("exit code:") && !result.contains("Exit code:"))
            {
                // No exit code mentioned = likely succeeded (output only)
                ToolOutcome::Verified
            } else {
                ToolOutcome::PartialSuccess {
                    detail: "Command completed with non-zero exit code".into(),
                }
            }
        }

        // Email tools — verify send confirmation
        "email_send" | "email_reply" => {
            if result.contains("sent") || result.contains("Sent")
                || result.contains("delivered") || result.contains("queued")
            {
                ToolOutcome::Verified
            } else {
                ToolOutcome::PartialSuccess {
                    detail: "Email operation returned but no send confirmation".into(),
                }
            }
        }
        "email_check" | "email_list" | "email_read" | "email_search" => {
            ToolOutcome::Verified
        }

        // Browser tools — snapshot/navigate verification
        "browse" | "browser_snapshot" | "browser_see" => {
            if result.len() > 50 {
                ToolOutcome::Verified
            } else {
                ToolOutcome::PartialSuccess {
                    detail: "Browser returned minimal content".into(),
                }
            }
        }

        // Network tools
        "web_fetch" | "http_fetch" => {
            if result.len() > 20 && !result.contains("Connection refused")
                && !result.contains("timed out")
            {
                ToolOutcome::Verified
            } else {
                ToolOutcome::PartialSuccess {
                    detail: "Fetch returned but may have connectivity issues".into(),
                }
            }
        }
        "web_search" => {
            if result.contains("http") || result.contains("result") || result.len() > 100 {
                ToolOutcome::Verified
            } else {
                ToolOutcome::PartialSuccess {
                    detail: "Search returned but results may be empty".into(),
                }
            }
        }

        // System tools — always verified (info queries)
        "system_info" | "disk_usage" | "list_processes" | "date_calc"
        | "calculate" | "discover_tools" | "check_bond" | "get_weather"
        | "screenshot" | "list_files" | "search_files" => {
            ToolOutcome::Verified
        }

        // Vault tools
        "vault_store" | "vault_set_pin" => {
            if result.contains("Stored") || result.contains("stored")
                || result.contains("Set") || result.contains("set")
            {
                ToolOutcome::Verified
            } else {
                ToolOutcome::PartialSuccess {
                    detail: "Vault operation completed but no confirmation".into(),
                }
            }
        }

        // Delegation
        "spawn_agents" | "claude_think" | "claude_code" => {
            if result.len() > 50 {
                ToolOutcome::Verified
            } else {
                ToolOutcome::PartialSuccess {
                    detail: "Agent delegation returned minimal output".into(),
                }
            }
        }

        // Default — no postcondition defined
        _ => ToolOutcome::Unverified,
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

// ── Phase 2: Proactive Intelligence helpers ─────────────────────────────────

/// Extract (hour, day_of_week) from a Unix timestamp using simple math (UTC).
fn chrono_from_ts(ts: f64) -> (u32, u32) {
    let secs = ts as i64;
    let day_seconds = ((secs % 86400) + 86400) % 86400;
    let hour = (day_seconds / 3600) as u32;
    // Day of week: Jan 1 1970 was Thursday (4). 0=Sun..6=Sat
    let days_since_epoch = secs / 86400;
    let dow = (((days_since_epoch % 7) + 4) % 7) as u32;
    (hour, dow)
}

fn ensure_workflow_table(conn: &rusqlite::Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS workflow_observations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            hour INTEGER NOT NULL,
            day_of_week INTEGER NOT NULL,
            activity TEXT NOT NULL,
            observed_at REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_workflow_hour ON workflow_observations(hour);",
    )
    .ok();
}

fn ensure_maintenance_table(conn: &rusqlite::Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS maintenance_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_name TEXT NOT NULL,
            started_at REAL NOT NULL,
            completed_at REAL,
            status TEXT NOT NULL DEFAULT 'running',
            summary TEXT,
            reported INTEGER NOT NULL DEFAULT 0
        );",
    )
    .ok();
}

/// Query workflow hints: aggregated activity counts by hour.
fn query_workflow_hints(conn: &rusqlite::Connection) -> Vec<serde_json::Value> {
    let mut stmt = match conn.prepare(
        "SELECT hour, activity, COUNT(DISTINCT CAST(observed_at / 86400 AS INTEGER)) as days_observed
         FROM workflow_observations
         WHERE observed_at > ?1
         GROUP BY hour, activity
         HAVING days_observed >= 3
         ORDER BY hour, days_observed DESC",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let cutoff = now_ts() - 30.0 * 86400.0; // last 30 days
    stmt.query_map(rusqlite::params![cutoff], |row| {
        Ok(serde_json::json!({
            "hour": row.get::<_, i64>(0)?,
            "activity": row.get::<_, String>(1)?,
            "days_observed": row.get::<_, i64>(2)?,
        }))
    })
    .ok()
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// Count interactions in the last hour from bond events.
fn count_recent_interactions(conn: &rusqlite::Connection, since_ts: f64) -> u32 {
    conn.query_row(
        "SELECT COUNT(*) FROM bond_events WHERE timestamp > ?1",
        rusqlite::params![since_ts],
        |row| row.get::<_, u32>(0),
    )
    .unwrap_or(0)
}

/// Query recent maintenance log entries (last 24h, unreported first).
fn query_maintenance_log(conn: &rusqlite::Connection) -> Vec<serde_json::Value> {
    let mut stmt = match conn.prepare(
        "SELECT task_name, started_at, completed_at, status, summary, reported
         FROM maintenance_log
         WHERE started_at > ?1
         ORDER BY reported ASC, started_at DESC
         LIMIT 10",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let cutoff = now_ts() - 86400.0;
    stmt.query_map(rusqlite::params![cutoff], |row| {
        Ok(serde_json::json!({
            "task_name": row.get::<_, String>(0)?,
            "started_at": row.get::<_, f64>(1)?,
            "completed_at": row.get::<_, Option<f64>>(2)?,
            "status": row.get::<_, String>(3)?,
            "summary": row.get::<_, Option<String>>(4)?,
            "reported": row.get::<_, bool>(5)?,
        }))
    })
    .ok()
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// Format tool names into a human-readable progress message.
/// e.g. "browse" → "Browsing web...", "memory_stats" → "Checking memory..."
fn format_tool_progress(tool_names: &[&str], step: usize) -> String {
    let friendly: Vec<&str> = tool_names.iter().map(|name| match *name {
        "browse" | "browser_snapshot" | "browser_see" => "Browsing web",
        "browser_click_element" | "browser_click_xy" | "browser_type_element" | "browser_type_xy" => "Interacting with page",
        "web_search" | "browser_search" => "Searching the web",
        "remember" | "recall" | "save_user_fact" => "Checking memory",
        "memory_stats" | "review_memories" | "resolve_conflicts" => "Reviewing memories",
        "run_command" => "Running command",
        "read_file" => "Reading file",
        "write_file" => "Writing file",
        "list_files" | "search_files" => "Scanning files",
        "email_check" | "email_list" => "Checking email",
        "email_read" => "Reading email",
        "email_send" | "email_reply" => "Sending email",
        "calendar_today" | "calendar_list_events" => "Checking calendar",
        "calendar_create_event" => "Creating event",
        "system_info" => "Checking system",
        "telegram_send" => "Sending message",
        "http_fetch" => "Fetching data",
        "web_fetch" => "Fetching & analyzing page",
        "life_search" | "search_sources" => "Searching for options",
        "rank_results" => "Ranking results",
        "queue_task" => "Queuing task",
        "update_task" | "complete_task" => "Updating task",
        "discover_tools" => "Finding tools",
        _ => "Working",
    }).collect();

    // Deduplicate
    let mut unique: Vec<&str> = Vec::new();
    for f in &friendly {
        if !unique.contains(f) { unique.push(f); }
    }

    let desc = unique.join(", ");
    if step > 1 {
        format!("[Step {} — {}...]", step, desc.to_lowercase())
    } else {
        format!("[{}...]", desc)
    }
}

/// Load user interests from memory (preferences table or memories with interest keywords).
fn load_user_interests(conn: &rusqlite::Connection) -> Vec<String> {
    // Try preferences table first (from recall_preferences / save_user_fact)
    let mut interests: Vec<String> = Vec::new();

    // Query the preferences/facts for interest-related entries
    if let Ok(mut stmt) = conn.prepare(
        "SELECT value FROM user_preferences WHERE category IN ('hobby', 'interest', 'sport', 'food', 'music', 'shopping', 'travel', 'general')
         UNION
         SELECT content FROM memories WHERE content LIKE '%likes %' OR content LIKE '%interested in%' OR content LIKE '%hobby%' OR content LIKE '%favorite%'
         LIMIT 50"
    ) {
        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
            for row in rows.flatten() {
                // Extract the interest from the memory text
                let trimmed = row.trim().to_string();
                if !trimmed.is_empty() && !interests.contains(&trimmed) {
                    interests.push(trimmed);
                }
            }
        }
    }

    // Cap at 20 interests to keep state manageable
    interests.truncate(20);
    interests
}

/// Load user location from memory or config.
fn load_user_location(conn: &rusqlite::Connection) -> String {
    // Try preferences table
    if let Ok(location) = conn.query_row(
        "SELECT value FROM user_preferences WHERE category = 'location' ORDER BY updated_at DESC LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    ) {
        if !location.is_empty() {
            return location;
        }
    }

    // Try memories
    if let Ok(location) = conn.query_row(
        "SELECT content FROM memories WHERE content LIKE '%lives in%' OR content LIKE '%located in%' OR content LIKE '%location%' ORDER BY created_at DESC LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    ) {
        if !location.is_empty() {
            return location;
        }
    }

    String::new()
}

/// Strip Qwen3.5 `<think>...</think>` reasoning blocks from LLM output.
/// These are internal reasoning tokens that shouldn't be shown to the user.
/// Classify interaction domains from user text for CK-5 episode recording.
fn classify_interaction_domains(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut domains = Vec::new();
    if lower.contains("email") || lower.contains("inbox") {
        domains.push("communication".to_string());
    }
    if lower.contains("code") || lower.contains("debug") || lower.contains("build") {
        domains.push("development".to_string());
    }
    if lower.contains("schedule") || lower.contains("calendar") || lower.contains("meeting") {
        domains.push("planning".to_string());
    }
    if lower.contains("remember") || lower.contains("recall") || lower.contains("memory") {
        domains.push("memory".to_string());
    }
    if lower.contains("search") || lower.contains("find") || lower.contains("browse") {
        domains.push("research".to_string());
    }
    if domains.is_empty() {
        domains.push("general".to_string());
    }
    domains
}

/// Resolve {{var}} references in a template string using a string-valued variable map.
fn resolve_template_vars(template: &str, vars: &std::collections::HashMap<String, String>) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("{{{{{}}}}}", key), value);
    }
    result
}

fn strip_think_tags(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("<think>") {
        result.push_str(&remaining[..start]);
        let after = &remaining[start + "<think>".len()..];
        if let Some(end) = after.find("</think>") {
            remaining = &after[end + "</think>".len()..];
        } else {
            // Unclosed think tag — drop everything after it
            return result.trim().to_string();
        }
    }
    result.push_str(remaining);
    result.trim().to_string()
}
