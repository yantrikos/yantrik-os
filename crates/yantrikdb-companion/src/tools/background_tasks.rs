//! Background task tools — run_background, list_background_tasks,
//! check_background_task, stop_background_task.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};
use crate::sanitize;

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(RunBackgroundTool));
    reg.register(Box::new(ListBackgroundTasksTool));
    reg.register(Box::new(CheckBackgroundTaskTool));
    reg.register(Box::new(StopBackgroundTaskTool));
}

// ── Run Background ──

pub struct RunBackgroundTool;

impl Tool for RunBackgroundTool {
    fn name(&self) -> &'static str { "run_background" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "background_tasks" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "run_background",
                "description": "Run a command in the background. Returns a task ID for tracking. Use for long-running operations like downloads, builds, or data processing.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "The shell command to run in the background"
                        },
                        "label": {
                            "type": "string",
                            "description": "Short human-readable description of the task"
                        }
                    },
                    "required": ["command", "label"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return "Missing required parameter: command".to_string(),
        };
        let label = args
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("background task");

        // Validate command safety
        if let Some(reason) = sanitize::detect_harmful_command(command) {
            return format!("Command blocked: {reason}");
        }

        let tm = match &ctx.task_manager {
            Some(tm) => tm,
            None => return "Background tasks not available".to_string(),
        };

        let mut tm = match tm.lock() {
            Ok(t) => t,
            Err(_) => return "Task manager unavailable".to_string(),
        };

        match tm.spawn(ctx.db.conn(), command, label) {
            Ok(task_id) => format!("Task started: {task_id} — \"{label}\"\nUse check_background_task to monitor progress."),
            Err(e) => format!("Failed to start task: {e}"),
        }
    }
}

// ── List Background Tasks ──

pub struct ListBackgroundTasksTool;

impl Tool for ListBackgroundTasksTool {
    fn name(&self) -> &'static str { "list_background_tasks" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "background_tasks" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_background_tasks",
                "description": "List background tasks with their status.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "status": {
                            "type": "string",
                            "enum": ["running", "completed", "failed", "stopped"],
                            "description": "Filter by status (omit for all tasks)"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let status_filter = args.get("status").and_then(|v| v.as_str());

        let tm = match &ctx.task_manager {
            Some(tm) => tm,
            None => return "Background tasks not available".to_string(),
        };

        let tm = match tm.lock() {
            Ok(t) => t,
            Err(_) => return "Task manager unavailable".to_string(),
        };

        let tasks = tm.list(ctx.db.conn(), status_filter);
        if tasks.is_empty() {
            return match status_filter {
                Some(s) => format!("No {s} tasks."),
                None => "No background tasks.".to_string(),
            };
        }

        let mut lines = vec!["ID | Label | Status | Command".to_string()];
        lines.push("---|-------|--------|--------".to_string());
        for t in &tasks {
            let cmd_short = if t.command.len() > 40 {
                format!("{}...", &t.command[..40])
            } else {
                t.command.clone()
            };
            lines.push(format!(
                "{} | {} | {} | {}",
                t.task_id, t.label, t.status, cmd_short
            ));
        }
        lines.join("\n")
    }
}

// ── Check Background Task ──

pub struct CheckBackgroundTaskTool;

impl Tool for CheckBackgroundTaskTool {
    fn name(&self) -> &'static str { "check_background_task" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "background_tasks" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "check_background_task",
                "description": "Check the status and output of a background task.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "task_id": {
                            "type": "string",
                            "description": "The task ID to check"
                        },
                        "lines": {
                            "type": "integer",
                            "description": "Number of output lines to show (default: 20)"
                        }
                    },
                    "required": ["task_id"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let task_id = match args.get("task_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return "Missing required parameter: task_id".to_string(),
        };
        let tail_lines = args
            .get("lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;

        let tm = match &ctx.task_manager {
            Some(tm) => tm,
            None => return "Background tasks not available".to_string(),
        };

        let mut tm = match tm.lock() {
            Ok(t) => t,
            Err(_) => return "Task manager unavailable".to_string(),
        };

        // Poll first to get fresh status
        tm.poll(ctx.db.conn());

        match tm.get_status(ctx.db.conn(), task_id) {
            Some(info) => {
                let mut result = format!(
                    "Task: {} ({})\nStatus: {}\nCommand: {}",
                    info.task_id, info.label, info.status, info.command
                );
                if let Some(code) = info.exit_code {
                    result.push_str(&format!("\nExit code: {}", code));
                }
                let output = crate::task_manager::TaskManager::read_output(task_id, tail_lines);
                if !output.is_empty() {
                    result.push_str(&format!("\n\nOutput (last {} lines):\n{}", tail_lines, output));
                } else {
                    result.push_str("\n\nNo output yet.");
                }
                result
            }
            None => format!("Unknown task: {task_id}"),
        }
    }
}

// ── Stop Background Task ──

pub struct StopBackgroundTaskTool;

impl Tool for StopBackgroundTaskTool {
    fn name(&self) -> &'static str { "stop_background_task" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "background_tasks" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "stop_background_task",
                "description": "Stop a running background task.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "task_id": {
                            "type": "string",
                            "description": "The task ID to stop"
                        }
                    },
                    "required": ["task_id"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let task_id = match args.get("task_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return "Missing required parameter: task_id".to_string(),
        };

        let tm = match &ctx.task_manager {
            Some(tm) => tm,
            None => return "Background tasks not available".to_string(),
        };

        let mut tm = match tm.lock() {
            Ok(t) => t,
            Err(_) => return "Task manager unavailable".to_string(),
        };

        match tm.stop(ctx.db.conn(), task_id) {
            Ok(msg) => msg,
            Err(e) => e,
        }
    }
}
