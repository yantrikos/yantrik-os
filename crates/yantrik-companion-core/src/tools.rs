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
                    audit_log(ctx.db, name, tool.category(), args, &msg);
                    return msg;
                }

                // The second gate, and a different question from the first. Permission asks
                // whether this tool may ever run; this asks whether it may run *now*, given what
                // has already entered the conversation. See `crate::taint`.
                if let Err(refusal) = crate::taint::check_call(name, tool.category(), args) {
                    tracing::warn!("{}", refusal);
                    audit_log(ctx.db, name, tool.category(), args, &refusal);
                    return refusal;
                }

                let result = tool.execute(ctx, args);
                // Recorded after the fact, because what a tool returns is what taints the turn —
                // and a tool that failed returned nothing to be tainted by.
                crate::taint::note_call(name, tool.category(), args);
                audit_log(ctx.db, name, tool.category(), args, &result);
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

    /// The arguments this tool's own schema says it must have, that this call does not.
    ///
    /// Empty for a tool nobody registered: "unknown tool" is a different answer, and
    /// [`Self::execute`] is the one that gives it.
    pub fn missing_required_args(&self, name: &str, args: &serde_json::Value) -> Vec<String> {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|t| missing_required_args(&t.definition(), args))
            .unwrap_or_default()
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
fn audit_log(
    db: &YantrikDB,
    tool_name: &str,
    category: &str,
    args: &serde_json::Value,
    result: &str,
) {
    let summary = summarize_json(args);

    // What a secret-returning tool returned is the secret. The audit line goes into yantrikdb as
    // an ordinary memory — durable, embedded and searchable — so recording the first two hundred
    // characters of `vault_get` would file the user's passwords under "audit/tools" and hand them
    // to the next recall that happens to match.
    let result_preview = if crate::taint::returns_secret(tool_name, category) {
        "<withheld: this tool returns credentials>"
    } else {
        &result[..result.floor_char_boundary(200.min(result.len()))]
    };
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

/// Argument names whose values must never be written down.
///
/// Erring towards redacting too much: an audit line missing a value is a small loss, and one
/// containing a password is a durable, embedded, searchable copy of it.
///
/// Separators are stripped from the key before matching, so `api_key`, `apiKey` and `x-api-key`
/// are all one rule. Writing the spellings out instead is how `x-api-key` slipped through the
/// first version — the list had `api_key` and `apikey` and neither matches a hyphen.
const NEVER_RECORD: [&str; 11] = [
    "password",
    "authorization",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "apikey",
    "credential",
    "privatekey",
    "cookie",
    "session",
];

/// Short markers that must be a whole word of the key rather than any run of letters in it.
///
/// `pin` inside `shipping_address` and `auth` inside `author` are both false. Redacting those
/// costs nothing dramatic, but an audit log where half the fields say `<redacted>` for no reason
/// stops being read, and a log nobody reads is not an audit. As whole segments these still catch
/// `pin`, `pin_code`, `vault_pin`, `otp` and `oauth`.
const NEVER_RECORD_AS_WORD: [&str; 4] = ["pin", "auth", "otp", "key"];

/// Split a key into its words: `x-api-key`, `apiKey` and `api_key` all become `[api, key]`.
fn key_segments(key: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;

    for c in key.chars() {
        if !c.is_ascii_alphanumeric() {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
            previous_lower = false;
            continue;
        }
        // A capital after a lower-case letter starts a new word, so camelCase splits too.
        if c.is_ascii_uppercase() && previous_lower && !current.is_empty() {
            segments.push(std::mem::take(&mut current));
        }
        previous_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        current.push(c.to_ascii_lowercase());
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

fn is_sensitive_key(key: &str) -> bool {
    let segments = key_segments(key);
    let joined: String = segments.concat();

    if NEVER_RECORD.iter().any(|marker| joined.contains(marker)) {
        return true;
    }
    segments
        .iter()
        .any(|segment| NEVER_RECORD_AS_WORD.contains(&segment.as_str()))
}

/// Compact JSON summary for audit (keys only, truncated values).
///
/// Values were recorded verbatim, truncated at forty characters — which is longer than most
/// passwords. `vault_store` writes its argument straight into the memory the companion searches.
fn summarize_json(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::Object(map) => {
            let parts: Vec<String> = map
                .iter()
                .take(4)
                .map(|(k, v)| {
                    if is_sensitive_key(k) {
                        // The key is kept: knowing a PIN was supplied is the useful half of the
                        // audit, and the value is the half that must not survive.
                        return format!("{k}=<redacted>");
                    }
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

/// The names in a tool definition's `required` list that this call does not supply.
///
/// Every tool already publishes which of its arguments are compulsory — that is what the
/// `required` array in its JSON schema is for — but nothing read it, so a caller that composed
/// the arguments itself could ask a tool to run on nothing. On 22 September 2026 the query
/// planner wrote the step `{"tool": "recall", "args": {}}`, the recipe executor ran it, and
/// `recall` answered `Error: query is required`; the synthesis step then read that error as its
/// only evidence and wrote a sentence round it, which the desktop posted as one of the
/// companion's thoughts.
///
/// Present-but-empty counts as missing. `{"query": ""}` and `{"query": null}` fail inside the
/// tool for the same reason an absent key does, and a caller that can be told beforehand should
/// be told beforehand.
pub fn missing_required_args(definition: &serde_json::Value, args: &serde_json::Value) -> Vec<String> {
    let Some(required) = definition["function"]["parameters"]["required"].as_array() else {
        return Vec::new();
    };
    required
        .iter()
        .filter_map(|name| name.as_str())
        .filter(|name| {
            match args.get(name) {
                None | Some(serde_json::Value::Null) => true,
                Some(serde_json::Value::String(s)) => s.trim().is_empty(),
                Some(serde_json::Value::Array(a)) => a.is_empty(),
                Some(serde_json::Value::Object(o)) => o.is_empty(),
                _ => false,
            }
        })
        .map(str::to_string)
        .collect()
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

/// Paths the AI must never touch, matched anywhere in the path as text.
///
/// The first part is every place in `yantrik_ipc_contracts::home_paths::PROTECTED`, written out
/// again because a const cannot be built from another's entries; a test holds the two together.
/// `validate_path` also asks the shared list itself, a whole component at a time, after links
/// are followed. What follows is this list's own: places outside the home, and the work
/// directory.
pub const BLOCKED_SEGMENTS: &[&str] = &[
    ".ssh", ".gnupg", ".config/labwc", ".config/yantrik",
    "memory.db", ".bashrc", ".profile", ".bash_history",
    ".bash_profile", ".bash_login", ".bash_logout", ".zshrc", ".zshenv", ".zprofile", ".zlogin",
    ".pam_environment", ".config/autostart", ".config/environment.d", ".config/systemd",
    ".local/share/applications", ".config/mimeapps.list",
    "/etc/shadow", "/etc/passwd",
    // The work directory, where other programs (whisper, ffmpeg, edge-tts) write while following
    // links. Its runtime spelling is outside every root anyway; its home fallback is under $HOME,
    // so both are named here, from the same constants `private_dir` makes them with.
    yantrik_ml::private_dir::WORK_NAME, yantrik_ml::private_dir::WORK_HOME_REL,
];

/// Validate a path is safe for the AI to access.
/// Returns the expanded, validated path or an error string.
///
/// Defense layers:
/// 1. Block `..` traversal
/// 2. Block known sensitive path segments
/// 3. Restrict to $HOME or this account's private scratch directory
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

    let roots = allowed_roots();
    if roots.is_empty() {
        return Err(NO_HOME.to_string());
    }
    if !under_any(&expanded, &roots) {
        return Err("Path must be under your home directory".to_string());
    }

    resolves_within(std::path::Path::new(&expanded), &roots)?;
    Ok(expanded)
}

/// [`validate_path`] for a tool that writes: also nowhere hidden in the home, where programs read
/// their settings and startup (`home_paths::where_programs_look`). The protected list can never
/// name all of those - ~/.gitconfig, ~/.vimrc, ~/.local/bin - so a write goes into none of the
/// home's dot folders and dotfiles, nor ~/bin, wherever a link would take it. Reading keeps
/// `validate_path` alone.
///
/// The tools' own scratch directory is exempt, where its home fallback (~/.cache/yantrik/tmp)
/// would otherwise be refused: the model writes a diagram or a screenshot there to open it
/// again. Only when the path is inside it both as written and where it resolves, so a link left
/// in the scratch directory does not carry a write out of it.
pub fn validate_write_path(path: &str) -> Result<String, String> {
    let expanded = validate_path(path)?;
    let at = std::path::Path::new(&expanded);
    if in_scratch(at) {
        return Ok(expanded);
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).filter(|h| h.is_absolute());
    if home.is_some_and(|home| yantrik_ipc_contracts::home_paths::where_programs_look(at, &home)) {
        return Err(format!("Access denied: {}", yantrik_ipc_contracts::home_paths::HIDDEN_RULE));
    }
    Ok(expanded)
}

/// Whether `path` is inside the tools' scratch directory, as written and as resolved.
fn in_scratch(path: &std::path::Path) -> bool {
    let Ok(scratch) = yantrik_ml::private_dir::scratch_dir() else { return false };
    let roots = with_canonical(vec![scratch]);
    let resolved = yantrik_ipc_contracts::home_paths::resolve(path);
    under_any(&path.to_string_lossy(), &roots)
        && resolved.is_some_and(|real| under_any(&real.to_string_lossy(), &roots))
}

/// Where `path` really goes, links followed, must be inside `roots` and outside the protected
/// places. The deepest part of the path that resolves decides: looking only at the path and its
/// parent let a link two levels up (~/l -> /etc, asked as ~/l/X/y.txt) go unchecked whenever X
/// did not exist, and a write would then create X under /etc (#443). A link on the way that leads
/// nowhere is refused, since whatever is written through it lands where it points.
///
/// The part below the deepest that resolves does not exist yet, and is checked too, joined to
/// where the rest really is: through a link `~/x/c -> ~/.config`, `~/x/c/labwc/autostart`
/// resolves only as far as ~/.config, which is allowed, and a write would then create
/// ~/.config/labwc/autostart. Neither half names a protected place; the two together do.
fn resolves_within(path: &std::path::Path, roots: &[std::path::PathBuf]) -> Result<(), String> {
    if roots.is_empty() {
        return Err(NO_HOME.to_string());
    }
    let mut probe = path.to_path_buf();
    loop {
        match probe.canonicalize() {
            Ok(resolved) => {
                let tail = path.strip_prefix(&probe).unwrap_or(std::path::Path::new(""));
                let whole = resolved.join(tail);
                let resolved_str = whole.to_string_lossy().to_string();
                for blocked in BLOCKED_SEGMENTS {
                    if resolved_str.contains(blocked) {
                        return Err(format!("Access denied: path resolves to protected location ({blocked})"));
                    }
                }
                if yantrik_ipc_contracts::home_paths::is_protected(&whole) {
                    return Err("Access denied: path resolves to a protected location".to_string());
                }
                if !under_any(&resolved.to_string_lossy(), roots) {
                    return Err("Access denied: path resolves outside your home directory".to_string());
                }
                return Ok(());
            }
            Err(_) => {
                if probe.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
                    return Err("Access denied: the path goes through a link that leads nowhere".to_string());
                }
                match probe.parent() {
                    Some(parent) => probe = parent.to_path_buf(),
                    None => return Err("Access denied: no part of the path exists".to_string()),
                }
            }
        }
    }
}

/// The refusal when there is no root at all: "must be under your home directory" sent the model
/// looking for a mistake in a path that had none, when the account running it has no usable HOME.
const NO_HOME: &str = "Access denied: there is no home directory to work in";

/// Where the file tools may reach: the home directory, and the private scratch directory the
/// tools write their own outputs to (a diagram, a screenshot) so the model can open them again.
///
/// Not the shared /tmp: anything another account left there, at a name it chose, is text the
/// model would read as the person's. And not an empty or relative HOME, which as a string prefix
/// matched every path there is.
///
/// Each root is listed as written and as resolved. The path as the model wrote it is checked
/// before resolution and the canonical path after, and each must meet its own kind: a HOME that
/// is a link (/home/ann -> /data/ann) would otherwise refuse every file really inside it, because
/// its canonical paths start /data/ann.
fn allowed_roots() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        let home = std::path::PathBuf::from(home);
        if home.is_absolute() && home != std::path::Path::new("/") {
            roots.push(home);
        }
    }
    if let Ok(scratch) = yantrik_ml::private_dir::scratch_dir() {
        roots.push(scratch);
    }
    with_canonical(roots)
}

/// `roots` plus the canonical form of each that resolves somewhere else. A root that resolves
/// to `/` is dropped in both spellings: it would admit every path there is.
fn with_canonical(roots: Vec<std::path::PathBuf>) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for root in roots {
        let canon = root.canonicalize().ok();
        if canon.as_deref() == Some(std::path::Path::new("/")) {
            continue;
        }
        if let Some(canon) = canon.filter(|c| *c != root) {
            out.push(canon);
        }
        out.push(root);
    }
    out
}

/// Whether `path` is one of `roots` or inside one, compared a component at a time, so
/// /home/ann does not admit /home/anne.
fn under_any(path: &str, roots: &[std::path::PathBuf]) -> bool {
    let path = std::path::Path::new(path);
    roots.iter().any(|root| path.starts_with(root))
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

#[cfg(test)]
mod required_args_tests {
    //! A tool publishes which of its arguments are compulsory. Until a caller reads that, the
    //! only way to find out is to run the tool and read the error it returns — which is what
    //! the recipe executor used to do, and what the companion then said out loud.

    use super::*;

    #[test]
    fn a_tool_call_that_supplies_nothing_is_caught_before_it_runs() {
        // `recall`'s own schema, as it publishes it.
        let definition = serde_json::json!({
            "type": "function",
            "function": {
                "name": "recall",
                "description": "Search stored memories; read-only",
                "parameters": {
                    "type": "object",
                    "properties": { "query": { "type": "string" } },
                    "required": ["query"]
                }
            }
        });

        // What the query planner actually wrote on 22 September.
        assert_eq!(
            missing_required_args(&definition, &serde_json::json!({})),
            vec!["query".to_string()]
        );
        // The three other ways of saying nothing.
        for empty in [
            serde_json::json!({ "query": "" }),
            serde_json::json!({ "query": "   " }),
            serde_json::json!({ "query": null }),
        ] {
            assert_eq!(missing_required_args(&definition, &empty), vec!["query".to_string()],
                "{empty}");
        }

        // A real call is left alone, and so is a placeholder a later step will fill in.
        assert!(missing_required_args(&definition, &serde_json::json!({ "query": "morning brief" }))
            .is_empty());
        assert!(missing_required_args(&definition, &serde_json::json!({ "query": "{{topic}}" }))
            .is_empty());

        // A tool that requires nothing is never in the way.
        let no_args = serde_json::json!({
            "function": { "parameters": { "type": "object", "properties": {} } }
        });
        assert!(missing_required_args(&no_args, &serde_json::json!({})).is_empty());
    }
}

#[cfg(test)]
mod audit_tests {
    //! The audit line is written into yantrikdb as an ordinary memory: durable, embedded, and
    //! returned by recall. Anything it records about a credential is a copy of that credential
    //! that outlives the conversation and can be searched for.

    use super::*;

    #[test]
    fn a_password_argument_is_not_written_down() {
        let args = serde_json::json!({ "service": "reddit.com", "password": "hunter2" });
        let line = summarize_json(&args);
        assert!(!line.contains("hunter2"), "the password reached the audit log: {line}");
        // The key survives, because "a password was supplied" is the useful half.
        assert!(line.contains("password=<redacted>"), "{line}");
        assert!(line.contains("reddit.com"), "and the rest must still be legible: {line}");
    }

    #[test]
    fn a_key_that_only_looks_sensitive_is_left_alone() {
        // Over-redaction has a cost too: an audit log full of <redacted> stops being read. `pin`
        // as a bare substring matches `shipping_address`, which is why short markers are only
        // matched at a boundary.
        for key in ["shipping_address", "spinner", "author", "keyboard_layout", "session_count_display"] {
            let args = serde_json::json!({ key: "ordinary" });
            let line = summarize_json(&args);
            if key == "session_count_display" {
                // This one *is* redacted, and deliberately: anything with "session" in the name is
                // more likely to be a token than a counter, and the cost of being wrong is uneven.
                continue;
            }
            assert!(line.contains("ordinary"), "{key} was redacted for no reason: {line}");
        }
    }

    #[test]
    fn every_spelling_of_a_secret_is_caught() {
        for key in [
            "password", "passwd", "new_password", "PIN", "pin", "api_key", "apiKey",
            "x-api-key", "token", "refresh_token", "secret", "authorization",
            "private_key", "cookie", "session_id", "otp", "passphrase",
        ] {
            let args = serde_json::json!({ key: "SENSITIVE-VALUE" });
            let line = summarize_json(&args);
            assert!(!line.contains("SENSITIVE-VALUE"), "{key} leaked: {line}");
        }
    }

    #[test]
    fn ordinary_arguments_are_still_recorded() {
        // Redacting everything would make the audit log useless, which is its own failure.
        let args = serde_json::json!({ "service": "github.com", "limit": 5 });
        let line = summarize_json(&args);
        assert!(line.contains("github.com"), "{line}");
        assert!(line.contains("5"), "{line}");
    }

    #[test]
    fn what_a_credential_tool_returned_is_not_recorded() {
        // The other half: `vault_get` returns the passwords themselves, and the audit line used
        // to keep the first two hundred characters of whatever came back.
        assert!(crate::taint::returns_secret("vault_get", "vault"));
        assert!(!crate::taint::returns_secret("browse", "browser"));
    }
}

#[cfg(test)]
mod path_root_tests {
    use super::{under_any, validate_path, with_canonical};
    use std::path::PathBuf;

    #[test]
    fn the_work_dir_is_out_of_reach_in_both_spellings() {
        // Other programs write there while following links; the model must not be able to plant
        // anything in it, even through the home fallback that sits under $HOME.
        let work_in_home = format!("~/{}/voice-0123/voice.wav", yantrik_ml::private_dir::WORK_HOME_REL);
        let err = validate_path(&work_in_home).unwrap_err();
        assert!(err.contains("not allowed"), "{err}");
        let work_in_runtime = format!("/run/user/1000/{}/voice-0123/voice.wav", yantrik_ml::private_dir::WORK_NAME);
        assert!(validate_path(&work_in_runtime).is_err());
        if let Ok(work) = yantrik_ml::private_dir::work_dir() {
            assert!(validate_path(work.join("x.txt").to_str().unwrap()).is_err());
        }
    }

    #[test]
    fn a_link_deep_in_the_path_is_followed_to_where_it_leads() {
        use super::resolves_within;
        let root = std::env::temp_dir().join(format!("yantrik-resolves-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("notes")).unwrap();
        let root = root.canonicalize().unwrap();
        std::os::unix::fs::symlink("/etc", root.join("escape")).unwrap();
        std::os::unix::fs::symlink("/etc/yantrik-no-such-thing", root.join("dangling")).unwrap();
        let roots = [root.clone()];
        let at = |p: &str| resolves_within(&root.join(p), &roots);

        assert!(at("notes/new.txt").is_ok(), "a new file in a real folder");
        assert!(at("notes/new/deeper/file.txt").is_ok(), "new folders under a real one");
        for out in ["escape/passwd", "escape/new.txt", "escape/no-such-dir/new.txt", "escape/a/b/c/d.txt"] {
            assert!(at(out).is_err(), "{out} leads out through the link");
        }
        for nowhere in ["dangling", "dangling/new.txt"] {
            assert!(at(nowhere).is_err(), "{nowhere} goes through a link that leads nowhere");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_protected_name_split_across_a_link_is_refused() {
        use super::resolves_within;
        let root = std::env::temp_dir().join(format!("yantrik-resolves-split-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".config")).unwrap();
        std::fs::create_dir_all(root.join(".ssh")).unwrap();
        std::fs::create_dir_all(root.join("x")).unwrap();
        let root = root.canonicalize().unwrap();
        std::os::unix::fs::symlink(root.join(".config"), root.join("x/c")).unwrap();
        std::os::unix::fs::symlink(root.join(".ssh"), root.join("keys")).unwrap();
        let roots = [root.clone()];
        let at = |p: &str| resolves_within(&root.join(p), &roots);

        for refused in ["x/c/labwc/autostart", "x/c/yantrik/config.yaml", "x/c/autostart/a.desktop", "keys/authorized_keys", "keys/new/deeper"] {
            let err = at(refused).unwrap_err();
            assert!(err.contains("protected location"), "{refused}: {err}");
        }
        assert!(at("x/c/gtk-3.0/settings.ini").is_ok(), "an ordinary folder under the link");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn with_no_root_at_all_the_refusal_says_there_is_no_home() {
        let err = super::resolves_within(std::path::Path::new("/home/ann/notes.txt"), &[]).unwrap_err();
        assert!(err.ends_with("there is no home directory to work in"), "{err}");
    }

    #[test]
    fn a_write_goes_nowhere_hidden_in_the_home_but_a_read_may() {
        for hidden in ["~/.vimrc", "~/.gitconfig", "~/.config/nvim/init.lua", "~/.local/bin/x", "~/bin/x", "~/.cargo/config.toml"] {
            let err = super::validate_write_path(hidden).unwrap_err();
            assert!(err.contains("hidden folders or dotfiles"), "{hidden}: {err}");
        }
        assert!(super::validate_path("~/.gitconfig").is_ok(), "reading a dotfile that is not protected");
        assert!(super::validate_write_path("~/yantrik-write-rule-test.txt").is_ok());
        if let Ok(scratch) = yantrik_ml::private_dir::scratch_dir() {
            let own = scratch.join("diagram.svg");
            assert!(super::validate_write_path(own.to_str().unwrap()).is_ok(), "the tools' own scratch");
        }
    }

    #[test]
    fn every_place_every_side_protects_is_blocked_here_too() {
        for place in yantrik_ipc_contracts::home_paths::PROTECTED {
            assert!(super::BLOCKED_SEGMENTS.contains(place), "BLOCKED_SEGMENTS is missing {place}");
        }
    }

    #[test]
    fn a_root_admits_itself_and_what_is_inside_it_only() {
        let roots = [PathBuf::from("/home/ann"), PathBuf::from("/run/user/1000/yantrik-scratch")];
        assert!(under_any("/home/ann", &roots));
        assert!(under_any("/home/ann/notes.txt", &roots));
        assert!(under_any("/run/user/1000/yantrik-scratch/diagram.png", &roots));
        assert!(!under_any("/run/user/1000/yantrik/companion.sock", &roots), "the socket dir is not scratch");
        assert!(!under_any("/home/anne/notes.txt", &roots), "a name that starts the same is not inside");
        assert!(!under_any("/tmp/planted.txt", &roots), "the shared /tmp is not ours");
        assert!(!under_any("/etc/shadow", &roots));
    }

    #[test]
    fn no_roots_admit_nothing() {
        // An unset or empty HOME used to be the empty prefix, which every path starts with.
        assert!(!under_any("/etc/shadow", &[]));
        assert!(!under_any("/", &[]));
    }

    #[cfg(unix)]
    fn scratch_base(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("yantrik-roots-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[cfg(unix)]
    #[test]
    fn a_root_that_is_a_link_admits_what_is_really_inside_it() {
        let base = scratch_base("link");
        let real = base.join("data-ann");
        std::fs::create_dir(&real).unwrap();
        let home = base.join("home-ann");
        std::os::unix::fs::symlink(&real, &home).unwrap();
        let roots = with_canonical(vec![home.clone()]);
        // The resolved path, as validate_path checks it after canonicalizing.
        let resolved = real.canonicalize().unwrap().join("notes.txt");
        assert!(under_any(resolved.to_str().unwrap(), &roots));
        // And the path as written, as it is checked before.
        assert!(under_any(home.join("notes.txt").to_str().unwrap(), &roots));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn a_root_that_resolves_to_slash_is_dropped() {
        let base = scratch_base("slash");
        let home = base.join("home");
        std::os::unix::fs::symlink("/", &home).unwrap();
        assert!(with_canonical(vec![home]).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }
}
