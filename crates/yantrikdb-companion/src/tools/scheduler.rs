//! Scheduler tools — create, list, update, cancel scheduled tasks.

use super::{PermissionLevel, Tool, ToolContext, ToolRegistry};
use crate::scheduler::Scheduler;

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(CreateScheduleTool));
    reg.register(Box::new(ListSchedulesTool));
    reg.register(Box::new(UpdateScheduleTool));
    reg.register(Box::new(CancelScheduleTool));
}

// ── Create Schedule ──

struct CreateScheduleTool;

impl Tool for CreateScheduleTool {
    fn name(&self) -> &'static str { "create_schedule" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "scheduler" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "create_schedule",
                "description": "Create a scheduled task. Supports one-shot (once), recurring interval, or cron expressions. Use for reminders, daily routines, birthday/anniversary alerts, periodic checks.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "label": {
                            "type": "string",
                            "description": "Short name for the task (e.g. 'Daily standup', 'Mom birthday')"
                        },
                        "schedule_type": {
                            "type": "string",
                            "enum": ["once", "interval", "cron"],
                            "description": "once: fires once at 'at' time. interval: every N seconds. cron: 5-field cron (minute hour day month weekday)."
                        },
                        "at": {
                            "type": "string",
                            "description": "For 'once': ISO datetime YYYY-MM-DDTHH:MM (UTC). Ignored for interval/cron."
                        },
                        "interval_seconds": {
                            "type": "integer",
                            "description": "For 'interval': seconds between fires (e.g. 3600 = hourly)."
                        },
                        "cron": {
                            "type": "string",
                            "description": "For 'cron': 5-field expression (minute hour day month weekday). Examples: '0 9 * * *' = daily 9am, '0 9 15 3 *' = March 15 at 9am."
                        },
                        "description": {
                            "type": "string",
                            "description": "Longer description shown when the task fires."
                        },
                        "urgency": {
                            "type": "number",
                            "description": "0.0-1.0 how urgent the notification is (default 0.6)."
                        },
                        "max_invocations": {
                            "type": "integer",
                            "description": "Stop after N fires (null = unlimited for interval/cron)."
                        },
                        "action": {
                            "type": "string",
                            "description": "Natural language instruction to auto-execute when this fires. The companion will execute this autonomously using tools. E.g. 'Check the weather and send a notification'. Leave empty for reminder-only schedules."
                        }
                    },
                    "required": ["label", "schedule_type"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let label = args.get("label").and_then(|v| v.as_str()).unwrap_or_default();
        let schedule_type = args.get("schedule_type").and_then(|v| v.as_str()).unwrap_or("once");

        if label.is_empty() {
            return "Error: label is required".to_string();
        }
        if !["once", "interval", "cron"].contains(&schedule_type) {
            return format!("Error: schedule_type must be once, interval, or cron (got '{schedule_type}')");
        }

        let description = args.get("description").and_then(|v| v.as_str()).unwrap_or("");
        let urgency = args.get("urgency").and_then(|v| v.as_f64()).unwrap_or(0.6).clamp(0.0, 1.0);
        let max_invocations = args.get("max_invocations").and_then(|v| v.as_i64());

        let now = now_ts();

        // Compute next_invoke and extract schedule params
        let (next_invoke, interval_secs, cron_expr) = match schedule_type {
            "once" => {
                let at_str = args.get("at").and_then(|v| v.as_str()).unwrap_or_default();
                if at_str.is_empty() {
                    return "Error: 'at' is required for schedule_type 'once' (YYYY-MM-DDTHH:MM)".to_string();
                }
                match chrono::NaiveDateTime::parse_from_str(at_str, "%Y-%m-%dT%H:%M") {
                    Ok(dt) => {
                        let ts = dt.and_utc().timestamp() as f64;
                        if ts <= now {
                            return format!("Error: 'at' time {} is in the past", at_str);
                        }
                        (ts, None, None)
                    }
                    Err(_) => {
                        return format!("Error: invalid 'at' format '{}', use YYYY-MM-DDTHH:MM", at_str);
                    }
                }
            }
            "interval" => {
                let secs = args.get("interval_seconds").and_then(|v| v.as_i64()).unwrap_or(3600);
                if secs < 60 {
                    return "Error: interval_seconds must be at least 60".to_string();
                }
                (now + secs as f64, Some(secs), None)
            }
            "cron" => {
                let expr = args.get("cron").and_then(|v| v.as_str()).unwrap_or_default();
                if expr.is_empty() {
                    return "Error: 'cron' expression is required for schedule_type 'cron'".to_string();
                }
                match crate::cron_mini::next_cron(expr, now) {
                    Some(next) => (next, None, Some(expr)),
                    None => {
                        return format!("Error: invalid cron expression '{}'. Use 5 fields: minute hour day month weekday", expr);
                    }
                }
            }
            _ => unreachable!(),
        };

        let action = args.get("action").and_then(|v| v.as_str());

        let task_id = Scheduler::create(
            ctx.db.conn(),
            label,
            description,
            schedule_type,
            interval_secs,
            cron_expr,
            next_invoke,
            max_invocations,
            urgency,
            action,
            &serde_json::json!({}),
        );

        let next_str = format_ts(next_invoke);
        format!("Scheduled '{}' ({}). Next fire: {}. ID: {}", label, schedule_type, next_str, task_id)
    }
}

// ── List Schedules ──

struct ListSchedulesTool;

impl Tool for ListSchedulesTool {
    fn name(&self) -> &'static str { "list_schedules" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "scheduler" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_schedules",
                "description": "List scheduled tasks. Shows label, type, next fire time, and status.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "status": {
                            "type": "string",
                            "enum": ["active", "paused", "completed", "cancelled"],
                            "description": "Filter by status (default: active)."
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let status = args.get("status").and_then(|v| v.as_str()).unwrap_or("active");
        let tasks = Scheduler::list(ctx.db.conn(), Some(status));

        if tasks.is_empty() {
            return format!("No {} scheduled tasks.", status);
        }

        let mut lines = vec![format!("{} scheduled tasks ({}):", tasks.len(), status)];
        for task in &tasks {
            let next_str = task
                .next_invoke
                .map(|ts| format_ts(ts))
                .unwrap_or_else(|| "\u{2014}".to_string());
            let desc_part = if task.description.is_empty() {
                String::new()
            } else {
                format!(" \u{2014} {}", task.description)
            };
            lines.push(format!(
                "  - {} ({}){} | next: {} | fires: {} [{}]",
                task.label, task.schedule_type, desc_part, next_str,
                task.invocation_count, task.task_id,
            ));
        }
        lines.join("\n")
    }
}

// ── Update Schedule ──

struct UpdateScheduleTool;

impl Tool for UpdateScheduleTool {
    fn name(&self) -> &'static str { "update_schedule" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "scheduler" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "update_schedule",
                "description": "Update a scheduled task. Use to reschedule (change next_invoke), rename, adjust urgency, or change cron/interval. Useful after a birthday fires — set next_invoke to next year.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "task_id": {
                            "type": "string",
                            "description": "The task ID to update."
                        },
                        "label": { "type": "string" },
                        "description": { "type": "string" },
                        "next_invoke": {
                            "type": "string",
                            "description": "New next fire time as ISO datetime YYYY-MM-DDTHH:MM (UTC)."
                        },
                        "urgency": { "type": "number" },
                        "status": {
                            "type": "string",
                            "enum": ["active", "paused"]
                        },
                        "interval_seconds": { "type": "integer" },
                        "cron": { "type": "string" }
                    },
                    "required": ["task_id"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or_default();
        if task_id.is_empty() {
            return "Error: task_id is required".to_string();
        }

        // Check task exists
        if Scheduler::get(ctx.db.conn(), task_id).is_none() {
            return format!("Error: no task found with ID '{}'", task_id);
        }

        // Build updates object — convert next_invoke from ISO string to timestamp
        let mut updates = args.clone();
        if let Some(next_str) = args.get("next_invoke").and_then(|v| v.as_str()) {
            match chrono::NaiveDateTime::parse_from_str(next_str, "%Y-%m-%dT%H:%M") {
                Ok(dt) => {
                    let ts = dt.and_utc().timestamp() as f64;
                    updates["next_invoke"] = serde_json::json!(ts);
                }
                Err(_) => {
                    return format!("Error: invalid next_invoke format '{}', use YYYY-MM-DDTHH:MM", next_str);
                }
            }
        }

        Scheduler::update(ctx.db.conn(), task_id, &updates);
        format!("Schedule '{}' updated.", task_id)
    }
}

// ── Cancel Schedule ──

struct CancelScheduleTool;

impl Tool for CancelScheduleTool {
    fn name(&self) -> &'static str { "cancel_schedule" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "scheduler" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "cancel_schedule",
                "description": "Cancel a scheduled task by its ID.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "task_id": {
                            "type": "string",
                            "description": "The task ID to cancel."
                        }
                    },
                    "required": ["task_id"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let task_id = args.get("task_id").and_then(|v| v.as_str()).unwrap_or_default();
        if task_id.is_empty() {
            return "Error: task_id is required".to_string();
        }

        if Scheduler::cancel(ctx.db.conn(), task_id) {
            format!("Schedule '{}' cancelled.", task_id)
        } else {
            format!("No active/paused task found with ID '{}'", task_id)
        }
    }
}

// ── Helpers ──

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Format a unix timestamp as ISO-like string (UTC).
fn format_ts(ts: f64) -> String {
    let secs = ts as i64;
    let days = secs.div_euclid(86400);
    let time_of_day = secs.rem_euclid(86400);
    let hour = time_of_day / 3600;
    let minute = (time_of_day % 3600) / 60;

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02}T{:02}:{:02}Z", y, m, d, hour, minute)
}
