//! Tool trait, context, registry, and shared helpers.
//!
//! Lives in companion-core so that sub-crates (companion-tools) can implement
//! tools without depending on the full companion crate.

use crate::permission::PermissionLevel;
use yantrikdb_core::YantrikDB;

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

/// Shared context passed to every tool at execution time.
pub struct ToolContext<'a> {
    pub db: &'a YantrikDB,
    /// Maximum permission level allowed. Tools above this are denied.
    pub max_permission: PermissionLevel,
    /// Tool metadata for discover_tools (populated by companion).
    pub registry_metadata: Option<&'a [ToolMetadata]>,
    /// Background task manager (type-erased; downcast in heavy tools).
    pub task_manager: Option<&'a dyn std::any::Any>,
    /// When true, tools that persist data should skip saving.
    pub incognito: bool,
    /// Context for spawning parallel sub-agents (type-erased; downcast in heavy tools).
    pub agent_spawner: Option<&'a dyn std::any::Any>,
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

                // The second gate, and a different question from the first. Permission asks
                // whether this tool may ever run; this asks whether it may run *now*, given what
                // has already entered the conversation. See `crate::taint`.
                if let Err(refusal) = crate::taint::check(name, tool.category()) {
                    tracing::warn!("{}", refusal);
                    audit_log(ctx.db, name, args, &refusal);
                    return refusal;
                }

                let result = tool.execute(ctx, args);
                // Recorded after the fact, because what a tool returns is what taints the turn —
                // and a tool that failed returned nothing to be tainted by.
                crate::taint::note(name, tool.category());
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

/// Extract first sentence from description (for compact metadata).
pub fn first_sentence(text: &str, max_len: usize) -> String {
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
pub const BLOCKED_SEGMENTS: &[&str] = &[
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
    let canon_path = std::path::Path::new(&expanded);
    if canon_path.exists() {
        match canon_path.canonicalize() {
            Ok(resolved) => {
                let resolved_str = resolved.to_string_lossy().to_string();
                for blocked in BLOCKED_SEGMENTS {
                    if resolved_str.contains(blocked) {
                        return Err(format!(
                            "Access denied: path resolves to protected location ({})",
                            blocked
                        ));
                    }
                }
                if !resolved_str.starts_with(&home) && !resolved_str.starts_with("/tmp") {
                    return Err(
                        "Access denied: path resolves outside your home directory".to_string()
                    );
                }
            }
            Err(_) => {}
        }
    } else if let Some(parent) = canon_path.parent() {
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


#[cfg(test)]
mod gate_tests {
    //! Does the gate actually fire?
    //!
    //! `taint`'s own tests check the policy. These drive the real [`ToolRegistry::execute`] with
    //! real tools, because a correct policy that is never consulted protects nothing — and the
    //! wiring is one forgotten line, in a function that has two other early returns.

    use super::*;
    use crate::permission::PermissionLevel;

    /// A tool that does nothing but claim a name and a category, which is all the policy reads.
    struct Fake {
        name: &'static str,
        category: &'static str,
    }

    impl Tool for Fake {
        fn name(&self) -> &'static str {
            self.name
        }
        fn permission(&self) -> PermissionLevel {
            PermissionLevel::Safe
        }
        fn category(&self) -> &'static str {
            self.category
        }
        fn definition(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
            format!("{} ran", self.name)
        }
    }

    fn registry() -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(Fake { name: "browse", category: "browser" }));
        reg.register(Box::new(Fake { name: "vault_get", category: "vault" }));
        reg.register(Box::new(Fake { name: "browser_login", category: "browser" }));
        reg.register(Box::new(Fake { name: "run_command", category: "system" }));
        reg
    }

    /// The audit log writes to the database, so a real one is needed even though nothing reads it
    /// back. In memory, so the test leaves nothing behind.
    fn db() -> yantrikdb_core::YantrikDB {
        yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database")
    }

    fn ctx(db: &yantrikdb_core::YantrikDB) -> ToolContext<'_> {
        ToolContext {
            db,
            max_permission: PermissionLevel::Dangerous,
            registry_metadata: None,
            task_manager: None,
            incognito: true,
            agent_spawner: None,
        }
    }

    #[test]
    fn a_page_cannot_make_the_agent_fetch_a_credential() {
        let db = db();
        let ctx = ctx(&db);
        let reg = registry();
        crate::taint::begin_turn();

        assert_eq!(reg.execute(&ctx, "browse", &serde_json::json!({})), "browse ran");

        let refused = reg.execute(&ctx, "vault_get", &serde_json::json!({}));
        assert!(refused.starts_with("Refused:"), "the gate did not fire: {refused}");
        assert!(refused.contains("browse"), "and it must name what caused it: {refused}");
    }

    #[test]
    fn the_ordinary_case_is_untouched() {
        // If the rule breaks ordinary work it will be turned off, and then it protects nothing.
        let db = db();
        let ctx = ctx(&db);
        let reg = registry();
        crate::taint::begin_turn();

        assert_eq!(reg.execute(&ctx, "browse", &serde_json::json!({})), "browse ran");
        assert_eq!(
            reg.execute(&ctx, "run_command", &serde_json::json!({})),
            "run_command ran",
            "no credential is in play, so there is nothing to leak"
        );
    }

    #[test]
    fn logging_in_mid_session_still_works() {
        let db = db();
        let ctx = ctx(&db);
        let reg = registry();
        crate::taint::begin_turn();

        reg.execute(&ctx, "browse", &serde_json::json!({}));
        assert_eq!(
            reg.execute(&ctx, "browser_login", &serde_json::json!({})),
            "browser_login ran",
            "a tool that never returns the secret must stay usable after browsing"
        );
    }

    #[test]
    fn the_trifecta_is_refused_through_the_registry() {
        let db = db();
        let ctx = ctx(&db);
        let reg = registry();
        crate::taint::begin_turn();

        reg.execute(&ctx, "vault_get", &serde_json::json!({}));
        reg.execute(&ctx, "browse", &serde_json::json!({}));

        let refused = reg.execute(&ctx, "run_command", &serde_json::json!({}));
        assert!(refused.starts_with("Refused:"), "{refused}");
        assert!(refused.contains("vault_get") && refused.contains("browse"), "{refused}");
    }

    #[test]
    fn a_failed_tool_does_not_taint_what_follows() {
        // `note` runs after execute, so a tool that never produced content never made the turn
        // untrusted. Checked because the ordering is easy to get backwards.
        let db = db();
        let ctx = ctx(&db);
        let reg = registry();
        crate::taint::begin_turn();

        let unknown = reg.execute(&ctx, "no_such_tool", &serde_json::json!({}));
        assert!(unknown.starts_with("Unknown tool"));
        assert_eq!(
            reg.execute(&ctx, "vault_get", &serde_json::json!({})),
            "vault_get ran",
            "a tool that did not run cannot have brought anything in"
        );
    }
}
