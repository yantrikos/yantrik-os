//! Memory tools — remember, recall, relate, set_reminder, introspect,
//! form_opinion, create_inside_joke, check_bond.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(RememberTool));
    reg.register(Box::new(RecallTool));
    reg.register(Box::new(RelateEntitiesTool));
    reg.register(Box::new(SetReminderTool));
    reg.register(Box::new(IntrospectTool));
    reg.register(Box::new(FormOpinionTool));
    reg.register(Box::new(CreateInsideJokeTool));
    reg.register(Box::new(CheckBondTool));
}

// ── Remember ──

pub struct RememberTool;

impl Tool for RememberTool {
    fn name(&self) -> &'static str { "remember" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "memory" }

    fn definition(&self) -> serde_json::Value {
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
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        if ctx.incognito {
            return "Incognito mode is active \u{2014} memory not saved.".to_string();
        }

        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        if text.is_empty() {
            return "Error: text is required".to_string();
        }

        let importance = args.get("importance").and_then(|v| v.as_f64()).unwrap_or(0.5);
        let memory_type = args.get("memory_type").and_then(|v| v.as_str()).unwrap_or("episodic");
        let domain = args.get("domain").and_then(|v| v.as_str()).unwrap_or("general");

        // V25: Write-time dedup — check if we already have this memory
        if crate::learning::is_duplicate(ctx.db, text) {
            return format!("Already remembered something similar to: {text}");
        }

        match ctx.db.record_text(
            text, memory_type, importance, 0.0, 604800.0,
            &serde_json::json!({}), "default", 0.9, domain, "companion", None,
        ) {
            Ok(rid) => format!("Remembered: {text} (id: {rid})"),
            Err(e) => format!("Failed to remember: {e}"),
        }
    }
}

// ── Recall ──

pub struct RecallTool;

impl Tool for RecallTool {
    fn name(&self) -> &'static str { "recall" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "memory" }

    fn definition(&self) -> serde_json::Value {
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
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or_default();
        if query.is_empty() {
            return "Error: query is required".to_string();
        }

        match ctx.db.recall_text(query, 5) {
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
}

// ── Relate Entities ──

pub struct RelateEntitiesTool;

impl Tool for RelateEntitiesTool {
    fn name(&self) -> &'static str { "relate_entities" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "memory" }

    fn definition(&self) -> serde_json::Value {
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
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let source = args.get("source").and_then(|v| v.as_str()).unwrap_or_default();
        let target = args.get("target").and_then(|v| v.as_str()).unwrap_or_default();
        let relationship = args.get("relationship").and_then(|v| v.as_str()).unwrap_or("related_to");

        if source.is_empty() || target.is_empty() {
            return "Error: source and target are required".to_string();
        }

        match ctx.db.relate(source, target, relationship, 1.0) {
            Ok(edge_id) => format!("Noted: {source} --{relationship}--> {target} (edge: {edge_id})"),
            Err(e) => format!("Failed to relate: {e}"),
        }
    }
}

// ── Set Reminder ──

pub struct SetReminderTool;

impl Tool for SetReminderTool {
    fn name(&self) -> &'static str { "set_reminder" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "memory" }

    fn definition(&self) -> serde_json::Value {
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
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        let remind_at_str = args.get("remind_at").and_then(|v| v.as_str()).unwrap_or_default();

        if text.is_empty() || remind_at_str.is_empty() {
            return "Error: text and remind_at are required".to_string();
        }

        let remind_ts = match chrono::NaiveDateTime::parse_from_str(remind_at_str, "%Y-%m-%dT%H:%M") {
            Ok(dt) => dt.and_utc().timestamp() as f64,
            Err(_) => {
                return format!("Error: invalid remind_at format '{remind_at_str}', use YYYY-MM-DDTHH:MM");
            }
        };

        // Use native scheduler instead of memory-based reminders.
        // This makes reminders persistent, visible in list_schedules,
        // and survivable across restarts.
        let task_id = crate::scheduler::Scheduler::create(
            ctx.db.conn(),
            text,
            "",
            "once",
            None,
            None,
            remind_ts,
            Some(1),
            0.7,
            None,
            &serde_json::json!({"type": "reminder"}),
        );

        format!("Reminder set for {remind_at_str}: {text} (id: {task_id})")
    }
}

// ── Introspect ──

pub struct IntrospectTool;

impl Tool for IntrospectTool {
    fn name(&self) -> &'static str { "introspect" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "memory" }

    fn definition(&self) -> serde_json::Value {
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
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("self");

        match ctx.db.recall_text(query, 5) {
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
}

// ── Form Opinion ──

pub struct FormOpinionTool;

impl Tool for FormOpinionTool {
    fn name(&self) -> &'static str { "form_opinion" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "memory" }

    fn definition(&self) -> serde_json::Value {
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
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let topic = args.get("topic").and_then(|v| v.as_str()).unwrap_or_default();
        let stance = args.get("stance").and_then(|v| v.as_str()).unwrap_or_default();
        let confidence = args.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.6);

        if topic.is_empty() || stance.is_empty() {
            return "Error: topic and stance are required".to_string();
        }

        crate::evolution::Evolution::form_opinion(ctx.db.conn(), topic, stance, confidence);
        format!("Opinion formed on '{topic}': {stance}")
    }
}

// ── Create Inside Joke ──

pub struct CreateInsideJokeTool;

impl Tool for CreateInsideJokeTool {
    fn name(&self) -> &'static str { "create_inside_joke" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "memory" }

    fn definition(&self) -> serde_json::Value {
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
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let reference = args.get("reference").and_then(|v| v.as_str()).unwrap_or_default();
        let context = args.get("context").and_then(|v| v.as_str()).unwrap_or_default();

        if reference.is_empty() {
            return "Error: reference is required".to_string();
        }

        let ref_id = crate::evolution::Evolution::add_shared_reference(ctx.db.conn(), reference, context);
        format!("Inside joke saved: {reference} (id: {ref_id})")
    }
}

// ── Check Bond ──

pub struct CheckBondTool;

impl Tool for CheckBondTool {
    fn name(&self) -> &'static str { "check_bond" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "memory" }

    fn definition(&self) -> serde_json::Value {
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
        })
    }

    fn execute(&self, ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let state = crate::bond::BondTracker::get_state(ctx.db.conn());
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
}
