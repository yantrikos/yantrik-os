//! Task queue tools — LLM can create, list, update, and complete persistent tasks.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};
use crate::task_queue::TaskQueue;

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(QueueTaskTool));
    reg.register(Box::new(ListTasksTool));
    reg.register(Box::new(UpdateTaskTool));
    reg.register(Box::new(CompleteTaskTool));
    reg.register(Box::new(CancelTaskTool));
}

// ── queue_task ──

struct QueueTaskTool;

impl Tool for QueueTaskTool {
    fn name(&self) -> &'static str { "queue_task" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "tasks" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "queue_task",
                "description": "Add a task to your persistent work queue",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "Short task title (e.g., 'Evaluate all browser tools')"
                        },
                        "description": {
                            "type": "string",
                            "description": "Detailed instructions for what to do. Be specific about tools to use, files to create, and how to report results."
                        },
                        "priority": {
                            "type": "integer",
                            "description": "1=low, 2=normal (default), 3=high, 4=urgent"
                        }
                    },
                    "required": ["title", "description"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let title = match args.get("title").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: title".to_string(),
        };
        let description = args.get("description").and_then(|v| v.as_str()).unwrap_or("");
        let priority = args.get("priority").and_then(|v| v.as_i64()).unwrap_or(2) as i32;

        match TaskQueue::enqueue(ctx.db.conn(), title, description, priority, "user") {
            Ok(id) => format!("Task queued: [{}] {} (priority {}). Will work on it during idle time.", id, title, priority),
            Err(e) => format!("Failed to queue task: {}", e),
        }
    }
}

// ── list_tasks ──

struct ListTasksTool;

impl Tool for ListTasksTool {
    fn name(&self) -> &'static str { "list_tasks" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "tasks" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_tasks",
                "description": "List tasks in your work queue",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "status": {
                            "type": "string",
                            "description": "Filter by status: 'pending', 'in_progress', 'completed', 'failed', or omit for all."
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let status = args.get("status").and_then(|v| v.as_str());
        let tasks = TaskQueue::list(ctx.db.conn(), status, 15);

        if tasks.is_empty() {
            return match status {
                Some(s) => format!("No {} tasks.", s),
                None => "Task queue is empty.".to_string(),
            };
        }

        let mut result = format!("Tasks ({}):\n\n", tasks.len());
        for t in &tasks {
            let icon = match t.status {
                crate::task_queue::TaskStatus::Pending => "○",
                crate::task_queue::TaskStatus::InProgress => "▶",
                crate::task_queue::TaskStatus::Completed => "✓",
                crate::task_queue::TaskStatus::Failed => "✗",
                crate::task_queue::TaskStatus::Cancelled => "—",
            };
            result.push_str(&format!("{} [{}] {} (P{}) — {}\n", icon, t.task_id, t.title, t.priority, t.status.as_str()));
            if !t.progress.is_empty() {
                let short = if t.progress.len() > 120 {
                    format!("{}...", &t.progress[..t.progress.floor_char_boundary(117)])
                } else {
                    t.progress.clone()
                };
                result.push_str(&format!("  Progress: {}\n", short));
            }
        }
        result
    }
}

// ── update_task ──

struct UpdateTaskTool;

impl Tool for UpdateTaskTool {
    fn name(&self) -> &'static str { "update_task" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "tasks" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "update_task",
                "description": "Update progress on a queued task",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "task_id": {
                            "type": "string",
                            "description": "The task ID to update."
                        },
                        "progress": {
                            "type": "string",
                            "description": "Summary of what's been done so far."
                        },
                        "steps_completed": {
                            "type": "integer",
                            "description": "Number of steps completed."
                        }
                    },
                    "required": ["task_id", "progress"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let task_id = match args.get("task_id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: task_id".to_string(),
        };
        let progress = args.get("progress").and_then(|v| v.as_str()).unwrap_or("");
        let steps = args.get("steps_completed").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

        TaskQueue::update_progress(ctx.db.conn(), task_id, progress, steps);
        format!("Task {} updated. Steps: {}. Progress: {}", task_id, steps, progress)
    }
}

// ── complete_task ──

struct CompleteTaskTool;

impl Tool for CompleteTaskTool {
    fn name(&self) -> &'static str { "complete_task" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "tasks" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "complete_task",
                "description": "Mark a queued task as completed",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "task_id": {
                            "type": "string",
                            "description": "The task ID to complete."
                        },
                        "result": {
                            "type": "string",
                            "description": "Summary of what was accomplished."
                        }
                    },
                    "required": ["task_id", "result"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let task_id = match args.get("task_id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: task_id".to_string(),
        };
        let result = args.get("result").and_then(|v| v.as_str()).unwrap_or("Completed.");

        TaskQueue::complete(ctx.db.conn(), task_id, result);
        format!("Task {} marked as completed.", task_id)
    }
}

// ── cancel_task ──

struct CancelTaskTool;

impl Tool for CancelTaskTool {
    fn name(&self) -> &'static str { "cancel_task" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "tasks" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "cancel_task",
                "description": "Cancel a pending or in-progress task",
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
        let task_id = match args.get("task_id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: task_id".to_string(),
        };

        TaskQueue::cancel(ctx.db.conn(), task_id);
        format!("Task {} cancelled.", task_id)
    }
}
