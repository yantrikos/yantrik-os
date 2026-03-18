//! Open Loops tools — show, resolve, snooze life threads.
//!
//! Allows the LLM to inspect and manage the user's open loops
//! (commitments, unanswered messages, stalled tasks, etc.).

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};
use crate::world_model::{self, ThreadType, ThreadStatus};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(ShowOpenLoopsTool));
    reg.register(Box::new(ResolveLoopTool));
    reg.register(Box::new(SnoozeLoopTool));
}

// ── show_open_loops ──

pub struct ShowOpenLoopsTool;

impl Tool for ShowOpenLoopsTool {
    fn name(&self) -> &'static str { "show_open_loops" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "life_management" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "show_open_loops",
                "description": "Show the user's open loops — unresolved commitments",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "type_filter": {
                            "type": "string",
                            "description": "Filter by thread type (optional). Values: email, whatsapp, commitment, task, call, text, telegram, calendar"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum items to return (default 10)"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        let type_filter = args.get("type_filter").and_then(|v| v.as_str());
        let conn = ctx.db.conn();

        let threads = if let Some(filter) = type_filter {
            let tt = ThreadType::from_str(filter);
            world_model::query_threads_by_type(conn, &tt, limit)
                .into_iter()
                .filter(|t| !t.status.is_terminal())
                .collect::<Vec<_>>()
        } else {
            world_model::query_open_threads(conn, limit)
        };

        if threads.is_empty() {
            return "No open loops found. Everything is clear!".to_string();
        }

        let mut lines = Vec::new();
        lines.push(format!("## Open Loops ({})\n", threads.len()));

        for t in &threads {
            let status_icon = match t.status {
                ThreadStatus::Overdue => "🔴",
                ThreadStatus::Stalled => "🟡",
                ThreadStatus::Snoozed => "⏸️",
                _ => "🔵",
            };
            let age = if t.days_open > 0 {
                format!(" ({} days)", t.days_open)
            } else {
                String::new()
            };
            lines.push(format!(
                "{} **[{}]** {} [{}]{}\n  ID: `{}:{}`",
                status_icon,
                t.thread_type.label(),
                t.label,
                t.status.as_str(),
                age,
                t.thread_type.as_str(),
                t.entity_id,
            ));

            // Show context details if available
            if t.context_json != "{}" && !t.context_json.is_empty() {
                if let Ok(ctx_map) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&t.context_json) {
                    for (k, v) in ctx_map.iter().take(3) {
                        if let Some(s) = v.as_str() {
                            lines.push(format!("  {}: {}", k, s));
                        }
                    }
                }
            }
            lines.push(String::new());
        }

        // Channel summary
        let summary = world_model::attention_summary(conn);
        if !summary.is_empty() {
            lines.push("**Pending by channel:**".to_string());
            for (ch, count) in &summary {
                lines.push(format!("  {} — {}", ch, count));
            }
        }

        lines.join("\n")
    }
}

// ── resolve_loop ──

pub struct ResolveLoopTool;

impl Tool for ResolveLoopTool {
    fn name(&self) -> &'static str { "resolve_loop" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "life_management" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "resolve_loop",
                "description": "Mark an open loop as resolved",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "thread_type": {
                            "type": "string",
                            "description": "Type: email, whatsapp, commitment, task, call, text, telegram, calendar"
                        },
                        "entity_id": {
                            "type": "string",
                            "description": "The entity_id of the thread to resolve (e.g. 'commitment:42', 'email:123')"
                        },
                        "evidence": {
                            "type": "string",
                            "description": "Brief note on how it was resolved (e.g. 'replied to email', 'completed task')"
                        }
                    },
                    "required": ["thread_type", "entity_id"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let thread_type_str = args.get("thread_type").and_then(|v| v.as_str()).unwrap_or("");
        let entity_id = args.get("entity_id").and_then(|v| v.as_str()).unwrap_or("");
        let evidence = args.get("evidence").and_then(|v| v.as_str()).unwrap_or("resolved by user");

        if thread_type_str.is_empty() || entity_id.is_empty() {
            return "Error: thread_type and entity_id are required".to_string();
        }

        let tt = ThreadType::from_str(thread_type_str);
        let conn = ctx.db.conn();

        world_model::resolve_thread(conn, &tt, entity_id, evidence);

        format!("✅ Resolved: [{}] {} — {}", thread_type_str, entity_id, evidence)
    }
}

// ── snooze_loop ──

pub struct SnoozeLoopTool;

impl Tool for SnoozeLoopTool {
    fn name(&self) -> &'static str { "snooze_loop" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "life_management" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "snooze_loop",
                "description": "Snooze an open loop for a specified duration",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "thread_type": {
                            "type": "string",
                            "description": "Type: email, whatsapp, commitment, task, call, text, telegram, calendar"
                        },
                        "entity_id": {
                            "type": "string",
                            "description": "The entity_id of the thread to snooze"
                        },
                        "hours": {
                            "type": "number",
                            "description": "Hours to snooze (default 24). Use 1 for 'later today', 24 for 'tomorrow', 168 for 'next week'."
                        }
                    },
                    "required": ["thread_type", "entity_id"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let thread_type_str = args.get("thread_type").and_then(|v| v.as_str()).unwrap_or("");
        let entity_id = args.get("entity_id").and_then(|v| v.as_str()).unwrap_or("");
        let hours = args.get("hours").and_then(|v| v.as_f64()).unwrap_or(24.0);

        if thread_type_str.is_empty() || entity_id.is_empty() {
            return "Error: thread_type and entity_id are required".to_string();
        }

        let tt = ThreadType::from_str(thread_type_str);
        let conn = ctx.db.conn();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let until_ts = now + hours * 3600.0;

        world_model::snooze_thread(conn, &tt, entity_id, until_ts);

        let label = if hours < 2.0 {
            "later today".to_string()
        } else if hours < 25.0 {
            format!("{:.0} hours", hours)
        } else {
            format!("{:.0} days", hours / 24.0)
        };

        format!("⏸️ Snoozed: [{}] {} for {}", thread_type_str, entity_id, label)
    }
}
