//! `forget_topic` (#1311): forgetting everything on a topic is the deletion that goes furthest,
//! so it asks `delete_for_good` and shows what goes: the text of every memory it would
//! tombstone, the entities it would delete and the schedules it would cancel. It works out what
//! matches first, asks, and only then changes anything, so the card and the change are the same
//! set. A topic that matches too much to show whole is not asked about and nothing is forgotten.

use rusqlite::params;
use yantrik_companion_tools::asks::{act, quoted, Card, Question};

use super::super::ToolContext;

fn list(items: &[String]) -> String {
    items.iter().map(|t| quoted(t).as_str().unwrap_or_default().to_string()).collect::<Vec<_>>().join("; ")
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}

/// Everything one run would change, found before anything is.
struct Matches {
    memories: Vec<(String, String)>,
    entities: Vec<(String, String)>,
    schedules: Vec<String>,
}

fn find(ctx: &ToolContext, topic: &str, keywords: &[String], patterns: &[String]) -> Matches {
    let conn = &ctx.db.conn();
    let topic_lower = topic.to_lowercase();
    let mut rids: Vec<String> = match ctx.db.recall_text(topic, 20) {
        Ok(results) => results
            .iter()
            .filter(|r| {
                // Must actually be about the topic, not just vaguely similar.
                let text = r.text.to_lowercase();
                text.contains(&topic_lower) || keywords.iter().any(|kw| text.contains(&kw.to_lowercase()))
            })
            .map(|r| r.rid.clone())
            .collect(),
        Err(_) => Vec::new(),
    };
    let mut entities: Vec<(String, String)> = Vec::new();
    let mut schedules: Vec<String> = Vec::new();
    for pattern in patterns {
        if let Ok(mut stmt) = conn.prepare("SELECT rid FROM memories WHERE LOWER(text) LIKE ?1 AND consolidation_status = 'active'") {
            if let Ok(rows) = stmt.query_map(params![pattern], |row| row.get::<_, String>(0)) {
                for rid in rows.flatten() {
                    if !rids.contains(&rid) {
                        rids.push(rid);
                    }
                }
            }
        }
        if let Ok(mut stmt) =
            conn.prepare("SELECT id, display_name FROM cortex_entities WHERE LOWER(display_name) LIKE ?1 OR LOWER(system_aliases) LIKE ?1")
        {
            if let Ok(rows) = stmt.query_map(params![pattern], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))) {
                for entity in rows.flatten() {
                    if !entities.iter().any(|known| known.0 == entity.0) {
                        entities.push(entity);
                    }
                }
            }
        }
        if let Ok(mut stmt) = conn.prepare(
            "SELECT label FROM scheduled_tasks WHERE (LOWER(label) LIKE ?1 OR LOWER(description) LIKE ?1) AND status IN ('active', 'paused')",
        ) {
            if let Ok(rows) = stmt.query_map(params![pattern], |row| row.get::<_, String>(0)) {
                for label in rows.flatten() {
                    if !schedules.contains(&label) {
                        schedules.push(label);
                    }
                }
            }
        }
    }
    let memories = rids
        .into_iter()
        .map(|rid| {
            let text = conn.query_row("SELECT text FROM memories WHERE rid = ?1", params![rid], |r| r.get::<_, String>(0)).unwrap_or_default();
            (rid, text)
        })
        .collect();
    Matches { memories, entities, schedules }
}

/// The card: what goes, quoted, a row for each kind of thing that does.
fn card(topic: &str, found: &Matches) -> Card {
    let mut card = Card::new("forget_topic").text("topic", topic);
    let texts: Vec<String> = found.memories.iter().map(|m| m.1.clone()).collect();
    card = card.plain("memories", format!("{}: {}", texts.len(), list(&texts)));
    if !found.entities.is_empty() {
        let names: Vec<String> = found.entities.iter().map(|e| e.1.clone()).collect();
        card = card.plain("entities", format!("{} deleted: {}", names.len(), list(&names)));
    }
    if !found.schedules.is_empty() {
        card = card.plain("schedules", format!("{} cancelled: {}", found.schedules.len(), list(&found.schedules)));
    }
    card.plain("also", "adds a rule not to bring the topic up again")
}

/// Forget a topic: find what matches, ask, then do it. The answer is the sentence for the model.
pub fn forget(ctx: &ToolContext, topic: &str, keywords: &[String]) -> String {
    let mut patterns: Vec<String> = vec![format!("%{}%", topic.to_lowercase())];
    patterns.extend(keywords.iter().map(|kw| format!("%{}%", kw.to_lowercase())));
    let found = find(ctx, topic, keywords, &patterns);
    let done = act(Question::DeleteForGood, &card(topic, &found), || {
        let conn = &ctx.db.conn();
        let ts = now();
        let mut tombstoned = 0u32;
        for (rid, _) in &found.memories {
            if let Ok(changes) = conn.execute(
                "UPDATE memories SET consolidation_status = 'tombstoned', updated_at = ?1 WHERE rid = ?2 AND consolidation_status = 'active'",
                params![ts, rid],
            ) {
                tombstoned += changes as u32;
            }
        }
        let mut deleted = 0u32;
        for (id, _) in &found.entities {
            // Cascade: remove relationships, pulse links, baselines, patterns.
            for sql in [
                "DELETE FROM cortex_relationships WHERE source_id = ?1 OR target_id = ?1",
                "DELETE FROM cortex_pulse_entities WHERE entity_id = ?1",
                "DELETE FROM cortex_baselines WHERE entity_id = ?1",
                "DELETE FROM cortex_patterns WHERE antecedent = ?1 OR consequent = ?1",
            ] {
                let _ = conn.execute(sql, params![id]);
            }
            if let Ok(changes) = conn.execute("DELETE FROM cortex_entities WHERE id = ?1", params![id]) {
                deleted += changes as u32;
            }
        }
        let mut cancelled = 0u32;
        for pattern in &patterns {
            if let Ok(changes) = conn.execute(
                "UPDATE scheduled_tasks SET status = 'cancelled', updated_at = ?1 \
                 WHERE (LOWER(label) LIKE ?2 OR LOWER(description) LIKE ?2) AND status IN ('active', 'paused')",
                params![ts, pattern],
            ) {
                cancelled += changes as u32;
            }
        }
        // A high-importance memory that prevents re-learning the topic.
        let suppression = format!(
            "USER PREFERENCE: Do NOT discuss, track, monitor, or bring up '{topic}'. \
             The user explicitly asked to forget this topic. \
             Do not create memories, entities, or scheduled tasks about it. \
             If this topic comes up in conversation, acknowledge you've been asked not to discuss it."
        );
        let _ = ctx.db.record_text(
            &suppression,
            "semantic",
            0.95,
            0.0,
            7_776_000.0,
            &serde_json::json!({"type": "topic_suppression", "suppressed_topic": topic}),
            "default",
            0.95,
            "preference",
            "companion",
            None,
        );
        Ok((tombstoned, deleted, cancelled))
    });
    match done {
        Ok((memories, entities, tasks)) => format!(
            "Forgot topic '{topic}': tombstoned {memories} memories, \
             deleted {entities} cortex entities, \
             cancelled {tasks} scheduled tasks. \
             Added suppression rule — I won't bring this up again."
        ),
        Err(why) => why,
    }
}
