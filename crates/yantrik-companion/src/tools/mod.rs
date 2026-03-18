//! Tool Store — modular, trait-based tool registry with permission model.
//!
//! Pure tools (48 modules) live in `yantrik-companion-tools` for faster builds.
//! Heavy tools (14 modules) that depend on companion internals stay here.

// Heavy tools — depend on companion-specific modules (automation, scheduler, etc.)
pub mod automation;
pub mod background_tasks;
pub mod calendar;
pub mod connector;
pub mod email;
pub mod mcp;
pub mod memory;
pub mod open_loops;
pub mod recipe;
pub mod scheduler;
pub mod spawn_agents;
pub mod task_queue;
pub mod telegram;
pub mod whatsapp;

use crate::config::CompanionConfig;

// Re-export Tool infrastructure from companion-core.
// This preserves all `use super::{Tool, ToolContext, ...}` imports in heavy tool modules.
pub use yantrik_companion_core::tools::*;
pub use yantrik_companion_core::permission::{PermissionLevel, parse_permission};

// Re-export pure tools so existing `crate::tools::browser::*` paths still work.
pub use yantrik_companion_tools::browser;
pub use yantrik_companion_tools::vision;
pub use yantrik_companion_tools::canvas;
pub use yantrik_companion_tools::github;
pub use yantrik_companion_tools::home_assistant;
pub use yantrik_companion_tools::network;
pub use yantrik_companion_tools::plugin;
pub use yantrik_companion_tools::discovery;

/// Context needed to spawn parallel sub-agents from a tool.
/// Stays in companion because it depends on companion-specific types.
/// Heavy tools downcast from `ctx.agent_spawner` via `Any`.
#[derive(Clone)]
pub struct AgentSpawnerContext {
    pub llm: std::sync::Arc<dyn yantrik_ml::LLMBackend>,
    pub db_path: String,
    pub embedding_dim: usize,
    pub max_steps: usize,
    pub max_tokens: usize,
    pub temperature: f64,
    pub user_name: String,
    pub config: crate::config::CompanionConfig,
}

// ── Factory ──

/// Build the full tool registry with all categories.
pub fn build_registry(config: &CompanionConfig) -> ToolRegistry {
    let mut reg = ToolRegistry::new();

    // Register all 48 pure tools from sub-crate
    yantrik_companion_tools::register_all(&mut reg);

    // Heavy tools (depend on companion internals)
    memory::register(&mut reg);
    background_tasks::register(&mut reg);
    scheduler::register(&mut reg);
    automation::register(&mut reg);
    task_queue::register(&mut reg);
    recipe::register(&mut reg);
    spawn_agents::register(&mut reg);
    open_loops::register(&mut reg);

    // Network tools — web_fetch gets LLM for AI extraction
    {
        let (ollama_base, model) = if config.llm.is_api_backend() {
            let url = config.llm.resolve_api_base_url().unwrap_or_default();
            let base = url.trim_end_matches("/v1").trim_end_matches('/').to_string();
            let mdl = config.llm.api_model.as_deref().unwrap_or("qwen3.5:35b").to_string();
            (base, mdl)
        } else {
            (String::new(), String::new())
        };
        yantrik_companion_tools::network::register(&mut reg, &ollama_base, &model);
    }

    // Life assistant tools — need Ollama for LLM extraction
    if config.llm.is_api_backend() {
        let api_url = config.llm.resolve_api_base_url().unwrap_or_default();
        let la_ollama = api_url.trim_end_matches("/v1").trim_end_matches('/').to_string();
        let la_model = config.llm.api_model.as_deref().unwrap_or("qwen3.5:9b");
        crate::life_assistant::register(&mut reg, &la_ollama, la_model);
    } else {
        crate::life_assistant::register(&mut reg, "", "");
    }

    // Conditionally register Home Assistant tools
    let ha = &config.home_assistant;
    if ha.enabled {
        if let (Some(base_url), Some(token)) = (&ha.base_url, &ha.token) {
            yantrik_companion_tools::home_assistant::register(&mut reg, base_url, token);
            tracing::info!("Home Assistant tools registered ({})", base_url);
        } else {
            tracing::warn!("Home Assistant enabled but base_url or token missing — skipping");
        }
    }

    // Conditionally register Telegram tools
    let tg = &config.telegram;
    if tg.enabled {
        if let (Some(token), Some(chat_id)) = (&tg.bot_token, &tg.chat_id) {
            telegram::register(&mut reg, token, chat_id);
            tracing::info!("Telegram tool registered");
        } else {
            tracing::warn!("Telegram enabled but bot_token or chat_id missing — skipping");
        }
    }

    // Conditionally register WhatsApp tools
    let wa = &config.whatsapp;
    if wa.enabled {
        if let (Some(phone_id), Some(token)) = (&wa.phone_number_id, &wa.access_token) {
            let recipient = wa.recipient.as_deref().unwrap_or("");
            whatsapp::register(&mut reg, phone_id, token, recipient);
            tracing::info!("WhatsApp tool registered");
        } else {
            tracing::warn!("WhatsApp enabled but phone_number_id or access_token missing — skipping");
        }
    }

    // Conditionally register email tools
    if config.email.enabled && !config.email.accounts.is_empty() {
        email::register(&mut reg, config.email.accounts.clone());
        tracing::info!("Email tools registered ({} accounts)", config.email.accounts.len());
    }

    // Conditionally register calendar tools (reuses email OAuth2)
    if config.calendar.enabled && !config.email.accounts.is_empty() {
        calendar::register(&mut reg, config.email.accounts.clone(), config.calendar.account.clone());
        tracing::info!("Calendar tools registered");
    }

    // Register vision tools (if using API backend like Ollama with vision support)
    if config.llm.is_api_backend() {
        let api_url = config.llm.resolve_api_base_url().unwrap_or_default();
        let ollama_base = api_url.trim_end_matches("/v1").trim_end_matches('/').to_string();
        let model = config.llm.api_model.as_deref().unwrap_or("qwen3.5:9b");
        yantrik_companion_tools::vision::register(&mut reg, &ollama_base, model);
        yantrik_companion_tools::canvas::register(&mut reg, &ollama_base, model);
        yantrik_companion_tools::browser::register_vision(&mut reg, &ollama_base, model);
        tracing::info!(base = %ollama_base, model, "Vision & Canvas tools registered");
    }

    // GitHub API tools (works without auth for public repos)
    yantrik_companion_tools::github::register(&mut reg, config.connectors.github_token.as_deref());

    // Connect MCP servers and register their tools
    if !config.mcp_servers.is_empty() {
        mcp::register_mcp_servers(&mut reg, &config.mcp_servers);
    }

    // Connector tools are registered separately in companion.rs (need ConnectorState)

    reg
}

// Shared helpers (validate_path, expand_home, glob_match, format_size, BLOCKED_SEGMENTS)
// are re-exported from yantrik_companion_core::tools::* above.
