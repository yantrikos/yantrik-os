//! Companion tools — memory tools + desktop system tools.
//!
//! Memory tools: remember, recall, relate, set_reminder,
//! introspect, form_opinion, create_inside_joke, check_bond.
//!
//! Desktop tools: open_url, read_clipboard, write_clipboard,
//! list_files, read_file, run_command.
//!
//! These are the functions the LLM can call during conversation.

use yantrikdb_core::YantrikDB;
use rusqlite::Connection;

/// Tool definitions in the format expected by format_tools().
pub fn companion_tool_defs() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "remember",
                "description": "Store something important about the user for later.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "description": "What to remember"},
                        "importance": {"type": "number", "description": "0.0-1.0"},
                        "memory_type": {
                            "type": "string",
                            "enum": ["episodic", "semantic", "procedural"]
                        },
                        "domain": {
                            "type": "string",
                            "description": "Topic: work, health, family, finance, hobby, general"
                        }
                    },
                    "required": ["text"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "recall",
                "description": "Search your memory for something about the user.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"}
                    },
                    "required": ["query"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "relate_entities",
                "description": "Note a relationship between two things.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "source": {"type": "string"},
                        "target": {"type": "string"},
                        "relationship": {"type": "string"}
                    },
                    "required": ["source", "target", "relationship"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "set_reminder",
                "description": "Set a reminder for the user at a specific time.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string"},
                        "remind_at": {"type": "string", "description": "ISO format YYYY-MM-DDTHH:MM"}
                    },
                    "required": ["text", "remind_at"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "introspect",
                "description": "Search your own self-memories — things you've observed about yourself.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "What to search your self-knowledge for"}
                    },
                    "required": ["query"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "form_opinion",
                "description": "Form or update your opinion on a topic.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "topic": {"type": "string"},
                        "stance": {"type": "string", "description": "Your opinion in 1-2 sentences"},
                        "confidence": {"type": "number", "description": "0.0-1.0 how confident you are"}
                    },
                    "required": ["topic", "stance"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "create_inside_joke",
                "description": "Save a shared reference or inside joke from this conversation.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "reference": {"type": "string", "description": "The inside joke or shared reference"},
                        "context": {"type": "string", "description": "What sparked it"}
                    },
                    "required": ["reference", "context"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "check_bond",
                "description": "Check your current bond level and relationship status with the user.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
    ]
}

/// Desktop tool definitions — system interaction tools for the AI shell.
pub fn desktop_tool_defs() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "open_url",
                "description": "Open a URL in the user's web browser.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "The URL to open"}
                    },
                    "required": ["url"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_clipboard",
                "description": "Read the current contents of the user's clipboard.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "write_clipboard",
                "description": "Write text to the user's clipboard.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "description": "Text to copy to clipboard"}
                    },
                    "required": ["text"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_files",
                "description": "List files in a directory on the user's system.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Directory path (e.g. ~/Downloads)"},
                        "pattern": {"type": "string", "description": "Optional glob pattern (e.g. *.pdf)"}
                    },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read the contents of a text file. Limited to first 2000 characters.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "File path to read"}
                    },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "run_command",
                "description": "Run a simple shell command. Only safe, read-only commands are allowed (ls, cat, date, uptime, df, free, whoami, pwd, echo).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {"type": "string", "description": "The command to run"}
                    },
                    "required": ["command"]
                }
            }
        }),
    ]
}

/// Execute a tool call and return the result as a string.
pub fn execute_tool(db: &YantrikDB, name: &str, args: &serde_json::Value) -> String {
    match name {
        // Memory tools
        "remember" => tool_remember(db, args),
        "recall" => tool_recall(db, args),
        "relate_entities" => tool_relate(db, args),
        "set_reminder" => tool_set_reminder(db, args),
        "introspect" => tool_introspect(db, args),
        "form_opinion" => tool_form_opinion(db.conn(), args),
        "create_inside_joke" => tool_create_inside_joke(db.conn(), args),
        "check_bond" => tool_check_bond(db.conn()),
        // Desktop tools
        "open_url" => tool_open_url(args),
        "read_clipboard" => tool_read_clipboard(),
        "write_clipboard" => tool_write_clipboard(args),
        "list_files" => tool_list_files(args),
        "read_file" => tool_read_file(args),
        "run_command" => tool_run_command(args),
        _ => format!("Unknown tool: {name}"),
    }
}

fn tool_remember(db: &YantrikDB, args: &serde_json::Value) -> String {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if text.is_empty() {
        return "Error: text is required".to_string();
    }

    let importance = args
        .get("importance")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5);
    let memory_type = args
        .get("memory_type")
        .and_then(|v| v.as_str())
        .unwrap_or("episodic");
    let domain = args
        .get("domain")
        .and_then(|v| v.as_str())
        .unwrap_or("general");

    match db.record_text(
        text,
        memory_type,
        importance,
        0.0,           // valence
        604800.0,      // half_life (7 days)
        &serde_json::json!({}),
        "default",     // namespace
        0.9,           // certainty
        domain,
        "companion",   // source
        None,          // emotional_state
    ) {
        Ok(rid) => format!("Remembered: {text} (id: {rid})"),
        Err(e) => format!("Failed to remember: {e}"),
    }
}

fn tool_recall(db: &YantrikDB, args: &serde_json::Value) -> String {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if query.is_empty() {
        return "Error: query is required".to_string();
    }

    match db.recall_text(query, 5) {
        Ok(results) => {
            if results.is_empty() {
                "No memories found matching that query.".to_string()
            } else {
                let mut out = String::from("Found memories:\n");
                for r in &results {
                    out.push_str(&format!("- {}\n", r.text));
                }
                out
            }
        }
        Err(e) => format!("Recall failed: {e}"),
    }
}

fn tool_relate(db: &YantrikDB, args: &serde_json::Value) -> String {
    let source = args
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let target = args
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let relationship = args
        .get("relationship")
        .and_then(|v| v.as_str())
        .unwrap_or("related_to");

    if source.is_empty() || target.is_empty() {
        return "Error: source and target are required".to_string();
    }

    match db.relate(source, target, relationship, 1.0) {
        Ok(edge_id) => format!("Noted: {source} --{relationship}--> {target} (edge: {edge_id})"),
        Err(e) => format!("Failed to relate: {e}"),
    }
}

fn tool_introspect(db: &YantrikDB, args: &serde_json::Value) -> String {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("self");

    // Search self-reflection memories
    match db.recall_text(query, 5) {
        Ok(results) => {
            let self_memories: Vec<_> = results
                .iter()
                .filter(|r| r.source == "self" || r.domain == "self-reflection")
                .collect();
            if self_memories.is_empty() {
                "I don't have any self-observations about that yet.".to_string()
            } else {
                let mut out = String::from("My self-observations:\n");
                for r in &self_memories {
                    out.push_str(&format!("- {}\n", r.text));
                }
                out
            }
        }
        Err(e) => format!("Introspection failed: {e}"),
    }
}

fn tool_form_opinion(conn: &Connection, args: &serde_json::Value) -> String {
    let topic = args
        .get("topic")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let stance = args
        .get("stance")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let confidence = args
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.6);

    if topic.is_empty() || stance.is_empty() {
        return "Error: topic and stance are required".to_string();
    }

    crate::evolution::Evolution::form_opinion(conn, topic, stance, confidence);
    format!("Opinion formed on '{topic}': {stance}")
}

fn tool_create_inside_joke(conn: &Connection, args: &serde_json::Value) -> String {
    let reference = args
        .get("reference")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let context = args
        .get("context")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    if reference.is_empty() {
        return "Error: reference is required".to_string();
    }

    let ref_id = crate::evolution::Evolution::add_shared_reference(conn, reference, context);
    format!("Inside joke saved: {reference} (id: {ref_id})")
}

fn tool_check_bond(conn: &Connection) -> String {
    let state = crate::bond::BondTracker::get_state(conn);
    format!(
        "Bond level: {} (score: {:.2})\nInteractions: {}\nDays together: {:.0}\nStreak: {} days\nVulnerability events: {}\nInside jokes: {}",
        state.bond_level.name(),
        state.bond_score,
        state.total_interactions,
        state.days_together,
        state.current_streak_days,
        state.vulnerability_events,
        state.shared_references,
    )
}

// ── Desktop tool implementations ──

fn tool_open_url(args: &serde_json::Value) -> String {
    let url = args.get("url").and_then(|v| v.as_str()).unwrap_or_default();
    if url.is_empty() {
        return "Error: url is required".to_string();
    }
    // Validate URL looks reasonable (no command injection)
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return "Error: URL must start with http:// or https://".to_string();
    }
    match std::process::Command::new("xdg-open").arg(url).spawn() {
        Ok(_) => format!("Opened: {url}"),
        Err(e) => format!("Failed to open URL: {e}"),
    }
}

fn tool_read_clipboard() -> String {
    // wl-paste for Wayland clipboard
    match std::process::Command::new("wl-paste")
        .arg("--no-newline")
        .output()
    {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            if text.is_empty() {
                "Clipboard is empty.".to_string()
            } else {
                // Limit to avoid blowing up context
                let truncated = if text.len() > 1000 { &text[..1000] } else { &text };
                format!("Clipboard contents:\n{truncated}")
            }
        }
        Ok(output) => {
            let err = String::from_utf8_lossy(&output.stderr);
            format!("Clipboard read failed: {err}")
        }
        Err(e) => format!("Failed to read clipboard (wl-paste not available?): {e}"),
    }
}

fn tool_write_clipboard(args: &serde_json::Value) -> String {
    let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default();
    if text.is_empty() {
        return "Error: text is required".to_string();
    }
    // wl-copy for Wayland clipboard
    let mut child = match std::process::Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return format!("Failed to write clipboard (wl-copy not available?): {e}"),
    };
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(text.as_bytes());
    }
    match child.wait() {
        Ok(s) if s.success() => "Copied to clipboard.".to_string(),
        Ok(s) => format!("wl-copy exited with: {s}"),
        Err(e) => format!("Failed to write clipboard: {e}"),
    }
}

fn tool_list_files(args: &serde_json::Value) -> String {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
    if path.is_empty() {
        return "Error: path is required".to_string();
    }
    // Expand ~ to home directory
    let expanded = if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            format!("{}/{}", home, &path[2..])
        } else {
            path.to_string()
        }
    } else {
        path.to_string()
    };

    let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("*");

    let dir = std::path::Path::new(&expanded);
    if !dir.is_dir() {
        return format!("Error: '{}' is not a directory", path);
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => return format!("Error reading directory: {e}"),
    };

    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Simple glob match: * matches anything, *.ext matches extension
        if pattern == "*" || glob_match(pattern, &name) {
            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let suffix = if is_dir { "/" } else { "" };
            files.push(format!("  {}{} ({})", name, suffix, format_size(size)));
        }
    }

    if files.is_empty() {
        format!("No files matching '{}' in {}", pattern, path)
    } else {
        files.sort();
        let count = files.len();
        // Limit output
        files.truncate(50);
        let mut result = format!("Files in {} ({} items):\n", path, count);
        result.push_str(&files.join("\n"));
        if count > 50 {
            result.push_str(&format!("\n  ... and {} more", count - 50));
        }
        result
    }
}

fn tool_read_file(args: &serde_json::Value) -> String {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
    if path.is_empty() {
        return "Error: path is required".to_string();
    }
    let expanded = if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            format!("{}/{}", home, &path[2..])
        } else {
            path.to_string()
        }
    } else {
        path.to_string()
    };

    match std::fs::read_to_string(&expanded) {
        Ok(content) => {
            if content.len() > 2000 {
                format!("{}\n... (truncated, {} total bytes)", &content[..2000], content.len())
            } else {
                content
            }
        }
        Err(e) => format!("Error reading file: {e}"),
    }
}

/// Safe command allowlist — only read-only, harmless commands.
const SAFE_COMMANDS: &[&str] = &[
    "ls", "cat", "head", "tail", "date", "uptime", "df", "free",
    "whoami", "pwd", "echo", "wc", "file", "stat", "uname",
    "hostname", "id", "which", "env", "printenv",
];

fn tool_run_command(args: &serde_json::Value) -> String {
    let command = args.get("command").and_then(|v| v.as_str()).unwrap_or_default();
    if command.is_empty() {
        return "Error: command is required".to_string();
    }

    // Extract the base command (first word)
    let base_cmd = command.split_whitespace().next().unwrap_or("");
    if !SAFE_COMMANDS.contains(&base_cmd) {
        return format!(
            "Error: '{}' is not in the safe command list. Allowed: {}",
            base_cmd,
            SAFE_COMMANDS.join(", ")
        );
    }

    // Block shell metacharacters that could enable injection
    if command.contains('|') || command.contains(';') || command.contains('&')
        || command.contains('`') || command.contains('$') || command.contains('>')
        || command.contains('<')
    {
        return "Error: shell metacharacters (|;&`$><) are not allowed".to_string();
    }

    match std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let mut result = String::new();
            if !stdout.is_empty() {
                let truncated = if stdout.len() > 2000 { &stdout[..2000] } else { &stdout };
                result.push_str(truncated);
            }
            if !stderr.is_empty() {
                result.push_str(&format!("\nStderr: {}", &stderr[..stderr.len().min(500)]));
            }
            if result.is_empty() {
                "(no output)".to_string()
            } else {
                result
            }
        }
        Err(e) => format!("Failed to run command: {e}"),
    }
}

/// Simple glob matching (supports * and *.ext patterns).
fn glob_match(pattern: &str, name: &str) -> bool {
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
fn format_size(bytes: u64) -> String {
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

fn tool_set_reminder(db: &YantrikDB, args: &serde_json::Value) -> String {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let remind_at_str = args
        .get("remind_at")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    if text.is_empty() || remind_at_str.is_empty() {
        return "Error: text and remind_at are required".to_string();
    }

    // Parse ISO datetime
    let remind_ts = match chrono::NaiveDateTime::parse_from_str(remind_at_str, "%Y-%m-%dT%H:%M") {
        Ok(dt) => dt.and_utc().timestamp() as f64,
        Err(_) => {
            return format!("Error: invalid remind_at format '{remind_at_str}', use YYYY-MM-DDTHH:MM");
        }
    };

    let metadata = serde_json::json!({
        "remind_at": remind_ts,
        "type": "reminder",
    });

    match db.record_text(
        text,
        "episodic",
        0.8,
        0.0,
        604800.0,
        &metadata,
        "default",
        0.9,
        "reminder",
        "companion",
        None,
    ) {
        Ok(rid) => format!("Reminder set for {remind_at_str}: {text} (id: {rid})"),
        Err(e) => format!("Failed to set reminder: {e}"),
    }
}
