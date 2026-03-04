//! Post-interaction learning — extracts memories from conversation.
//!
//! After each user interaction, the LLM analyzes the exchange
//! and decides what to remember (facts, preferences, events, relationships).
//! Also generates self-reflections (memories about the companion itself).
//!
//! V25: Atomic extraction (one fact per memory), write-time dedup gate,
//! compound-splitting fallback, and cleaned response input.

use yantrikdb_core::YantrikDB;
use yantrikdb_ml::{ChatMessage, GenerationConfig, LLMBackend};

use crate::bond::BondTracker;
use crate::config::MemoryEvolutionConfig;
use crate::sanitize;

/// Similarity threshold for write-time dedup. If the best existing match
/// has similarity >= this, the new memory is skipped as a duplicate.
const DEDUP_SIMILARITY_THRESHOLD: f64 = 0.85;

/// Maximum atomic memories to store per exchange.
const MAX_MEMORIES_PER_EXCHANGE: usize = 5;

/// System event prefixes that should NEVER be stored as learned memories.
/// These come from the SystemObserver and are stored separately with system/* domains.
const SYSTEM_EVENT_PREFIXES: &[&str] = &[
    "App opened:", "App closed:", "CPU spike:", "Memory high:",
    "Battery ", "Network disconnected", "Connected to network",
    "File created:", "File modified:", "File deleted:", "File renamed:",
    "User idle ", "User returned", "Disk low ", "Notification from ",
];

const EXTRACTION_PROMPT: &str = r#"You are a memory extraction assistant. Given a conversation exchange, extract atomic facts to remember.

Output valid JSON:
{
  "should_remember": true/false,
  "memories": [
    {
      "text": "single atomic fact, max 120 chars",
      "type": "episodic" | "semantic" | "procedural",
      "importance": 0.0-1.0,
      "valence": -1.0 to 1.0,
      "domain": "identity" | "preference" | "work" | "health" | "family" | "finance" | "hobby" | "travel" | "location" | "general",
      "entities": [{"source": "str", "target": "str", "relationship": "str"}]
    }
  ]
}

RULES:
- Each memory = ONE atomic fact. Never combine facts with "|", "and also", or semicolons.
- Max 150 characters per memory text.
- ALWAYS extract personal facts the user shares: name, age, location, city, country, timezone, job, company, relationships, interests, etc. These are HIGH importance.
- domain "identity" = facts about who the user IS (name, role, background, age).
- domain "location" = where the user lives, works, or is located (city, country, timezone).
- domain "preference" = things the user likes/dislikes/prefers.
- For episodic memories, include brief context: "User mentioned X while discussing Y".
- Skip tool calls, memory searches, system operations — only extract from conversation content.
- Skip greetings-only messages, but DO extract facts even from short messages like "I'm in Dallas" or "I'm 30".
- NEVER extract system events: app starts/stops, CPU/memory/battery/network/disk changes, file operations, process lists. These are logged separately.
- When the user expresses preferences (likes/dislikes, favorite things, preferred styles, dietary restrictions, budget ranges), extract with domain "preference" and importance >= 0.7.
- Preference examples: "I love Thai food" → domain: preference, importance: 0.8. "I prefer window seats" → domain: preference, importance: 0.7.
- When user confirms a search choice ("I'll go with restaurant X", "book that hotel"), extract with domain "preference", importance: 0.7.
- Max 5 memories per exchange."#;

/// Extract and learn from a conversation exchange.
///
/// The response_text should already be cleaned via `sanitize::clean_response_for_learning()`
/// before calling this function.
pub fn extract_and_learn(
    db: &YantrikDB,
    llm: &dyn LLMBackend,
    user_text: &str,
    response_text: &str,
    evolution_config: &MemoryEvolutionConfig,
) {
    // Skip trivial exchanges (but not too aggressively — short messages
    // like "I'm in Dallas" or "I'm 30" are still valuable user facts)
    if user_text.len() < 8 {
        return;
    }

    // Skip if response was entirely tool output (cleaned to empty)
    if response_text.is_empty() {
        return;
    }

    let messages = vec![
        ChatMessage::system(EXTRACTION_PROMPT),
        ChatMessage::user(format!(
            "User said: {user_text}\nAssistant replied: {response_text}"
        )),
    ];

    let config = GenerationConfig {
        max_tokens: 512,
        temperature: 0.0,
        top_p: None,
        ..Default::default()
    };

    let response = match llm.chat(&messages, &config, None) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Learning extraction failed: {e}");
            return;
        }
    };

    // Parse JSON from response (handle markdown code block wrapping)
    let text = response.text.trim();
    let json_str = if text.starts_with("```") {
        text.lines()
            .skip(1)
            .take_while(|l| !l.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.to_string()
    };

    let parsed: serde_json::Value = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("Learning JSON parse failed: {e}, text: {json_str}");
            return;
        }
    };

    let should_remember = parsed
        .get("should_remember")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if !should_remember {
        return;
    }

    // V25: Parse memories array (with legacy single memory_text fallback)
    let memory_entries = extract_memory_entries(&parsed);
    let mut stored_count = 0;

    for entry in memory_entries.into_iter().take(MAX_MEMORIES_PER_EXCHANGE) {
        let raw_text = entry.get("text").and_then(|v| v.as_str()).unwrap_or_default();
        let memory_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("episodic");
        let importance = sanitize::clamp_importance(
            entry.get("importance").and_then(|v| v.as_f64()).unwrap_or(0.5)
        );
        let valence = sanitize::clamp_valence(
            entry.get("valence").and_then(|v| v.as_f64()).unwrap_or(0.0)
        );
        let raw_domain = entry.get("domain").and_then(|v| v.as_str()).unwrap_or("general");
        let domain = sanitize::validate_domain(raw_domain);

        // Validate memory text — reject injection attempts
        let memory_text = match sanitize::validate_memory_text(raw_text) {
            Some(t) => t,
            None => continue,
        };

        // Reject system event text that the LLM shouldn't be memorizing
        if SYSTEM_EVENT_PREFIXES.iter().any(|p| memory_text.starts_with(p)) {
            tracing::debug!(memory_text, "Rejecting system event from learning");
            continue;
        }

        // Write-time dedup gate: check if we already have this memory
        if is_duplicate(db, &memory_text) {
            tracing::debug!(memory_text, "Skipping duplicate memory");
            continue;
        }

        match db.record_text(
            &memory_text,
            memory_type,
            importance,
            valence,
            604800.0,
            &serde_json::json!({}),
            "default",
            0.9,
            domain,
            "companion",
            None,
        ) {
            Ok(rid) => {
                tracing::debug!(rid, memory_text, "Learned from conversation");
                // Assign importance tier + variable half-life (Gap 5)
                crate::memory_evolution::assign_memory_tier(
                    db.conn(), &rid, importance, memory_type, domain, evolution_config,
                );
                stored_count += 1;

                // Record per-memory entity relationships
                if let Some(entities) = entry.get("entities").and_then(|v| v.as_array()) {
                    for entity in entities.iter().take(3) {
                        let source_raw = entity.get("source").and_then(|v| v.as_str()).unwrap_or("");
                        let target_raw = entity.get("target").and_then(|v| v.as_str()).unwrap_or("");
                        let rel_raw = entity
                            .get("relationship")
                            .and_then(|v| v.as_str())
                            .unwrap_or("related_to");

                        if let (Some(source), Some(target), Some(rel)) = (
                            sanitize::validate_entity_field(source_raw),
                            sanitize::validate_entity_field(target_raw),
                            sanitize::validate_entity_field(rel_raw),
                        ) {
                            let _ = db.relate(source, target, rel, 1.0);
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("Failed to record learned memory: {e}"),
        }
    }

    if stored_count > 0 {
        tracing::debug!(stored_count, "Learning pass complete");
    }

    // Self-reflection pass (the companion observes itself)
    reflect_on_exchange(db, llm, user_text, response_text);
}

/// Extract memory entries from the LLM response.
///
/// Tries the V25 `memories` array first, falls back to legacy `memory_text` field,
/// and splits compound memories as a last resort.
fn extract_memory_entries(parsed: &serde_json::Value) -> Vec<serde_json::Value> {
    // V25: Try `memories` array first
    if let Some(memories) = parsed.get("memories").and_then(|v| v.as_array()) {
        if !memories.is_empty() {
            return memories.clone();
        }
    }

    // Legacy fallback: single `memory_text` field
    let raw_text = match parsed.get("memory_text").and_then(|v| v.as_str()) {
        Some(t) if !t.is_empty() => t,
        _ => return Vec::new(),
    };

    // If the legacy text is compound (pipe/semicolon separated, >150 chars), split it
    if raw_text.len() > 150 {
        return split_compound_memory(raw_text, parsed);
    }

    // Single atomic memory from legacy format
    vec![serde_json::json!({
        "text": raw_text,
        "type": parsed.get("memory_type").and_then(|v| v.as_str()).unwrap_or("episodic"),
        "importance": parsed.get("importance").and_then(|v| v.as_f64()).unwrap_or(0.5),
        "valence": parsed.get("valence").and_then(|v| v.as_f64()).unwrap_or(0.0),
        "domain": parsed.get("domain").and_then(|v| v.as_str()).unwrap_or("general"),
        "entities": parsed.get("entities").cloned().unwrap_or(serde_json::json!([]))
    })]
}

/// Split a compound memory text into atomic entries.
///
/// Handles pipe-separated ("fact1 | fact2"), semicolon-separated ("fact1; fact2"),
/// and period-separated compound blobs.
fn split_compound_memory(text: &str, parsed: &serde_json::Value) -> Vec<serde_json::Value> {
    let parts: Vec<&str> = if text.contains(" | ") {
        text.split(" | ").collect()
    } else if text.contains("; ") {
        text.split("; ").collect()
    } else if text.matches(". ").count() >= 2 {
        // Only split on periods if there are 3+ sentences
        text.split(". ")
            .filter(|s| s.len() > 10)
            .collect()
    } else {
        return vec![serde_json::json!({
            "text": text,
            "type": parsed.get("memory_type").and_then(|v| v.as_str()).unwrap_or("episodic"),
            "importance": parsed.get("importance").and_then(|v| v.as_f64()).unwrap_or(0.5),
            "valence": parsed.get("valence").and_then(|v| v.as_f64()).unwrap_or(0.0),
            "domain": parsed.get("domain").and_then(|v| v.as_str()).unwrap_or("general"),
            "entities": serde_json::json!([])
        })];
    };

    let base_importance = parsed.get("importance").and_then(|v| v.as_f64()).unwrap_or(0.5);
    let base_valence = parsed.get("valence").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let memory_type = parsed.get("memory_type").and_then(|v| v.as_str()).unwrap_or("episodic");
    let domain = parsed.get("domain").and_then(|v| v.as_str()).unwrap_or("general");

    parts
        .into_iter()
        .map(|part| {
            let trimmed = part.trim().trim_end_matches('.');
            serde_json::json!({
                "text": trimmed,
                "type": memory_type,
                "importance": base_importance,
                "valence": base_valence,
                "domain": domain,
                "entities": serde_json::json!([])
            })
        })
        .filter(|entry| {
            entry.get("text")
                .and_then(|v| v.as_str())
                .map(|t| t.len() >= 10)
                .unwrap_or(false)
        })
        .take(MAX_MEMORIES_PER_EXCHANGE)
        .collect()
}

/// Check if a memory is a near-duplicate of an existing record.
///
/// Does one embed + one cosine lookup (~2ms). Returns true if best match >= threshold.
pub fn is_duplicate(db: &YantrikDB, text: &str) -> bool {
    match db.recall_text(text, 1) {
        Ok(results) if !results.is_empty() => {
            results[0].scores.similarity >= DEDUP_SIMILARITY_THRESHOLD
        }
        _ => false,
    }
}

const SELF_REFLECTION_PROMPT: &str = r#"You just had a conversation with your user. Reflect briefly on the exchange from YOUR perspective as their companion.

Output valid JSON:
- has_reflection: true/false
- reflection_text: 1 sentence about yourself, your relationship, or what you observed about your own behavior
- reflection_type: "capability" | "preference" | "limitation" | "relationship_observation" | "emotional_response"
- importance: 0.0-1.0
- valence: -1.0 to 1.0

Only reflect on meaningful exchanges. Skip trivial ones. Write in first person ("I noticed...", "I felt...", "I think I...")."#;

/// Maximum self-reflections allowed per 30-minute window.
const MAX_REFLECTIONS_PER_WINDOW: i64 = 1;
/// Window duration in seconds for reflection rate-limiting.
const REFLECTION_WINDOW_SECS: f64 = 1800.0; // 30 minutes

/// Generate a self-reflection from the exchange and store as a self-memory.
fn reflect_on_exchange(
    db: &YantrikDB,
    llm: &dyn LLMBackend,
    user_text: &str,
    response_text: &str,
) {
    // Only reflect on substantial exchanges
    if user_text.len() < 50 {
        return;
    }

    // Rate-limit: max 1 self-reflection per 30 minutes
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let cutoff = now - REFLECTION_WINDOW_SECS;
    let recent_count: i64 = db.conn()
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE source = 'self' AND created_at > ?1",
            [cutoff],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if recent_count >= MAX_REFLECTIONS_PER_WINDOW {
        tracing::debug!(recent_count, "Self-reflection cap reached, skipping");
        return;
    }

    let messages = vec![
        ChatMessage::system(SELF_REFLECTION_PROMPT),
        ChatMessage::user(format!(
            "User said: {user_text}\nYou replied: {response_text}"
        )),
    ];

    let config = GenerationConfig {
        max_tokens: 128,
        temperature: 0.0,
        top_p: None,
        ..Default::default()
    };

    let response = match llm.chat(&messages, &config, None) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("Self-reflection failed: {e}");
            return;
        }
    };

    let text = response.text.trim();
    let json_str = if text.starts_with("```") {
        text.lines()
            .skip(1)
            .take_while(|l| !l.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.to_string()
    };

    let parsed: serde_json::Value = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(_) => return,
    };

    let has_reflection = parsed
        .get("has_reflection")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if !has_reflection {
        return;
    }

    let raw_reflection = parsed
        .get("reflection_text")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let importance = sanitize::clamp_importance(
        parsed.get("importance").and_then(|v| v.as_f64()).unwrap_or(0.3)
    );
    let valence = sanitize::clamp_valence(
        parsed.get("valence").and_then(|v| v.as_f64()).unwrap_or(0.0)
    );

    // Validate reflection text — reject injection attempts
    if let Some(reflection_text) = sanitize::validate_memory_text(raw_reflection) {
        // Write-time dedup for self-reflections too
        if is_duplicate(db, &reflection_text) {
            tracing::debug!(reflection_text, "Skipping duplicate self-reflection");
            return;
        }

        match db.record_text(
            &reflection_text,
            "semantic",
            importance,
            valence,
            1_209_600.0, // 14-day half-life for self-reflections
            &serde_json::json!({}),
            "default",
            0.8,
            "self-reflection",
            "self",
            None,
        ) {
            Ok(rid) => tracing::debug!(rid, reflection_text, "Self-reflection recorded"),
            Err(e) => tracing::debug!("Failed to record self-reflection: {e}"),
        }
    }
}

/// Detect if the user reacted to humor in the companion's previous response.
/// Call this with the user's NEW message to check reaction to PREVIOUS response.
pub fn detect_humor_reaction(conn: &rusqlite::Connection, user_text: &str) {
    let lower = user_text.to_lowercase();
    let humor_indicators = [
        "haha", "lol", "lmao", "rofl", "\u{1F602}", "\u{1F923}",
        "that's funny", "you're funny", "hilarious",
        "good one", "nice one", "made me laugh",
    ];

    if humor_indicators.iter().any(|h| lower.contains(h)) {
        BondTracker::record_humor(conn, true);
        tracing::debug!("Humor success detected");
    }
}
