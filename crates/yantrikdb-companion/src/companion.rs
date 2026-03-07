//! CompanionService — the main agent brain.
//!
//! Ties together YantrikDB (memory), LLMEngine (inference), instincts (drives),
//! urges (action queue), learning (memory extraction), bond tracking,
//! personality evolution, and self-narrative into a single 9-step pipeline.

use yantrikdb_core::YantrikDB;
use yantrikdb_ml::{
    ChatMessage, GenerationConfig, LLMBackend, ToolCall,
    format_tools, parse_tool_calls, extract_text_content,
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
use crate::urges::UrgeQueue;

/// Core tools always included in the LLM prompt — no discover_tools needed for these.
/// These cover the most common user needs. Everything else is discoverable.
pub const CORE_TOOLS: &[&str] = &[
    // Memory (always essential)
    "remember", "recall", "discover_tools",
    // Files & system
    "run_command", "read_file", "write_file", "list_files", "search_files",
    "system_info", "date_calc", "disk_usage",
    // Process & diagnostics
    "list_processes", "diagnose_process",
    // Scheduling & reminders
    "set_reminder", "create_schedule", "list_schedules",
    // Communication
    "telegram_send", "send_notification",
    // Browser (full workflow — launch → navigate → interact → read)
    "launch_browser", "browse", "browser_snapshot", "browser_scroll",
    "browser_click_element", "browser_type_element", "browser_search",
    // Browser vision (screenshot + AI analysis)
    "browser_see",
    // Browser coordinate-based interaction (works with ANY site — React, Shadow DOM, etc.)
    "browser_click_xy", "browser_type_xy",
    // Web search (lightweight, no browser needed)
    "web_search",
    // Life assistant (real-world search + ranking + preferences)
    "life_search", "recall_preferences", "save_user_fact",
    // Life assistant sub-tools (search pipeline)
    "search_sources", "extract_search_results", "rank_results",
    // Browser lifecycle (cleanup zombie processes)
    "browser_cleanup", "browser_status",
    // Network & HTTP
    "http_fetch", "network_diagnose",
    // Utility
    "calculate", "screenshot",
    // Email (check, list, read, send, reply, search)
    "email_check", "email_list", "email_read", "email_send", "email_reply", "email_search",
    // Calendar (today, list, create, update, delete)
    "calendar_today", "calendar_list_events", "calendar_create_event", "calendar_update_event", "calendar_delete_event",
    // Memory hygiene (review, conflicts, stats)
    "memory_stats", "resolve_conflicts", "review_memories",
    // Task queue (persistent work across think cycles)
    "queue_task", "list_tasks", "update_task", "complete_task",
    // Recipe engine (structured automation)
    "create_recipe", "list_recipes", "run_recipe",
    // Claude Code delegation (complex reasoning)
    "claude_think", "claude_code",
    // Weather & bond
    "get_weather", "check_bond",
    // Connectors (OAuth — Google, Spotify, etc.)
    "list_connections", "connect_service", "sync_service", "disconnect_service",
];

/// The companion agent — memory + inference + instincts + bond + evolution in one struct.
pub struct CompanionService {
    pub db: YantrikDB,
    pub llm: Box<dyn LLMBackend>,
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

    // Native tools JSON array — passed via API `tools` parameter for API backends.
    // llama-server with --jinja renders these into the chat template natively.
    native_core_tools: Vec<serde_json::Value>,

    // Whether to use native OpenAI tool calling format (API backend with --jinja).
    use_native_tools: bool,

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
}

impl CompanionService {
    /// Create a new companion from pre-built YantrikDB and LLM backend.
    pub fn new(db: YantrikDB, llm: Box<dyn LLMBackend>, config: CompanionConfig) -> Self {
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

        // Tool trace learning table
        ToolTraces::ensure_table(db.conn());

        // Persistent task queue for multi-cycle autonomous work
        crate::task_queue::TaskQueue::ensure_table(db.conn());
        // Recipe engine tables
        crate::recipe::RecipeStore::ensure_tables(db.conn());
        // Calendar local cache
        crate::calendar::ensure_table(db.conn());

        // Memory evolution tables + backfill existing memories
        memory_evolution::ensure_tables(db.conn());
        memory_evolution::ensure_weaving_tables(db.conn());
        memory_evolution::backfill_tiers(db.conn(), &config.memory_evolution);

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

        // Build stable tools prefix — core tools only for KV caching.
        // Full tool set is discoverable via discover_tools meta-tool.
        let max_perm = parse_permission(&config.tools.max_permission);
        let use_native_tools = llm.backend_name() == "api";

        // Native tools: JSON array for API `tools` parameter
        let native_core_tools = if config.tools.enabled {
            registry.definitions_for(CORE_TOOLS, max_perm)
        } else {
            Vec::new()
        };

        // Text-injected tools: only for non-API backends
        let tools_system_message = if config.tools.enabled && !use_native_tools {
            let core_defs = registry.definitions_for(CORE_TOOLS, max_perm);
            tracing::info!(
                core = core_defs.len(),
                total = registry.definitions(max_perm).len(),
                "Tools prefix: text-injected for {} backend",
                llm.backend_name(),
            );
            format_tools(&core_defs)
        } else {
            if config.tools.enabled {
                tracing::info!(
                    core = native_core_tools.len(),
                    total = registry.definitions(max_perm).len(),
                    "Native tool calling: {} core tools via API tools parameter",
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
            use_native_tools,
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

        // 3. Extend native core tools with extra tools from enabled skills
        if !snapshot.extra_core_tools.is_empty() {
            let max_perm = tools::parse_permission(&self.config.tools.max_permission);
            let extra_names: Vec<&str> = snapshot.extra_core_tools.iter().map(|s| s.as_str()).collect();
            let extra_defs = self.registry.definitions_for(&extra_names, max_perm);
            let added = extra_defs.len();
            self.native_core_tools.extend(extra_defs);
            tracing::info!(
                added,
                extra = ?snapshot.extra_core_tools,
                "Extended core tools from Skill Store"
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
        };
        self.registry.execute(&ctx, tool_name, args)
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

        // Step 1: Check session timeout
        self.check_session_timeout();

        // Step 2: Smart multi-signal recall (Gap 1+2)
        let smart = if self.config.memory_evolution.smart_recall_enabled {
            memory_evolution::smart_recall(&self.db, user_text, &self.config.memory_evolution)
        } else {
            let mems = self.db.recall_text(user_text, 5).unwrap_or_default();
            memory_evolution::SmartRecallResult::from_primary(mems)
        };
        let memories = smart.all_unique();
        let (recall_confidence, recall_hint) = (smart.confidence, smart.hint);

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

        // Step 6: Build bond-aware LLM context
        let personality = self.db.get_personality().ok();
        let patterns_json: Vec<serde_json::Value> = self
            .active_patterns
            .iter()
            .cloned()
            .collect();

        // Gather soul signals
        let narrative_text = Narrative::get(self.db.conn());
        let style = Evolution::get_style(self.db.conn());
        let opinions = Evolution::get_opinions(self.db.conn(), 3);
        let shared_refs = if self.config.memory_evolution.reference_freshness_enabled {
            memory_evolution::get_fresh_references(self.db.conn(), 3)
        } else {
            Evolution::get_shared_references(self.db.conn(), 3)
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
        };

        let context_messages = context::build_messages(
            user_text,
            &self.config,
            &state,
            &memories,
            &urges,
            &patterns_json,
            &self.conversation_history,
            personality.as_ref(),
            Some(&signals),
            self.use_native_tools,
        );

        // Build message array — single system message (Qwen3.5 requires it):
        // [0] system: context (+ text-injected tools for non-API backends)
        // [1..N-1] conversation history
        // [N] user query
        let max_perm = parse_permission(&self.config.tools.max_permission);
        let mut messages = Vec::with_capacity(context_messages.len() + 1);

        // Build native tools array (for API backend) or text prefix (for non-API)
        let needs_tools = self.config.tools.enabled && user_text.split_whitespace().count() > 2;
        let mut native_tools: Vec<serde_json::Value> = if self.use_native_tools {
            self.native_core_tools.clone()
        } else {
            Vec::new()
        };

        // Per-query relevant tool selection
        if needs_tools {
            let relevant: Vec<_> = ToolCache::select_relevant(
                self.db.conn(), &self.db, user_text, 10,
            ).into_iter().filter(|def| {
                let name = def["function"]["name"].as_str().unwrap_or("");
                !CORE_TOOLS.contains(&name)
            }).take(5).collect();

            if self.use_native_tools {
                // API backend: add to native tools array
                native_tools.extend(relevant);
            } else {
                // Non-API backend: text-inject into system message
                let mut tools_prefix = self.tools_system_message.clone();
                if !relevant.is_empty() {
                    tools_prefix.push_str(&format_tools(&relevant));
                }
                if !tools_prefix.is_empty() {
                    if let Some(first) = context_messages.first() {
                        let combined = format!("{}\n\n{}", tools_prefix, first.content);
                        messages.push(ChatMessage::system(&combined));
                        messages.extend_from_slice(&context_messages[1..]);
                    } else {
                        messages.push(ChatMessage::system(&tools_prefix));
                    }
                } else {
                    messages.extend(context_messages.clone());
                }
            }
        }

        // For API backend or when no tools needed: just use context messages directly
        if messages.is_empty() {
            if !self.use_native_tools && !self.tools_system_message.is_empty() && self.config.tools.enabled {
                // Non-API with core tools but no per-query tools: still text-inject core
                if let Some(first) = context_messages.first() {
                    let combined = format!("{}\n\n{}", self.tools_system_message, first.content);
                    messages.push(ChatMessage::system(&combined));
                    messages.extend_from_slice(&context_messages[1..]);
                } else {
                    messages.push(ChatMessage::system(&self.tools_system_message));
                }
            } else {
                messages.extend(context_messages);
            }
        }

        // Tool chain learning: inject trace hints into system prompt
        if self.config.agent.trace_learning && self.config.tools.enabled {
            let hints = ToolTraces::find_similar(
                self.db.conn(), &self.db, user_text, 3,
                self.config.agent.trace_min_similarity,
            );
            if !hints.is_empty() {
                let hint_text = ToolTraces::format_hints(&hints);
                // Inject into system message
                if let Some(sys_msg) = messages.first_mut() {
                    sys_msg.content.push_str(&hint_text);
                }
                // Mark hints as used
                for hint in &hints {
                    ToolTraces::mark_used(self.db.conn(), &hint.trace_id);
                }
            }
        }

        // Step 7: Call LLM with robust agent loop
        let gen_config = GenerationConfig {
            max_tokens: self.config.llm.max_tokens,
            temperature: self.config.llm.temperature,
            top_p: Some(0.9),
            ..Default::default()
        };

        let mut tool_calls_made = Vec::new();
        let mut injected_tool_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut response_text = String::new();
        let mut is_offline = false;
        let mut agent_loop = AgentLoop::new(user_text, self.config.agent.max_nudges);

        // Discovery rounds are limited; actual tool rounds reset the counter.
        let mut discovery_budget = self.config.tools.max_tool_rounds;
        let max_total_rounds = self.config.agent.max_steps.max(15);

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

            // Use native tool_calls if available, fall back to text parsing
            let tool_calls: Vec<ToolCall> = if !llm_response.tool_calls.is_empty() {
                llm_response.tool_calls.clone()
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
            );

            if !text_part.is_empty() {
                response_text = text_part;
            }
        }

        // If we exhausted rounds without a clean response, request summary
        if response_text.is_empty() && !tool_calls_made.is_empty() {
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
        }

        // Clean up tool call XML and Qwen3.5 thinking blocks from final response
        response_text = extract_text_content(&response_text);
        response_text = strip_think_tags(&response_text);

        // SecurityGuard: filter output for sensitive info leaks
        response_text = self.guard.check_response(&response_text, &self.db);

        // Update conversation history
        self.conversation_history
            .push(ChatMessage::user(user_text));
        self.conversation_history
            .push(ChatMessage::assistant(&response_text));

        // Trim history to max turns
        let max = self.config.conversation.max_history_turns * 2;
        if self.conversation_history.len() > max {
            let drain = self.conversation_history.len() - max;
            self.conversation_history.drain(..drain);
        }

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
        let memories = smart.all_unique();
        let (recall_confidence, recall_hint) = (smart.confidence, smart.hint);

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

        let signals = ContextSignals {
            self_memories: &self_memories,
            narrative: &narrative_text,
            style: &style,
            opinions: &opinions,
            shared_refs: &shared_refs,
            system_state: &self.system_context,
            recall_confidence,
            recall_hint: recall_hint.as_deref(),
        };

        let context_messages = context::build_messages(
            user_text,
            &self.config,
            &state,
            &memories,
            &urges,
            &patterns_json,
            &self.conversation_history,
            personality.as_ref(),
            Some(&signals),
            self.use_native_tools,
        );

        // Build message array — single system message (Qwen3.5 requires it):
        let max_perm = parse_permission(&self.config.tools.max_permission);
        let mut messages = Vec::with_capacity(context_messages.len() + 1);

        // Build native tools array (for API backend) or text prefix (for non-API)
        let needs_tools = self.config.tools.enabled && user_text.split_whitespace().count() > 2;
        let mut native_tools: Vec<serde_json::Value> = if self.use_native_tools {
            self.native_core_tools.clone()
        } else {
            Vec::new()
        };

        if needs_tools {
            let relevant: Vec<_> = ToolCache::select_relevant(
                self.db.conn(), &self.db, user_text, 10,
            ).into_iter().filter(|def| {
                let name = def["function"]["name"].as_str().unwrap_or("");
                !CORE_TOOLS.contains(&name)
            }).take(5).collect();

            if self.use_native_tools {
                native_tools.extend(relevant);
            } else {
                let mut tools_prefix = self.tools_system_message.clone();
                if !relevant.is_empty() {
                    tools_prefix.push_str(&format_tools(&relevant));
                }
                if !tools_prefix.is_empty() {
                    if let Some(first) = context_messages.first() {
                        let combined = format!("{}\n\n{}", tools_prefix, first.content);
                        messages.push(ChatMessage::system(&combined));
                        messages.extend_from_slice(&context_messages[1..]);
                    } else {
                        messages.push(ChatMessage::system(&tools_prefix));
                    }
                } else {
                    messages.extend(context_messages.clone());
                }
            }
        }

        if messages.is_empty() {
            if !self.use_native_tools && !self.tools_system_message.is_empty() && self.config.tools.enabled {
                if let Some(first) = context_messages.first() {
                    let combined = format!("{}\n\n{}", self.tools_system_message, first.content);
                    messages.push(ChatMessage::system(&combined));
                    messages.extend_from_slice(&context_messages[1..]);
                } else {
                    messages.push(ChatMessage::system(&self.tools_system_message));
                }
            } else {
                messages.extend(context_messages);
            }
        }

        // Tool chain learning: inject trace hints into system prompt
        if self.config.agent.trace_learning && self.config.tools.enabled {
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

        // Step 7: Call LLM with streaming + robust agent loop
        let gen_config = GenerationConfig {
            max_tokens: self.config.llm.max_tokens,
            temperature: self.config.llm.temperature,
            top_p: Some(0.9),
            ..Default::default()
        };

        let mut tool_calls_made = Vec::new();
        let mut injected_tool_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut response_text: String;
        let mut is_offline = false;
        let mut agent_loop = AgentLoop::new(user_text, self.config.agent.max_nudges);

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

                // Use native tool_calls if available, fall back to text parsing
                let tool_calls: Vec<ToolCall> = if !r.tool_calls.is_empty() {
                    r.tool_calls.clone()
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
                    );

                    // Remaining rounds: discovery rounds are budget-limited,
                    // actual tool rounds run until the hard cap.
                    response_text = display_text.clone();
                    let mut discovery_budget = self.config.tools.max_tool_rounds.saturating_sub(1);
                    let max_total_rounds = self.config.agent.max_steps.max(15);

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
        }

        response_text = extract_text_content(&response_text);
        response_text = strip_think_tags(&response_text);

        // SecurityGuard: filter output for sensitive info leaks
        response_text = self.guard.check_response(&response_text, &self.db);

        // Update conversation history
        self.conversation_history.push(ChatMessage::user(user_text));
        self.conversation_history
            .push(ChatMessage::assistant(&response_text));

        let max = self.config.conversation.max_history_turns * 2;
        if self.conversation_history.len() > max {
            let drain = self.conversation_history.len() - max;
            self.conversation_history.drain(..drain);
        }

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
        // Sync bond level so templates render with personality
        self.proactive_engine.set_bond_level(self.bond_level);
        if let Some(msg) = self.proactive_engine.check(&self.urge_queue, self.db.conn()) {
            self.set_proactive_message(msg);
        }
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
        // Keep conversation history bounded
        let max = 30;
        if self.conversation_history.len() > max {
            let drain = self.conversation_history.len() - max;
            self.conversation_history.drain(..drain);
        }

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
    api_tool_calls: &[yantrikdb_ml::ApiToolCall],
    native_tools: &mut Vec<serde_json::Value>,
    incognito: bool,
) {
    let ctx = ToolContext {
        db,
        max_permission: max_perm,
        registry_metadata: None,
        task_manager: Some(task_manager),
        incognito,
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
                    messages.push(ChatMessage::tool(call_id, name, &override_result));
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
                        let new_text = format_tools(&new_defs);
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
            // Native format: role="tool" with tool_call_id
            let call_id = api_tool_calls.get(idx)
                .map(|tc| tc.id.as_str())
                .unwrap_or_else(|| "call_0");
            messages.push(ChatMessage::tool(call_id, name, &safe_result));
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
    api_tool_calls: &[yantrikdb_ml::ApiToolCall],
    native_tools: &mut Vec<serde_json::Value>,
    agent_loop: &mut AgentLoop,
    error_recovery: bool,
    incognito: bool,
    mut cortex: Option<&mut crate::cortex::ContextCortex>,
) {
    let ctx = ToolContext {
        db,
        max_permission: max_perm,
        registry_metadata: None,
        task_manager: Some(task_manager),
        incognito,
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
                messages.push(ChatMessage::tool(call_id, name, &stop_msg));
            } else {
                messages.push(ChatMessage::user(&format!("[system] {}", stop_msg)));
            }
            continue;
        }

        // Execute the tool
        let result = if name == "discover_tools" {
            let metadata = registry.list_metadata(max_perm);
            let disc_ctx = ToolContext {
                db,
                max_permission: max_perm,
                registry_metadata: Some(&metadata),
                task_manager: Some(task_manager),
                incognito,
            };
            registry.execute(&disc_ctx, name, args)
        } else {
            registry.execute(&ctx, name, args)
        };

        guard.check_tool_result(name, &result, db);

        // Determine success/failure for agent loop tracking
        let is_error = result.starts_with("Error:")
            || result.starts_with("error:")
            || result.starts_with("Permission denied")
            || result.starts_with("Tool not found")
            || result.starts_with("BLOCKED");

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
                    messages.push(ChatMessage::tool(call_id, name, &override_result));
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
                        let new_text = format_tools(&new_defs);
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
            messages.push(ChatMessage::tool(call_id, name, &safe_result));
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
