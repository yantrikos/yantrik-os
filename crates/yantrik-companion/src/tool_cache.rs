//! Tool definition cache — embeds and indexes tool definitions for smart selection.
//!
//! Instead of sending all 70+ tool definitions to the LLM on every query,
//! this module embeds each tool description and selects only the most relevant
//! tools per query via cosine similarity.

use rusqlite::Connection;
use serde_json::Value;
use yantrikdb_core::YantrikDB;

/// Tool definitions cached with embeddings for semantic selection.
pub struct ToolCache;

/// Tools that are always included regardless of similarity score.
/// Mirrors CORE_TOOLS in companion.rs — keep in sync.
const ALWAYS_INCLUDE: &[&str] = &[
    "remember", "recall", "discover_tools",
    "run_command", "read_file", "write_file", "list_files", "search_files",
    "system_info", "current_time",
    "set_reminder", "create_schedule", "list_schedules",
    "telegram_send", "calculator",
];

/// Keyword → tool groups. When query matches keywords, these tools are force-included.
const KEYWORD_TOOLS: &[(&[&str], &[&str])] = &[
    (
        &["browse", "browser", "web", "website", "url", "http", "google", "search online",
          "reddit", "twitter", "youtube", "hacker news", "open site", "check site",
          "internet", "webpage", "page", "subreddit", "login", "sign in"],
        &["launch_browser", "browse", "browser_read", "browser_search", "browser_tabs",
          "browser_screenshot", "browser_click", "browser_type"],
    ),
    (
        &["git", "commit", "branch", "repo", "push", "pull", "merge", "diff", "log",
          "clone", "stash", "checkout"],
        &["git_status", "git_log", "git_diff", "git_commit", "git_branch", "git_checkout"],
    ),
    (
        &["docker", "container", "image", "compose", "pod", "kubernetes"],
        &["docker_ps", "docker_images", "docker_run", "docker_stop", "docker_logs"],
    ),
    (
        &["file", "folder", "directory", "download", "read", "write", "copy", "move",
          "delete", "create", "open file", "save"],
        &["list_files", "read_file", "write_file", "file_info", "search_files"],
    ),
    (
        &["wifi", "network", "ip", "ping", "dns", "connected", "internet connection",
          "bluetooth", "vpn"],
        &["wifi_status", "wifi_scan", "wifi_connect", "network_info"],
    ),
    (
        &["process", "kill", "cpu", "ram", "memory usage", "disk", "storage", "space",
          "battery", "top", "htop", "system info", "timezone", "time zone", "clock",
          "date", "hostname", "reboot", "shutdown", "update", "upgrade", "install",
          "config", "configure", "admin", "sudo", "command", "run", "execute", "shell"],
        &["list_processes", "system_info", "disk_usage", "run_command"],
    ),
    (
        &["screenshot", "screen", "capture", "wallpaper", "display", "brightness",
          "resolution", "monitor"],
        &["screenshot", "set_wallpaper", "display_info"],
    ),
    (
        &["weather", "temperature", "forecast", "rain", "sunny", "climate"],
        &["weather_current", "weather_forecast"],
    ),
    (
        &["timer", "alarm", "remind", "schedule", "countdown", "stopwatch",
          "time", "date", "clock"],
        &["set_timer", "set_alarm", "current_time"],
    ),
    (
        &["calculate", "math", "convert", "compute", "equation", "percentage"],
        &["calculator"],
    ),
    (
        &["background", "task", "running", "spawn", "long running", "track",
          "monitor task", "background task"],
        &["run_background", "list_background_tasks", "check_background_task",
          "stop_background_task"],
    ),
    (
        &["schedule", "scheduled", "cron", "recurring", "every day", "every week",
          "every hour", "repeat", "periodic", "birthday", "anniversary",
          "remind me", "reminder"],
        &["create_schedule", "list_schedules", "update_schedule", "cancel_schedule",
          "set_reminder"],
    ),
    (
        &["telegram", "message phone", "send message", "text me", "notify me",
          "reach me", "phone"],
        &["telegram_send"],
    ),
    (
        &["memory", "memories", "forget", "clean", "purge", "review", "conflict",
          "conflicts", "duplicate", "noise", "tier", "curate", "hygiene",
          "memory health", "memory stats", "too many memories"],
        &["memory_stats", "review_memories", "forget_memory", "update_memory",
          "resolve_conflicts", "purge_system_noise"],
    ),
];

impl ToolCache {
    /// Create the tool_cache table if it doesn't exist.
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tool_cache (
                name        TEXT PRIMARY KEY,
                definition  TEXT NOT NULL,
                embed_text  TEXT NOT NULL,
                embedding   BLOB,
                updated_at  REAL NOT NULL
            );",
        )
        .expect("failed to create tool_cache table");
    }

    /// Sync tool definitions into the cache.
    ///
    /// For each definition, checks if the cached version matches.
    /// If changed or new, re-embeds the description and upserts.
    /// Removes entries for tools no longer in the definitions list.
    pub fn sync(conn: &Connection, db: &YantrikDB, definitions: &[Value]) {
        let now = now_ts();
        let mut updated = 0usize;
        let mut names: Vec<String> = Vec::new();

        for def in definitions {
            let name = match def["function"]["name"].as_str() {
                Some(n) => n,
                None => continue,
            };
            names.push(name.to_string());

            let def_json = serde_json::to_string(def).unwrap_or_default();
            let embed_text = build_embed_text(def);

            // Check if cached version matches
            let existing: Option<String> = conn
                .query_row(
                    "SELECT definition FROM tool_cache WHERE name = ?1",
                    [name],
                    |row| row.get(0),
                )
                .ok();

            if existing.as_deref() == Some(&def_json) {
                continue; // No change
            }

            // Embed the tool description
            let embedding = match db.embed(&embed_text) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(tool = name, error = %e, "Failed to embed tool definition");
                    continue;
                }
            };
            let embedding_blob = embedding_to_blob(&embedding);

            conn.execute(
                "INSERT OR REPLACE INTO tool_cache (name, definition, embed_text, embedding, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![name, def_json, embed_text, embedding_blob, now],
            )
            .ok();

            updated += 1;
        }

        // Remove stale entries
        if !names.is_empty() {
            let placeholders: String = names
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "DELETE FROM tool_cache WHERE name NOT IN ({})",
                placeholders
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = names
                .iter()
                .map(|n| n as &dyn rusqlite::types::ToSql)
                .collect();
            let removed = conn.execute(&sql, params.as_slice()).unwrap_or(0);
            if removed > 0 {
                tracing::info!(removed, "Removed stale tool cache entries");
            }
        }

        tracing::info!(total = names.len(), updated, "Tool cache synced");
    }

    /// Select the most relevant tools for a user query using cosine similarity.
    ///
    /// Returns up to `limit` tool definitions (JSON schemas), always including
    /// core tools (remember, recall_memory).
    pub fn select_relevant(
        conn: &Connection,
        db: &YantrikDB,
        query: &str,
        limit: usize,
    ) -> Vec<Value> {
        // Embed the user query
        let query_embedding = match db.embed(query) {
            Ok(e) => e,
            Err(_) => {
                // Fallback: return all cached definitions
                return Self::all_definitions(conn);
            }
        };

        // Load all cached embeddings
        let mut stmt = match conn
            .prepare("SELECT name, definition, embedding FROM tool_cache")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let mut scored: Vec<(f32, String, String)> = Vec::new();
        let rows = stmt.query_map([], |row| {
            let name: String = row.get(0)?;
            let def: String = row.get(1)?;
            let emb_blob: Vec<u8> = row.get(2)?;
            Ok((name, def, emb_blob))
        });

        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (name, def, emb_blob) = row;
                let embedding = blob_to_embedding(&emb_blob);
                if embedding.is_empty() {
                    continue;
                }
                let sim = cosine_similarity(&query_embedding, &embedding);
                scored.push((sim, name, def));
            }
        }

        // Sort by similarity descending
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let mut selected: Vec<Value> = Vec::new();
        let mut selected_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        // First: add always-included core tools
        for &core_name in ALWAYS_INCLUDE {
            if let Some(pos) = scored.iter().position(|(_, n, _)| n == core_name) {
                let (_, name, def) = scored.remove(pos);
                if let Ok(v) = serde_json::from_str(&def) {
                    selected.push(v);
                    selected_names.insert(name);
                }
            }
        }

        // Keyword-based boosting: force-include category tools when query matches
        let lower_query = query.to_lowercase();
        for (keywords, tool_names) in KEYWORD_TOOLS {
            let matched = keywords.iter().any(|kw| lower_query.contains(kw));
            if matched {
                // Pull matched tools from scored list into selected
                for &tool_name in *tool_names {
                    if selected_names.contains(tool_name) {
                        continue;
                    }
                    if let Some(pos) = scored.iter().position(|(_, n, _)| n == tool_name) {
                        let (_, name, def) = scored.remove(pos);
                        if let Ok(v) = serde_json::from_str(&def) {
                            selected.push(v);
                            selected_names.insert(name);
                        }
                    }
                }
            }
        }

        // Then: add top-K by similarity
        for (sim, name, def) in &scored {
            if selected.len() >= limit {
                break;
            }
            if selected_names.contains(name) {
                continue;
            }
            if *sim < 0.20 {
                break; // Below relevance floor
            }
            if let Ok(v) = serde_json::from_str::<Value>(def) {
                selected.push(v);
                selected_names.insert(name.clone());
            }
        }

        tracing::debug!(
            selected = selected.len(),
            top_sim = scored.first().map(|s| s.0).unwrap_or(0.0),
            "Smart tool selection"
        );

        selected
    }

    /// Select tools by pure cosine similarity — no ALWAYS_INCLUDE, no keyword boosting.
    ///
    /// Returns up to `limit` tool definitions ranked by embedding similarity to the query.
    /// The caller is responsible for adding core/always-on tools.
    /// Floor threshold: 0.15 cosine similarity.
    pub fn select_by_similarity(
        conn: &Connection,
        db: &YantrikDB,
        query: &str,
        limit: usize,
    ) -> Vec<Value> {
        let query_embedding = match db.embed(query) {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };

        let mut stmt = match conn
            .prepare("SELECT name, definition, embedding FROM tool_cache")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let mut scored: Vec<(f32, String, String)> = Vec::new();
        let rows = stmt.query_map([], |row| {
            let name: String = row.get(0)?;
            let def: String = row.get(1)?;
            let emb_blob: Vec<u8> = row.get(2)?;
            Ok((name, def, emb_blob))
        });

        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (name, def, emb_blob) = row;
                let embedding = blob_to_embedding(&emb_blob);
                if embedding.is_empty() {
                    continue;
                }
                let sim = cosine_similarity(&query_embedding, &embedding);
                scored.push((sim, name, def));
            }
        }

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let mut results: Vec<Value> = Vec::new();
        for (sim, _name, def) in &scored {
            if results.len() >= limit {
                break;
            }
            if *sim < 0.15 {
                break;
            }
            if let Ok(v) = serde_json::from_str::<Value>(def) {
                results.push(v);
            }
        }

        tracing::debug!(
            returned = results.len(),
            top_sim = scored.first().map(|s| s.0).unwrap_or(0.0),
            "Pure similarity tool selection"
        );

        results
    }

    /// Select tools ranked by cosine similarity, returning scores and compact descriptions.
    ///
    /// Returns `(similarity, tool_name, compact_card)` tuples sorted by similarity descending.
    /// The compact_card is a short text suitable for MCQ prompts (~40 tokens per tool).
    /// Floor threshold: 0.15 cosine similarity.
    pub fn select_ranked_with_scores(
        conn: &Connection,
        db: &YantrikDB,
        query: &str,
        limit: usize,
    ) -> Vec<(f32, String, String)> {
        let query_embedding = match db.embed(query) {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };

        let mut stmt = match conn
            .prepare("SELECT name, definition, embedding FROM tool_cache")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let mut scored: Vec<(f32, String, String)> = Vec::new();
        let rows = stmt.query_map([], |row| {
            let name: String = row.get(0)?;
            let def: String = row.get(1)?;
            let emb_blob: Vec<u8> = row.get(2)?;
            Ok((name, def, emb_blob))
        });

        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (name, def, emb_blob) = row;
                let embedding = blob_to_embedding(&emb_blob);
                if embedding.is_empty() {
                    continue;
                }
                let sim = cosine_similarity(&query_embedding, &embedding);
                if sim < 0.15 {
                    continue;
                }
                // Build compact card from definition
                let card = if let Ok(v) = serde_json::from_str::<Value>(&def) {
                    build_compact_card(&v)
                } else {
                    name.clone()
                };
                scored.push((sim, name, card));
            }
        }

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        tracing::debug!(
            returned = scored.len(),
            top_sim = scored.first().map(|s| s.0).unwrap_or(0.0),
            "Ranked tool selection with scores"
        );

        scored
    }

    /// Fallback: return all cached definitions.
    fn all_definitions(conn: &Connection) -> Vec<Value> {
        let mut stmt = match conn.prepare("SELECT definition FROM tool_cache") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map([], |row| {
            let def: String = row.get(0)?;
            Ok(def)
        })
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .filter_map(|def| serde_json::from_str(&def).ok())
        .collect()
    }
}

/// Build natural language text from a tool JSON schema for embedding.
fn build_embed_text(def: &Value) -> String {
    let name = def["function"]["name"].as_str().unwrap_or("");
    let desc = def["function"]["description"].as_str().unwrap_or("");
    let params = &def["function"]["parameters"]["properties"];
    let mut text = format!("{}: {}", name, desc);
    if let Some(obj) = params.as_object() {
        for (key, val) in obj {
            if let Some(d) = val["description"].as_str() {
                text.push_str(&format!(". {} ({})", key, d));
            }
        }
    }
    text
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (ai, bi) in a.iter().zip(b.iter()) {
        dot += ai * bi;
        norm_a += ai * ai;
        norm_b += bi * bi;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < 1e-10 {
        0.0
    } else {
        dot / denom
    }
}

fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Build a compact tool card for MCQ prompts (~40 tokens).
///
/// Format: `tool_name — Short description. Use for: keyword1, keyword2, keyword3.`
fn build_compact_card(def: &Value) -> String {
    let name = def["function"]["name"].as_str().unwrap_or("unknown");
    let desc = def["function"]["description"].as_str().unwrap_or("");

    // Truncate description to first sentence or 80 chars
    let short_desc = if let Some(pos) = desc.find(". ") {
        &desc[..pos]
    } else if desc.len() > 80 {
        &desc[..desc.floor_char_boundary(80)]
    } else {
        desc
    };

    // Extract parameter names as keywords
    let params = &def["function"]["parameters"]["properties"];
    let keywords: Vec<&str> = if let Some(obj) = params.as_object() {
        obj.keys().map(|k| k.as_str()).take(4).collect()
    } else {
        Vec::new()
    };

    if keywords.is_empty() {
        format!("{} — {}", name, short_desc)
    } else {
        format!("{} — {}. Keywords: {}", name, short_desc, keywords.join(", "))
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
