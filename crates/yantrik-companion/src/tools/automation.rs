//! Automation tools — create, list, run, delete automations + workflow recording.

use super::{PermissionLevel, Tool, ToolContext, ToolRegistry};
use crate::automation::AutomationStore;
use crate::scheduler::Scheduler;

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(CreateAutomationTool));
    reg.register(Box::new(ListAutomationsTool));
    reg.register(Box::new(RunAutomationTool));
    reg.register(Box::new(DeleteAutomationTool));
    reg.register(Box::new(ToggleAutomationTool));
}

// ── Create Automation ──

struct CreateAutomationTool;

impl Tool for CreateAutomationTool {
    fn name(&self) -> &'static str { "create_automation" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "automation" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "create_automation",
                "description": "Create an automation rule",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Short name (e.g., 'Morning weather', 'Deploy workflow')"
                        },
                        "description": {
                            "type": "string",
                            "description": "What this automation does"
                        },
                        "trigger_type": {
                            "type": "string",
                            "enum": ["manual", "schedule", "event"],
                            "description": "manual: run on demand. schedule: time-based. event: on system event."
                        },
                        "schedule_cron": {
                            "type": "string",
                            "description": "For schedule triggers: 5-field cron (minute hour day month weekday). E.g. '0 9 * * 1' = every Monday 9am."
                        },
                        "schedule_interval_seconds": {
                            "type": "integer",
                            "description": "For schedule triggers: interval in seconds (min 60)."
                        },
                        "event_type": {
                            "type": "string",
                            "description": "For event triggers: SystemEvent type (NetworkChanged, BatteryChanged, ProcessStarted, ProcessStopped, FileChanged, NotificationReceived, UserIdle, UserResumed)."
                        },
                        "event_match": {
                            "type": "object",
                            "description": "For event triggers: field-level match filter. E.g. {\"connected\": true, \"ssid\": \"HomeWifi\"}"
                        },
                        "condition": {
                            "type": "string",
                            "description": "Optional natural language condition evaluated before running. E.g. 'Only if battery is above 50%'."
                        },
                        "steps": {
                            "type": "string",
                            "description": "Natural language instructions for what to do. You'll execute these using tools when the automation fires. E.g. 'Check the weather for my location and send me a notification with the forecast.'"
                        }
                    },
                    "required": ["name", "trigger_type", "steps"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        let trigger_type = args.get("trigger_type").and_then(|v| v.as_str()).unwrap_or("manual");
        let steps = args.get("steps").and_then(|v| v.as_str()).unwrap_or_default();

        if name.is_empty() {
            return "Error: name is required".to_string();
        }
        if steps.is_empty() {
            return "Error: steps are required".to_string();
        }
        if !["manual", "schedule", "event"].contains(&trigger_type) {
            return format!("Error: trigger_type must be manual, schedule, or event (got '{trigger_type}')");
        }

        let description = args.get("description").and_then(|v| v.as_str()).unwrap_or("");
        let condition = args.get("condition").and_then(|v| v.as_str());

        // Build trigger_config based on trigger_type
        let trigger_config = match trigger_type {
            "schedule" => {
                let cron = args.get("schedule_cron").and_then(|v| v.as_str());
                let interval = args.get("schedule_interval_seconds").and_then(|v| v.as_i64());

                if cron.is_none() && interval.is_none() {
                    return "Error: schedule triggers need schedule_cron or schedule_interval_seconds".to_string();
                }

                // Create a linked scheduled_task
                let now = now_ts();
                let (schedule_type, next_invoke, interval_secs, cron_expr) = if let Some(expr) = cron {
                    match crate::cron_mini::next_cron(expr, now) {
                        Some(next) => ("cron", next, None, Some(expr)),
                        None => return format!("Error: invalid cron expression '{}'", expr),
                    }
                } else {
                    let secs = interval.unwrap();
                    if secs < 60 {
                        return "Error: interval must be at least 60 seconds".to_string();
                    }
                    ("interval", now + secs as f64, Some(secs), None)
                };

                // Create the scheduled task — action will point to this automation
                let task_id = Scheduler::create(
                    ctx.db.conn(),
                    name,
                    description,
                    schedule_type,
                    interval_secs,
                    cron_expr,
                    next_invoke,
                    None, // unlimited
                    0.8,  // high urgency for automations
                    None, // action set below after we have automation_id
                    &serde_json::json!({}),
                );

                serde_json::json!({
                    "schedule_id": task_id,
                    "schedule_type": schedule_type,
                })
            }
            "event" => {
                let event_type = args.get("event_type").and_then(|v| v.as_str());
                if event_type.is_none() {
                    return "Error: event triggers need event_type".to_string();
                }
                let event_match = args.get("event_match").cloned().unwrap_or(serde_json::json!({}));
                serde_json::json!({
                    "event_type": event_type,
                    "match": event_match,
                })
            }
            _ => serde_json::json!({}),
        };

        let automation_id = AutomationStore::create(
            ctx.db.conn(),
            name,
            description,
            trigger_type,
            &trigger_config,
            condition,
            steps,
        );

        // For schedule triggers: update the scheduled_task action to point to this automation
        if trigger_type == "schedule" {
            if let Some(schedule_id) = trigger_config.get("schedule_id").and_then(|v| v.as_str()) {
                let action = format!("automation:{}", automation_id);
                Scheduler::update(
                    ctx.db.conn(),
                    schedule_id,
                    &serde_json::json!({"action": action}),
                );
            }
        }

        let trigger_desc = match trigger_type {
            "schedule" => {
                let cron = args.get("schedule_cron").and_then(|v| v.as_str());
                if let Some(expr) = cron {
                    format!("cron: {}", expr)
                } else {
                    let secs = args.get("schedule_interval_seconds").and_then(|v| v.as_i64()).unwrap_or(0);
                    format!("every {}s", secs)
                }
            }
            "event" => {
                let et = args.get("event_type").and_then(|v| v.as_str()).unwrap_or("?");
                format!("on {}", et)
            }
            _ => "manual".to_string(),
        };

        format!(
            "Automation '{}' created ({trigger_desc}). ID: {automation_id}",
            name
        )
    }
}

// ── List Automations ──

struct ListAutomationsTool;

impl Tool for ListAutomationsTool {
    fn name(&self) -> &'static str { "list_automations" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "automation" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_automations",
                "description": "List automation rules",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "trigger_type": {
                            "type": "string",
                            "enum": ["manual", "schedule", "event"],
                            "description": "Filter by trigger type."
                        },
                        "status": {
                            "type": "string",
                            "enum": ["active", "archived"],
                            "description": "Filter by status (default: active)."
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let trigger_type = args.get("trigger_type").and_then(|v| v.as_str());
        let status = args.get("status").and_then(|v| v.as_str());

        let automations = AutomationStore::list(ctx.db.conn(), trigger_type, status);

        if automations.is_empty() {
            return "No automations found.".to_string();
        }

        let mut lines = vec![format!("{} automations:", automations.len())];
        for a in &automations {
            let trigger = match a.trigger_type.as_str() {
                "schedule" => {
                    let st = a.trigger_config.get("schedule_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("scheduled");
                    st.to_string()
                }
                "event" => {
                    let et = a.trigger_config.get("event_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    format!("on {}", et)
                }
                _ => "manual".to_string(),
            };
            let enabled = if a.enabled { "" } else { " [disabled]" };
            let cond = if a.condition.is_some() { " [conditional]" } else { "" };
            lines.push(format!(
                "  - {} ({}{}{}) runs: {} [{}]",
                a.name, trigger, enabled, cond, a.run_count, a.automation_id
            ));
        }
        lines.join("\n")
    }
}

// ── Run Automation ──

struct RunAutomationTool;

impl Tool for RunAutomationTool {
    fn name(&self) -> &'static str { "run_automation" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "automation" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "run_automation",
                "description": "Run a saved automation by name or ID",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Automation name (case-insensitive search)"
                        },
                        "automation_id": {
                            "type": "string",
                            "description": "Automation ID (exact match)"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let automation = if let Some(id) = args.get("automation_id").and_then(|v| v.as_str()) {
            AutomationStore::get(ctx.db.conn(), id)
        } else if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
            AutomationStore::find_by_name(ctx.db.conn(), name)
        } else {
            return "Error: provide name or automation_id".to_string();
        };

        let Some(automation) = automation else {
            return "Error: automation not found".to_string();
        };

        if !automation.enabled {
            return format!("Automation '{}' is disabled.", automation.name);
        }

        // Record the run
        AutomationStore::record_run(ctx.db.conn(), &automation.automation_id);

        // Return steps for the LLM to execute
        let condition_note = if let Some(cond) = &automation.condition {
            format!("\n\nCondition to check first: {}", cond)
        } else {
            String::new()
        };

        format!(
            "EXECUTE automation '{}': {}{}",
            automation.name, automation.steps, condition_note
        )
    }
}

// ── Delete Automation ──

struct DeleteAutomationTool;

impl Tool for DeleteAutomationTool {
    fn name(&self) -> &'static str { "delete_automation" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "automation" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "delete_automation",
                "description": "Delete an automation by name or ID",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Automation name"
                        },
                        "automation_id": {
                            "type": "string",
                            "description": "Automation ID"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let automation_id = if let Some(id) = args.get("automation_id").and_then(|v| v.as_str()) {
            id.to_string()
        } else if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
            match AutomationStore::find_by_name(ctx.db.conn(), name) {
                Some(a) => a.automation_id,
                None => return format!("No automation found with name '{}'", name),
            }
        } else {
            return "Error: provide name or automation_id".to_string();
        };

        // Also cancel linked schedule if any
        if let Some(a) = AutomationStore::get(ctx.db.conn(), &automation_id) {
            if let Some(schedule_id) = a.trigger_config.get("schedule_id").and_then(|v| v.as_str()) {
                Scheduler::cancel(ctx.db.conn(), schedule_id);
            }
        }

        if AutomationStore::archive(ctx.db.conn(), &automation_id) {
            format!("Automation '{}' deleted.", automation_id)
        } else {
            format!("No automation found with ID '{}'", automation_id)
        }
    }
}

// ── Toggle Automation ──

struct ToggleAutomationTool;

impl Tool for ToggleAutomationTool {
    fn name(&self) -> &'static str { "toggle_automation" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "automation" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "toggle_automation",
                "description": "Enable or disable an automation",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "automation_id": {
                            "type": "string",
                            "description": "Automation ID"
                        },
                        "enabled": {
                            "type": "boolean",
                            "description": "true to enable, false to disable"
                        }
                    },
                    "required": ["automation_id", "enabled"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let id = args.get("automation_id").and_then(|v| v.as_str()).unwrap_or_default();
        let enabled = args.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);

        if id.is_empty() {
            return "Error: automation_id is required".to_string();
        }

        if AutomationStore::set_enabled(ctx.db.conn(), id, enabled) {
            let state = if enabled { "enabled" } else { "disabled" };
            format!("Automation '{}' {}.", id, state)
        } else {
            format!("No active automation found with ID '{}'", id)
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
