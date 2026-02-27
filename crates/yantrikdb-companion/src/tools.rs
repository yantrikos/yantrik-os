//! Companion tools — remember, recall, relate, set_reminder,
//! introspect, form_opinion, create_inside_joke, check_bond.
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

/// Execute a tool call and return the result as a string.
pub fn execute_tool(db: &YantrikDB, name: &str, args: &serde_json::Value) -> String {
    match name {
        "remember" => tool_remember(db, args),
        "recall" => tool_recall(db, args),
        "relate_entities" => tool_relate(db, args),
        "set_reminder" => tool_set_reminder(db, args),
        "introspect" => tool_introspect(db, args),
        "form_opinion" => tool_form_opinion(db.conn(), args),
        "create_inside_joke" => tool_create_inside_joke(db.conn(), args),
        "check_bond" => tool_check_bond(db.conn()),
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
