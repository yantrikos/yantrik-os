//! Memory tools — remember, recall, relate, set_reminder, introspect,
//! form_opinion, create_inside_joke, check_bond, forget_topic.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};
use rusqlite::params;

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(RememberTool));
    reg.register(Box::new(RecallTool));
    reg.register(Box::new(RelateEntitiesTool));
    reg.register(Box::new(SetReminderTool));
    reg.register(Box::new(IntrospectTool));
    reg.register(Box::new(FormOpinionTool));
    reg.register(Box::new(CreateInsideJokeTool));
    reg.register(Box::new(CheckBondTool));
    reg.register(Box::new(SaveUserFactTool));
    reg.register(Box::new(ForgetTopicTool));
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
                "description": "Store important facts about the user. Use 'facts' array to save multiple facts at once, or 'text' for a single fact.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "description": "What to remember (single fact)"},
                        "facts": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Multiple facts to remember at once (e.g. ['likes Thai food', 'lives in Dallas', 'works at Acme'])"
                        },
                        "importance": {"type": "number", "description": "0.0-1.0"},
                        "memory_type": {
                            "type": "string",
                            "enum": ["episodic", "semantic", "procedural"]
                        },
                        "domain": {
                            "type": "string",
                            "description": "Topic: work, health, family, finance, hobby, general"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        if ctx.incognito {
            return "Incognito mode is active \u{2014} memory not saved.".to_string();
        }

        let importance = args.get("importance").and_then(|v| v.as_f64()).unwrap_or(0.5);
        let memory_type = args.get("memory_type").and_then(|v| v.as_str()).unwrap_or("episodic");
        let domain = args.get("domain").and_then(|v| v.as_str()).unwrap_or("general");

        // Collect facts from both "text" (single) and "facts" (array)
        let mut all_facts: Vec<String> = Vec::new();
        if let Some(text) = args.get("text").and_then(|v| v.as_str()) {
            if !text.is_empty() {
                all_facts.push(text.to_string());
            }
        }
        if let Some(facts_arr) = args.get("facts").and_then(|v| v.as_array()) {
            for item in facts_arr {
                if let Some(s) = item.as_str() {
                    if !s.is_empty() {
                        all_facts.push(s.to_string());
                    }
                }
            }
        }

        if all_facts.is_empty() {
            return "Error: 'text' or 'facts' is required".to_string();
        }

        let mut saved = 0u32;
        let mut skipped = 0u32;
        let mut results = Vec::new();

        for fact in &all_facts {
            if crate::learning::is_duplicate(ctx.db, fact) {
                skipped += 1;
                continue;
            }
            match ctx.db.record_text(
                fact, memory_type, importance, 0.0, 604800.0,
                &serde_json::json!({}), "default", 0.9, domain, "companion", None,
            ) {
                Ok(_) => saved += 1,
                Err(e) => results.push(format!("Failed: {e}")),
            }
        }

        if all_facts.len() == 1 {
            if saved == 1 {
                format!("Remembered: {}", all_facts[0])
            } else if skipped == 1 {
                format!("Already remembered something similar to: {}", all_facts[0])
            } else {
                results.join("; ")
            }
        } else {
            let mut parts = vec![format!("Remembered {saved}/{} facts", all_facts.len())];
            if skipped > 0 {
                parts.push(format!("({skipped} duplicates skipped)"));
            }
            parts.join(" ")
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
                        "remind_at": {"type": "string", "description": "ISO format YYYY-MM-DDTHH:MM"},
                        "in": {"type": "string", "description": "Relative offset like '30m', '2h', '1d'. Alternative to remind_at."},
                        "time_offset": {"type": "string", "description": "Alias for 'in'."}
                    },
                    "required": ["text"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        let remind_at_str = args.get("remind_at").and_then(|v| v.as_str()).unwrap_or_default();
        let offset_str = args.get("in")
            .or_else(|| args.get("time_offset"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        if text.is_empty() {
            return "Error: text is required".to_string();
        }

        let remind_ts = if !remind_at_str.is_empty() {
            match chrono::NaiveDateTime::parse_from_str(remind_at_str, "%Y-%m-%dT%H:%M") {
                Ok(dt) => dt.and_utc().timestamp() as f64,
                Err(_) => {
                    return format!("Error: invalid remind_at format '{remind_at_str}', use YYYY-MM-DDTHH:MM");
                }
            }
        } else if !offset_str.is_empty() {
            match crate::tools::scheduler::parse_duration_offset(offset_str) {
                Some(secs) if secs >= 10 => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs_f64();
                    now + secs as f64
                }
                _ => return format!("Error: invalid offset '{}'. Use format like '30m', '2h', '1d'", offset_str),
            }
        } else {
            return "Error: remind_at or 'in' (e.g. '30m') is required".to_string();
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

// ── Save User Fact ──

pub struct SaveUserFactTool;

impl Tool for SaveUserFactTool {
    fn name(&self) -> &'static str { "save_user_fact" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "memory" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "save_user_fact",
                "description": "Save a confirmed user fact or preference as a high-importance, long-lived memory. Use when the user explicitly confirms a preference, identity fact, or important personal detail. More persistent than regular 'remember' — lasts 30 days with high importance.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "fact": {
                            "type": "string",
                            "description": "The confirmed fact to save (e.g., 'User prefers Thai food', 'User lives in Dallas')"
                        },
                        "domain": {
                            "type": "string",
                            "description": "Fact category: preference, identity, location, health, work, family, finance, hobby, travel, general"
                        },
                        "confidence": {
                            "type": "number",
                            "description": "How confident we are about this fact (0.0-1.0, default: 0.9)"
                        }
                    },
                    "required": ["fact", "domain"]
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        if ctx.incognito {
            return "Incognito mode is active — fact not saved.".to_string();
        }

        let fact = args.get("fact").and_then(|v| v.as_str()).unwrap_or_default();
        if fact.is_empty() {
            return "Error: fact is required".to_string();
        }

        let raw_domain = args.get("domain").and_then(|v| v.as_str()).unwrap_or("general");
        let domain = crate::sanitize::validate_domain(raw_domain);
        let confidence = args.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.9);

        // Write-time dedup — check if we already have this fact
        if crate::learning::is_duplicate(ctx.db, fact) {
            return format!("Already have a similar fact: {fact}");
        }

        // High importance (0.8), long half-life (30 days), semantic memory type
        let importance = crate::sanitize::clamp_importance(0.8 * confidence);
        match ctx.db.record_text(
            fact,
            "semantic",      // confirmed facts are semantic, not episodic
            importance,
            0.0,             // neutral valence
            2_592_000.0,     // 30-day half-life (vs 7 days for regular memories)
            &serde_json::json!({}),
            "default",
            0.95,            // high embedding confidence
            domain,
            "companion",
            None,
        ) {
            Ok(rid) => format!("Fact saved: {fact} (domain: {domain}, id: {rid})"),
            Err(e) => format!("Failed to save fact: {e}"),
        }
    }
}

// ── Forget Topic ──

pub struct ForgetTopicTool;

impl Tool for ForgetTopicTool {
    fn name(&self) -> &'static str { "forget_topic" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "memory" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "forget_topic",
                "description": "Forget everything about a topic the user no longer wants you to track or discuss. Removes related memories, cortex entities, and scheduled tasks, then adds a suppression rule so you don't re-learn it. Use when the user says things like 'stop talking about X', 'forget about X', 'don't bring up X anymore'.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "topic": {
                            "type": "string",
                            "description": "The topic to forget (e.g. 'stock market', 'crypto', 'my ex')"
                        },
                        "keywords": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Additional keywords to match against memories and entities (e.g. ['SPG', 'VICI', 'ticker', 'watchlist'])"
                        }
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

        let keywords: Vec<String> = args.get("keywords")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let conn = ctx.db.conn();
        let topic_lower = topic.to_lowercase();
        let like_pattern = format!("%{}%", topic_lower);

        let mut tombstoned_memories = 0u32;
        let mut deleted_entities = 0u32;
        let mut cancelled_tasks = 0u32;

        // ── 1. Tombstone memories matching the topic ──

        // 1a. Semantic search — find memories related to the topic
        let semantic_rids: Vec<String> = match ctx.db.recall_text(topic, 20) {
            Ok(results) => results.iter()
                .filter(|r| {
                    let text_lower = r.text.to_lowercase();
                    // Must actually be about the topic, not just vaguely similar
                    text_lower.contains(&topic_lower)
                        || keywords.iter().any(|kw| text_lower.contains(&kw.to_lowercase()))
                })
                .map(|r| r.rid.clone())
                .collect(),
            Err(_) => Vec::new(),
        };

        // 1b. Direct text search — catch anything semantic search missed
        let mut text_rids: Vec<String> = Vec::new();
        let mut patterns: Vec<String> = vec![like_pattern.clone()];
        for kw in &keywords {
            patterns.push(format!("%{}%", kw.to_lowercase()));
        }

        for pattern in &patterns {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT rid FROM memories WHERE LOWER(text) LIKE ?1 AND consolidation_status = 'active'"
            ) {
                if let Ok(rows) = stmt.query_map(params![pattern], |row| row.get::<_, String>(0)) {
                    for rid in rows.flatten() {
                        if !text_rids.contains(&rid) {
                            text_rids.push(rid);
                        }
                    }
                }
            }
        }

        // Merge and deduplicate
        let mut all_rids = semantic_rids;
        for rid in text_rids {
            if !all_rids.contains(&rid) {
                all_rids.push(rid);
            }
        }

        // Tombstone them
        for rid in &all_rids {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            if let Ok(changes) = conn.execute(
                "UPDATE memories SET consolidation_status = 'tombstoned', updated_at = ?1 WHERE rid = ?2 AND consolidation_status = 'active'",
                params![ts, rid],
            ) {
                tombstoned_memories += changes as u32;
            }
        }

        // ── 2. Delete cortex entities matching the topic ──

        let mut entity_ids: Vec<String> = Vec::new();
        for pattern in &patterns {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT id FROM cortex_entities WHERE LOWER(display_name) LIKE ?1 OR LOWER(system_aliases) LIKE ?1"
            ) {
                if let Ok(rows) = stmt.query_map(params![pattern], |row| row.get::<_, String>(0)) {
                    for id in rows.flatten() {
                        if !entity_ids.contains(&id) {
                            entity_ids.push(id);
                        }
                    }
                }
            }
        }

        for entity_id in &entity_ids {
            // Cascade: remove relationships, pulse links, baselines, patterns
            let _ = conn.execute(
                "DELETE FROM cortex_relationships WHERE source_id = ?1 OR target_id = ?1",
                params![entity_id],
            );
            let _ = conn.execute(
                "DELETE FROM cortex_pulse_entities WHERE entity_id = ?1",
                params![entity_id],
            );
            let _ = conn.execute(
                "DELETE FROM cortex_baselines WHERE entity_id = ?1",
                params![entity_id],
            );
            let _ = conn.execute(
                "DELETE FROM cortex_patterns WHERE antecedent = ?1 OR consequent = ?1",
                params![entity_id],
            );
            if let Ok(changes) = conn.execute(
                "DELETE FROM cortex_entities WHERE id = ?1",
                params![entity_id],
            ) {
                deleted_entities += changes as u32;
            }
        }

        // ── 3. Cancel scheduled tasks matching the topic ──

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        for pattern in &patterns {
            if let Ok(changes) = conn.execute(
                "UPDATE scheduled_tasks SET status = 'cancelled', updated_at = ?1 \
                 WHERE (LOWER(label) LIKE ?2 OR LOWER(description) LIKE ?2) \
                 AND status IN ('active', 'paused')",
                params![ts, pattern],
            ) {
                cancelled_tasks += changes as u32;
            }
        }

        // ── 4. Add suppression memory ──
        // High-importance semantic memory that prevents re-learning

        let suppression_text = format!(
            "USER PREFERENCE: Do NOT discuss, track, monitor, or bring up '{}'. \
             The user explicitly asked to forget this topic. \
             Do not create memories, entities, or scheduled tasks about it. \
             If this topic comes up in conversation, acknowledge you've been asked not to discuss it.",
            topic
        );

        let _ = ctx.db.record_text(
            &suppression_text,
            "semantic",
            0.95,          // very high importance — should surface in any related query
            0.0,
            7_776_000.0,   // 90-day half-life — long-lasting suppression
            &serde_json::json!({"type": "topic_suppression", "suppressed_topic": topic}),
            "default",
            0.95,
            "preference",
            "companion",
            None,
        );

        format!(
            "Forgot topic '{topic}': tombstoned {tombstoned_memories} memories, \
             deleted {deleted_entities} cortex entities, \
             cancelled {cancelled_tasks} scheduled tasks. \
             Added suppression rule — I won't bring this up again."
        )
    }
}
