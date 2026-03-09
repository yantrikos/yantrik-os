//! Tool Store — modular, trait-based tool registry with permission model.
//!
//! Each tool category lives in its own file and registers tools via
//! `register(&mut ToolRegistry)`. Mirrors the instincts pattern.
//!
//! Categories (32 modules, 116+ tools):
//! - memory:     remember, recall, relate, set_reminder, introspect, form_opinion, create_inside_joke, check_bond
//! - desktop:    open_url, read_clipboard, write_clipboard, list_files, read_file, run_command
//! - files:      write_file, manage_files, search_files, file_info
//! - system:     kill_process, send_notification, system_control
//! - network:    download_file, http_fetch
//! - media:      screenshot, audio_control, audio_info
//! - display:    display_info, set_resolution
//! - archive:    archive_create, archive_extract
//! - text:       word_count, diff_files, hash_file
//! - time:       date_calc, timer
//! - process:    list_processes, system_info
//! - wifi:       wifi_scan, wifi_connect, wifi_status, wifi_disconnect
//! - bluetooth:  bluetooth_scan, bluetooth_pair, bluetooth_connect, bluetooth_disconnect, bluetooth_info
//! - package:    package_search, package_install, package_remove, package_info, package_list
//! - service:    service_list, service_control, service_status
//! - encoding:   base64_encode, base64_decode, url_encode, json_format
//! - disk:       disk_usage, mount_info, dir_size
//! - wallpaper:  set_wallpaper
//! - git:        git_status, git_log, git_diff, git_clone, git_branch, git_commit, git_show, git_stash, git_diff_file
//! - weather:    get_weather
//! - calculator: calculate, unit_convert
//! - window:     list_windows, focus_window, close_window
//! - firewall:   firewall_status, firewall_list_rules, firewall_allow_port, firewall_block_port, firewall_block_ip, firewall_enable, firewall_disable
//! - antivirus:  antivirus_scan, antivirus_status, antivirus_update, antivirus_quarantine
//! - networking: network_interfaces, network_ping, network_traceroute, network_ports, network_dns, network_dns_set, network_vpn_status
//! - terminal:   read_terminal_buffer
//! - workspace:  save_workspace, recall_workspace
//! - knowledge:  search_fix_history, search_by_timeframe, summarize_work_session
//! - terminal_analysis: detect_terminal_errors, search_terminal_history, explain_last_error
//! - project:    detect_projects, set_active_project, get_project_context
//! - docker:     docker_ps, docker_images, docker_logs, docker_start, docker_stop, docker_exec
//! - ssh:        ssh_list_hosts, ssh_check_host, ssh_run
//! - artifacts:  generate_fix_summary, list_fixes, read_fix
//! - home_assistant: ha_get_state, ha_call_service, ha_list_entities
//! - browser:    launch_browser, browse, browser_read, browser_click, browser_type, browser_screenshot, browser_tabs, browser_search
//! - browser_lifecycle: browser_cleanup, browser_status (+ watchdog_check, kill_all_browsers)
//! - background_tasks: run_background, list_background_tasks, check_background_task, stop_background_task
//! - clipboard:  clipboard_history, clipboard_analyze, clipboard_fetch_url, clipboard_transform, text_action
//! - automation: create_automation, list_automations, run_automation, delete_automation, toggle_automation
//! - plugin:     (dynamic — loaded from ~/.config/yantrik/plugins/*.yaml)

pub mod memory;
pub mod desktop;
pub mod files;
pub mod system;
pub mod network;
pub mod media;
pub mod display;
pub mod archive;
pub mod text;
pub mod time;
pub mod process;
pub mod wifi;
pub mod bluetooth;
pub mod package;
pub mod service;
pub mod encoding;
pub mod disk;
pub mod wallpaper;
pub mod git;
pub mod weather;
pub mod calculator;
pub mod window;
pub mod firewall;
pub mod antivirus;
pub mod networking;
pub mod terminal;
pub mod workspace;
pub mod knowledge;
pub mod terminal_analysis;
pub mod project;
pub mod docker;
pub mod ssh;
pub mod artifacts;
pub mod home_assistant;
pub mod browser;
pub mod browser_lifecycle;
pub mod discovery;
pub mod background_tasks;
pub mod scheduler;
pub mod telegram;
pub mod memory_hygiene;
pub mod clipboard;
pub mod automation;
pub mod vision;
pub mod canvas;
pub mod connector;
pub mod email;
pub mod calendar;
pub mod task_queue;
pub mod recipe;
pub mod claude;
pub mod vault;
pub mod coder;
pub mod plugin;
pub mod spawn_agents;
pub mod edit;
pub mod grep;
pub mod glob;
pub mod mcp;
pub mod whatsapp;
pub mod github;

use crate::config::CompanionConfig;
use yantrikdb_core::YantrikDB;

// ── Permission Level ──

/// Risk level for a tool. Ordered so a single comparison gates access:
/// `tool.permission() > ctx.max_permission` → deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PermissionLevel {
    /// Read-only, no state change. Always allowed.
    Safe,
    /// Writes data but reversible (write file, clipboard, remember).
    Standard,
    /// System state changes (kill process, volume, resolution).
    Sensitive,
    /// Destructive/irreversible (delete files, shutdown).
    Dangerous,
}

impl std::fmt::Display for PermissionLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Safe => write!(f, "safe"),
            Self::Standard => write!(f, "standard"),
            Self::Sensitive => write!(f, "sensitive"),
            Self::Dangerous => write!(f, "dangerous"),
        }
    }
}

/// Parse a permission level from config string. Defaults to Sensitive.
pub fn parse_permission(s: &str) -> PermissionLevel {
    match s.to_lowercase().as_str() {
        "safe" => PermissionLevel::Safe,
        "standard" => PermissionLevel::Standard,
        "sensitive" => PermissionLevel::Sensitive,
        "dangerous" => PermissionLevel::Dangerous,
        _ => PermissionLevel::Sensitive,
    }
}

// ── Tool trait ──

/// A tool the companion LLM can invoke during conversation.
pub trait Tool: Send + Sync {
    /// Tool name used in LLM tool calls (e.g. "write_file").
    fn name(&self) -> &'static str;

    /// Risk level — checked against `ToolContext::max_permission` before execute.
    fn permission(&self) -> PermissionLevel;

    /// Category for grouping (e.g. "memory", "files", "system").
    fn category(&self) -> &'static str;

    /// JSON schema definition consumed by `format_tools()`.
    fn definition(&self) -> serde_json::Value;

    /// Execute the tool. Returns a result string fed back to the LLM.
    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String;
}

/// Context needed to spawn parallel sub-agents from a tool.
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

/// Shared context passed to every tool at execution time.
pub struct ToolContext<'a> {
    pub db: &'a YantrikDB,
    /// Maximum permission level allowed. Tools above this are denied.
    pub max_permission: PermissionLevel,
    /// Tool metadata for discover_tools (populated by companion).
    pub registry_metadata: Option<&'a [ToolMetadata]>,
    /// Background task manager for long-running processes.
    pub task_manager: Option<&'a std::sync::Mutex<crate::task_manager::TaskManager>>,
    /// When true, tools that persist data should skip saving.
    pub incognito: bool,
    /// Context for spawning parallel sub-agents.
    pub agent_spawner: Option<&'a AgentSpawnerContext>,
}

/// Compact tool metadata for discovery (no full JSON schema).
#[derive(Debug, Clone)]
pub struct ToolMetadata {
    pub name: &'static str,
    pub category: &'static str,
    pub permission: PermissionLevel,
    pub description: String,
}

// ── Tool Registry ──

/// Registry that holds all available tools and dispatches calls.
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    /// All tool definitions (for `format_tools()`).
    /// Only includes tools within the given permission ceiling.
    pub fn definitions(&self, max_permission: PermissionLevel) -> Vec<serde_json::Value> {
        self.tools
            .iter()
            .filter(|t| t.permission() <= max_permission)
            .map(|t| t.definition())
            .collect()
    }

    /// Execute a tool by name with permission gate and audit logging.
    pub fn execute(&self, ctx: &ToolContext, name: &str, args: &serde_json::Value) -> String {
        for tool in &self.tools {
            if tool.name() == name {
                // Permission gate
                if tool.permission() > ctx.max_permission {
                    let msg = format!(
                        "Permission denied: '{}' requires {} but max is {}",
                        name,
                        tool.permission(),
                        ctx.max_permission
                    );
                    tracing::warn!("{}", msg);
                    audit_log(ctx.db, name, args, &msg);
                    return msg;
                }

                let result = tool.execute(ctx, args);
                audit_log(ctx.db, name, args, &result);
                return result;
            }
        }
        format!("Unknown tool: {name}")
    }

    /// Compact metadata listing for tool discovery.
    pub fn list_metadata(&self, max_permission: PermissionLevel) -> Vec<ToolMetadata> {
        self.tools
            .iter()
            .filter(|t| t.permission() <= max_permission)
            .map(|t| {
                let def = t.definition();
                let full_desc = def["function"]["description"].as_str().unwrap_or("");
                ToolMetadata {
                    name: t.name(),
                    category: t.category(),
                    permission: t.permission(),
                    description: first_sentence(full_desc, 80),
                }
            })
            .collect()
    }

    /// Full JSON schemas for specific tool names (permission-filtered).
    pub fn definitions_for(
        &self,
        names: &[&str],
        max_permission: PermissionLevel,
    ) -> Vec<serde_json::Value> {
        self.tools
            .iter()
            .filter(|t| t.permission() <= max_permission && names.contains(&t.name()))
            .map(|t| t.definition())
            .collect()
    }

    /// Find tools in the same category as the given tool (for error recovery).
    /// Returns up to 3 alternative tool names.
    pub fn similar_tools(&self, tool_name: &str, max_permission: PermissionLevel) -> Vec<String> {
        let category = self.tools.iter()
            .find(|t| t.name() == tool_name)
            .map(|t| t.category());
        let category = match category {
            Some(c) => c,
            None => return Vec::new(),
        };
        self.tools
            .iter()
            .filter(|t| {
                t.category() == category
                    && t.name() != tool_name
                    && t.permission() <= max_permission
            })
            .take(3)
            .map(|t| t.name().to_string())
            .collect()
    }
}

/// Extract first sentence from description (for compact metadata).
fn first_sentence(text: &str, max_len: usize) -> String {
    let end = text
        .find(". ")
        .map(|i| i + 1)
        .unwrap_or(text.len())
        .min(max_len);
    let mut boundary = end;
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let result = &text[..boundary];
    if boundary < text.len() {
        format!("{}...", result.trim_end_matches('.'))
    } else {
        result.to_string()
    }
}

/// Log a tool execution to YantrikDB memory for AI self-recall.
fn audit_log(db: &YantrikDB, tool_name: &str, args: &serde_json::Value, result: &str) {
    let summary = summarize_json(args);
    let result_preview = &result[..result.floor_char_boundary(200.min(result.len()))];
    let text = format!("Tool: {tool_name}({summary}) → {result_preview}");
    let _ = db.record_text(
        &text,
        "semantic",
        0.3,
        0.0,
        604800.0,
        &serde_json::json!({}),
        "default",
        0.9,
        "audit/tools",
        "self",
        None,
    );
}

/// Compact JSON summary for audit (keys only, truncated values).
/// Uses ASCII "..." for truncation to avoid creating non-ASCII URLs in memory
/// that the LLM might copy verbatim.
fn summarize_json(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Object(map) => {
            let parts: Vec<String> = map
                .iter()
                .take(4)
                .map(|(k, v)| {
                    let short = match v {
                        serde_json::Value::String(s) if s.len() > 40 => {
                            format!("\"{}...\"", &s[..s.floor_char_boundary(40)])
                        }
                        serde_json::Value::String(s) => format!("\"{}\"", s),
                        _ => {
                            let s = v.to_string();
                            if s.len() > 40 { format!("{}...", &s[..s.floor_char_boundary(40)]) } else { s }
                        }
                    };
                    format!("{k}={short}")
                })
                .collect();
            parts.join(", ")
        }
        _ => val.to_string(),
    }
}

// ── Factory (mirrors load_instincts) ──

/// Build the full tool registry with all categories.
pub fn build_registry(config: &CompanionConfig) -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    memory::register(&mut reg);
    desktop::register(&mut reg);
    files::register(&mut reg);
    system::register(&mut reg);
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
        network::register(&mut reg, &ollama_base, &model);
    }
    media::register(&mut reg);
    display::register(&mut reg);
    archive::register(&mut reg);
    text::register(&mut reg);
    time::register(&mut reg);
    process::register(&mut reg);
    wifi::register(&mut reg);
    bluetooth::register(&mut reg);
    package::register(&mut reg);
    service::register(&mut reg);
    encoding::register(&mut reg);
    disk::register(&mut reg);
    wallpaper::register(&mut reg);
    git::register(&mut reg);
    weather::register(&mut reg);
    calculator::register(&mut reg);
    window::register(&mut reg);
    firewall::register(&mut reg);
    antivirus::register(&mut reg);
    networking::register(&mut reg);
    terminal::register(&mut reg);
    workspace::register(&mut reg);
    knowledge::register(&mut reg);
    terminal_analysis::register(&mut reg);
    project::register(&mut reg);
    docker::register(&mut reg);
    ssh::register(&mut reg);
    artifacts::register(&mut reg);
    browser::register(&mut reg);
    browser_lifecycle::register(&mut reg);
    discovery::register(&mut reg);
    background_tasks::register(&mut reg);
    scheduler::register(&mut reg);
    memory_hygiene::register(&mut reg);
    clipboard::register(&mut reg);
    automation::register(&mut reg);
    task_queue::register(&mut reg);
    recipe::register(&mut reg);
    claude::register(&mut reg);
    vault::register(&mut reg);
    coder::register(&mut reg);
    // Life assistant tools — need Ollama for LLM extraction
    if config.llm.is_api_backend() {
        let api_url = config.llm.resolve_api_base_url().unwrap_or_default();
        let la_ollama = api_url.trim_end_matches("/v1").trim_end_matches('/').to_string();
        let la_model = config.llm.api_model.as_deref().unwrap_or("qwen3.5:9b");
        crate::life_assistant::register(&mut reg, &la_ollama, la_model);
    } else {
        // Fallback: register with empty ollama (heuristic-only extraction)
        crate::life_assistant::register(&mut reg, "", "");
    }

    // Conditionally register Home Assistant tools
    let ha = &config.home_assistant;
    if ha.enabled {
        if let (Some(base_url), Some(token)) = (&ha.base_url, &ha.token) {
            home_assistant::register(&mut reg, base_url, token);
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
        // Convert OpenAI-compat URL to Ollama native URL for vision
        // e.g. "http://host:11434/v1" → "http://host:11434"
        let api_url = config.llm.resolve_api_base_url().unwrap_or_default();
        let ollama_base = api_url.trim_end_matches("/v1").trim_end_matches('/').to_string();
        let model = config.llm.api_model.as_deref().unwrap_or("qwen3.5:9b");
        vision::register(&mut reg, &ollama_base, model);
        canvas::register(&mut reg, &ollama_base, model);
        browser::register_vision(&mut reg, &ollama_base, model);
        tracing::info!(base = %ollama_base, model, "Vision & Canvas tools registered");
    }

    // GitHub API tools (works without auth for public repos)
    github::register(&mut reg, config.connectors.github_token.as_deref());

    // Load YAML plugins from ~/.config/yantrik/plugins/
    plugin::load_plugins(&mut reg);
    spawn_agents::register(&mut reg);
    edit::register(&mut reg);
    grep::register(&mut reg);
    glob::register(&mut reg);

    // Connect MCP servers and register their tools
    if !config.mcp_servers.is_empty() {
        mcp::register_mcp_servers(&mut reg, &config.mcp_servers);
    }

    reg
}

// ── Shared helpers ──

/// Expand `~/` to `$HOME/`.
pub fn expand_home(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, &path[2..]);
        }
    }
    path.to_string()
}

/// Paths the AI must never touch.
const BLOCKED_SEGMENTS: &[&str] = &[
    ".ssh", ".gnupg", ".config/labwc", ".config/yantrik",
    "memory.db", ".bashrc", ".profile", ".bash_history",
    "/etc/shadow", "/etc/passwd",
];

/// Validate a path is safe for the AI to access.
/// Returns the expanded, validated path or an error string.
///
/// Defense layers:
/// 1. Block `..` traversal
/// 2. Block known sensitive path segments
/// 3. Restrict to $HOME or /tmp
/// 4. Resolve symlinks and re-validate the canonical path
pub fn validate_path(path: &str) -> Result<String, String> {
    let expanded = expand_home(path);

    // Block paths with .. traversal
    if expanded.contains("..") {
        return Err("Path traversal (..) is not allowed".to_string());
    }

    // Check against blocked segments (pre-resolution check)
    for blocked in BLOCKED_SEGMENTS {
        if expanded.contains(blocked) {
            return Err(format!("Access to '{blocked}' is not allowed"));
        }
    }

    // Must be under $HOME or /tmp
    let home = std::env::var("HOME").unwrap_or_default();
    if !expanded.starts_with(&home) && !expanded.starts_with("/tmp") {
        return Err("Path must be under your home directory or /tmp".to_string());
    }

    // Resolve symlinks: if the path exists, canonicalize and re-validate.
    // This prevents symlink-based bypass (e.g. ~/link -> ~/.ssh/).
    let canon_path = std::path::Path::new(&expanded);
    if canon_path.exists() {
        match canon_path.canonicalize() {
            Ok(resolved) => {
                let resolved_str = resolved.to_string_lossy().to_string();
                // Re-check blocked segments on the resolved (real) path
                for blocked in BLOCKED_SEGMENTS {
                    if resolved_str.contains(blocked) {
                        return Err(format!(
                            "Access denied: path resolves to protected location ({})",
                            blocked
                        ));
                    }
                }
                // Re-check home/tmp constraint on resolved path
                if !resolved_str.starts_with(&home) && !resolved_str.starts_with("/tmp") {
                    return Err(
                        "Access denied: path resolves outside your home directory".to_string()
                    );
                }
            }
            Err(_) => {
                // Can't resolve — path might have broken symlinks, allow the
                // original expanded path (already validated above)
            }
        }
    }
    // For parent directory: if writing a new file, check the parent is safe
    else if let Some(parent) = canon_path.parent() {
        if parent.exists() {
            if let Ok(resolved_parent) = parent.canonicalize() {
                let rp = resolved_parent.to_string_lossy().to_string();
                for blocked in BLOCKED_SEGMENTS {
                    if rp.contains(blocked) {
                        return Err(format!(
                            "Access denied: parent directory resolves to protected location ({})",
                            blocked
                        ));
                    }
                }
                if !rp.starts_with(&home) && !rp.starts_with("/tmp") {
                    return Err(
                        "Access denied: parent directory resolves outside your home directory"
                            .to_string(),
                    );
                }
            }
        }
    }

    Ok(expanded)
}

/// Simple glob matching (supports `*`, `*.ext`, `prefix*`).
pub fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(ext) = pattern.strip_prefix("*.") {
        return name.ends_with(&format!(".{}", ext));
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    pattern == name
}

/// Format byte size into human-readable string.
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
