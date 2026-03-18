//! Knowledge tools — cross-time memory search for fixes, timeframes, and work sessions.
//!
//! Tools: search_fix_history, search_by_timeframe, summarize_work_session.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(SearchFixHistoryTool));
    reg.register(Box::new(SearchByTimeframeTool));
    reg.register(Box::new(SummarizeWorkSessionTool));
}

// ── Search Fix History ──

pub struct SearchFixHistoryTool;

impl Tool for SearchFixHistoryTool {
    fn name(&self) -> &'static str { "search_fix_history" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "knowledge" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "search_fix_history",
                "description": "Search memory for past fixes, solutions, and workarounds",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "topic": {"type": "string", "description": "Topic to search fixes for"}
                    },
                    "required": ["topic"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let topic = args.get("topic").and_then(|v| v.as_str()).unwrap_or_default();
        if topic.is_empty() {
            return "Error: topic is required".to_string();
        }

        match ctx.db.recall_text(topic, 5) {
            Ok(results) => {
                if results.is_empty() {
                    format!("No past fixes found for '{topic}'.")
                } else {
                    let mut out = format!("Found {} memories related to '{topic}':\n", results.len());
                    for (i, r) in results.iter().enumerate() {
                        out.push_str(&format!(
                            "\n{}. {}\n   created: {} | importance: {:.2} | score: {:.2}\n",
                            i + 1, r.text, r.created_at, r.importance, r.score
                        ));
                    }
                    out
                }
            }
            Err(e) => format!("Search failed: {e}"),
        }
    }
}

// ── Search by Timeframe ──

pub struct SearchByTimeframeTool;

impl Tool for SearchByTimeframeTool {
    fn name(&self) -> &'static str { "search_by_timeframe" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "knowledge" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "search_by_timeframe",
                "description": "Search memories from a date or time range",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Search query"},
                        "days_ago": {"type": "integer", "description": "How many days back to search"}
                    },
                    "required": ["query", "days_ago"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
        let days_ago = args.get("days_ago").and_then(|v| v.as_u64()).unwrap_or(7);

        if query.is_empty() {
            return "Error: query is required".to_string();
        }

        match ctx.db.recall_text(query, 10) {
            Ok(results) => {
                // created_at is f64 Unix timestamp — compare directly.
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64();
                let cutoff = now - (days_ago as f64 * 86400.0);

                let filtered: Vec<_> = results
                    .iter()
                    .filter(|r| r.created_at >= cutoff)
                    .collect();

                if filtered.is_empty() {
                    format!("No memories matching '{query}' in the last {} days.", days_ago)
                } else {
                    let mut out = format!(
                        "Found {} memories matching '{query}' (last {} days):\n",
                        filtered.len(), days_ago
                    );
                    for (i, r) in filtered.iter().enumerate() {
                        out.push_str(&format!(
                            "\n{}. {}\n   created: {} | importance: {:.2}\n",
                            i + 1, r.text, r.created_at, r.importance
                        ));
                    }
                    out
                }
            }
            Err(e) => format!("Search failed: {e}"),
        }
    }
}

// ── Summarize Work Session ──

pub struct SummarizeWorkSessionTool;

impl Tool for SummarizeWorkSessionTool {
    fn name(&self) -> &'static str { "summarize_work_session" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "knowledge" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "summarize_work_session",
                "description": "Summarize what you were working on in the last N hours",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "hours": {"type": "integer", "description": "Hours to look back (default: 4)"}
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let hours = args.get("hours").and_then(|v| v.as_u64()).unwrap_or(4);

        match ctx.db.recall_text("work session activity", 20) {
            Ok(results) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64();
                let cutoff = now - (hours as f64 * 3600.0);

                let filtered: Vec<_> = results
                    .iter()
                    .filter(|r| r.created_at >= cutoff && r.domain.contains("work"))
                    .collect();

                if filtered.is_empty() {
                    format!("No work activity found in the last {} hours.", hours)
                } else {
                    let mut out = format!(
                        "Work session summary (last {} hours, {} items):\n",
                        hours, filtered.len()
                    );
                    for (i, r) in filtered.iter().enumerate() {
                        out.push_str(&format!(
                            "\n{}. {}\n   at: {} | domain: {}\n",
                            i + 1, r.text, r.created_at, r.domain
                        ));
                    }
                    out
                }
            }
            Err(e) => format!("Summarize failed: {e}"),
        }
    }
}

