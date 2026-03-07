//! Tool chain learning — stores successful execution traces for retrieval.
//!
//! When a query is handled with tool calls, the sequence (goal → tools → outcome)
//! is recorded with an embedding of the goal text. On future similar queries,
//! these traces are retrieved as hints so the LLM can follow proven patterns.

use rusqlite::{params, Connection};
use yantrikdb_core::YantrikDB;

/// A stored trace hint retrieved by similarity search.
#[derive(Debug, Clone)]
pub struct TraceHint {
    pub trace_id: String,
    pub goal: String,
    pub tool_chain: Vec<String>,
    pub step_count: usize,
    pub similarity: f32,
}

/// Manages tool execution trace storage and retrieval.
pub struct ToolTraces;

impl ToolTraces {
    /// Create the tool_traces table if it doesn't exist.
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tool_traces (
                trace_id    TEXT PRIMARY KEY,
                goal_text   TEXT NOT NULL,
                goal_embed  BLOB,
                tool_chain  TEXT NOT NULL,
                outcome     TEXT NOT NULL,
                step_count  INTEGER NOT NULL,
                created_at  REAL NOT NULL,
                used_count  INTEGER DEFAULT 0,
                last_used   REAL
            );
            CREATE INDEX IF NOT EXISTS idx_tool_traces_outcome ON tool_traces(outcome);",
        )
        .expect("failed to create tool_traces table");
    }

    /// Record a completed tool execution trace.
    pub fn record(
        conn: &Connection,
        db: &YantrikDB,
        goal: &str,
        chain_summary: &[serde_json::Value],
        outcome: &str,
    ) {
        let trace_id = format!("trace_{}", now_ts() as u64);

        // Extract just tool names for the chain
        let chain_json = serde_json::to_string(chain_summary).unwrap_or_default();

        // Embed the goal for similarity search
        let embed_blob: Option<Vec<u8>> = db.embed(goal).ok().map(|e| {
            e.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<u8>>()
        });

        conn.execute(
            "INSERT OR REPLACE INTO tool_traces
                (trace_id, goal_text, goal_embed, tool_chain, outcome, step_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                trace_id,
                goal,
                embed_blob,
                chain_json,
                outcome,
                chain_summary.len(),
                now_ts(),
            ],
        )
        .ok();

        tracing::debug!(
            trace_id,
            goal,
            steps = chain_summary.len(),
            outcome,
            "Recorded tool trace"
        );
    }

    /// Find similar successful traces for a query.
    pub fn find_similar(
        conn: &Connection,
        db: &YantrikDB,
        query: &str,
        top_k: usize,
        min_sim: f32,
    ) -> Vec<TraceHint> {
        // Embed the query
        let query_embed = match db.embed(query) {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };

        // Load all successful traces with embeddings
        let mut stmt = match conn.prepare(
            "SELECT trace_id, goal_text, goal_embed, tool_chain, step_count
             FROM tool_traces
             WHERE outcome = 'success' AND goal_embed IS NOT NULL
             ORDER BY created_at DESC
             LIMIT 100",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let mut hints: Vec<TraceHint> = Vec::new();

        let rows = stmt
            .query_map([], |row| {
                let trace_id: String = row.get(0)?;
                let goal: String = row.get(1)?;
                let embed_blob: Vec<u8> = row.get(2)?;
                let chain_json: String = row.get(3)?;
                let step_count: i64 = row.get(4)?;
                Ok((trace_id, goal, embed_blob, chain_json, step_count as usize))
            })
            .ok();

        if let Some(rows) = rows {
            for row in rows.flatten() {
                let (trace_id, goal, embed_blob, chain_json, step_count) = row;

                // Decode embedding
                if embed_blob.len() != query_embed.len() * 4 {
                    continue;
                }
                let stored_embed: Vec<f32> = embed_blob
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();

                let sim = cosine_similarity(&query_embed, &stored_embed);
                if sim >= min_sim {
                    // Parse tool chain to get just tool names
                    let tool_chain: Vec<String> = serde_json::from_str::<Vec<serde_json::Value>>(&chain_json)
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|v| v.get("tool").and_then(|t| t.as_str()).map(String::from))
                        .collect();

                    hints.push(TraceHint {
                        trace_id,
                        goal,
                        tool_chain,
                        step_count,
                        similarity: sim,
                    });
                }
            }
        }

        // Sort by similarity descending, take top-K
        hints.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));
        hints.truncate(top_k);

        hints
    }

    /// Mark a trace as used (updates used_count and last_used).
    pub fn mark_used(conn: &Connection, trace_id: &str) {
        conn.execute(
            "UPDATE tool_traces SET used_count = used_count + 1, last_used = ?1 WHERE trace_id = ?2",
            params![now_ts(), trace_id],
        )
        .ok();
    }

    /// Format trace hints as a system prompt section.
    pub fn format_hints(hints: &[TraceHint]) -> String {
        if hints.is_empty() {
            return String::new();
        }

        let mut result = String::from("Similar tasks you solved before:\n");
        for hint in hints.iter().take(3) {
            let chain = hint.tool_chain.join(" → ");
            result.push_str(&format!("- \"{}\" → {}\n", hint.goal, chain));
        }
        result.push_str("Use these as a guide if relevant.\n\n");
        result
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < 1e-10 {
        0.0
    } else {
        dot / denom
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
