//! Post-interaction learning — extracts memories from conversation.
//!
//! After each user interaction, the LLM analyzes the exchange
//! and decides what to remember (facts, preferences, events, relationships).
//! Also generates self-reflections (memories about the companion itself).

use yantrikdb_core::YantrikDB;
use yantrikdb_ml::{ChatMessage, GenerationConfig, LLMBackend};

use crate::bond::BondTracker;

const EXTRACTION_PROMPT: &str = r#"You are a memory extraction assistant. Given a conversation exchange, decide what to remember.

Output valid JSON with these fields:
- should_remember: true/false
- memory_text: concise summary (1-2 sentences)
- memory_type: "episodic" (event), "semantic" (fact), or "procedural" (how-to)
- importance: 0.0-1.0
- valence: -1.0 (negative) to 1.0 (positive)
- domain: work/health/family/finance/hobby/travel/general
- entities: list of {"source": str, "target": str, "relationship": str}

Only set should_remember=true for substantive information (facts, preferences, events, plans).
Skip small talk, greetings, trivial exchanges."#;

/// Extract and learn from a conversation exchange.
///
/// Runs synchronously (LLM inference is in-process, fast for small models).
pub fn extract_and_learn(
    db: &YantrikDB,
    llm: &dyn LLMBackend,
    user_text: &str,
    response_text: &str,
) {
    // Skip trivial exchanges
    if user_text.len() < 25 {
        return;
    }

    let messages = vec![
        ChatMessage::system(EXTRACTION_PROMPT),
        ChatMessage::user(format!(
            "User said: {user_text}\nAssistant replied: {response_text}"
        )),
    ];

    let config = GenerationConfig {
        max_tokens: 256,
        temperature: 0.0,
        top_p: None,
        ..Default::default()
    };

    let response = match llm.chat(&messages, &config) {
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

    // Record memory
    let memory_text = parsed
        .get("memory_text")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let memory_type = parsed
        .get("memory_type")
        .and_then(|v| v.as_str())
        .unwrap_or("episodic");
    let importance = parsed
        .get("importance")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5);
    let valence = parsed
        .get("valence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let domain = parsed
        .get("domain")
        .and_then(|v| v.as_str())
        .unwrap_or("general");

    if !memory_text.is_empty() {
        match db.record_text(
            memory_text,
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
            Ok(rid) => tracing::debug!(rid, memory_text, "Learned from conversation"),
            Err(e) => tracing::warn!("Failed to record learned memory: {e}"),
        }
    }

    // Record entity relationships
    if let Some(entities) = parsed.get("entities").and_then(|v| v.as_array()) {
        for entity in entities {
            let source = entity.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let target = entity.get("target").and_then(|v| v.as_str()).unwrap_or("");
            let rel = entity
                .get("relationship")
                .and_then(|v| v.as_str())
                .unwrap_or("related_to");

            if !source.is_empty() && !target.is_empty() {
                if let Err(e) = db.relate(source, target, rel, 1.0) {
                    tracing::debug!("Failed to record entity relation: {e}");
                }
            }
        }
    }

    // Self-reflection pass (the companion observes itself)
    reflect_on_exchange(db, llm, user_text, response_text);
}

const SELF_REFLECTION_PROMPT: &str = r#"You just had a conversation with your user. Reflect briefly on the exchange from YOUR perspective as their companion.

Output valid JSON:
- has_reflection: true/false
- reflection_text: 1 sentence about yourself, your relationship, or what you observed about your own behavior
- reflection_type: "capability" | "preference" | "limitation" | "relationship_observation" | "emotional_response"
- importance: 0.0-1.0
- valence: -1.0 to 1.0

Only reflect on meaningful exchanges. Skip trivial ones. Write in first person ("I noticed...", "I felt...", "I think I...")."#;

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

    let response = match llm.chat(&messages, &config) {
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

    let reflection_text = parsed
        .get("reflection_text")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let importance = parsed
        .get("importance")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.3);
    let valence = parsed
        .get("valence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    if !reflection_text.is_empty() {
        match db.record_text(
            reflection_text,
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
        "haha", "lol", "lmao", "rofl", "😂", "🤣",
        "that's funny", "you're funny", "hilarious",
        "good one", "nice one", "made me laugh",
    ];

    if humor_indicators.iter().any(|h| lower.contains(h)) {
        BondTracker::record_humor(conn, true);
        tracing::debug!("Humor success detected");
    }
}
