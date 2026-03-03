//! Memory Evolution — closes Yantrik's 5 self-identified memory gaps.
//!
//! Gap 1: Smart multi-signal recall (contextual augmentation)
//! Gap 2: Cross-domain entity bridging (break storage silos)
//! Gap 3: Semantic drift correction (consolidate similar memories)
//! Gap 4: Dynamic shared references (freshness decay + auto-detection)
//! Gap 5: Variable half-lives + pruning (importance-based retention)
//!
//! All heavy operations (consolidation, pruning, reference detection) run in the
//! background think cycle. Message-path operations (smart recall, tier assignment,
//! context update) are fast and LLM-free.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use yantrikdb_core::YantrikDB;
use yantrikdb_core::types::RecallResult;
use yantrikdb_ml::{ChatMessage, GenerationConfig, LLMBackend};

use crate::config::MemoryEvolutionConfig;
use crate::evolution::{Evolution, SharedReference};
use crate::sanitize;

// ── SQL Schema ──────────────────────────────────────────────────────────────

/// Create all companion-managed tables for memory evolution.
pub fn ensure_tables(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS conversation_context (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            topic_text TEXT NOT NULL,
            last_mentioned_at REAL NOT NULL,
            mention_count INTEGER NOT NULL DEFAULT 1,
            active INTEGER NOT NULL DEFAULT 1
        );
        CREATE INDEX IF NOT EXISTS idx_conv_ctx_active
            ON conversation_context(active, last_mentioned_at DESC);

        CREATE TABLE IF NOT EXISTS active_entities (
            entity_name TEXT PRIMARY KEY,
            last_mentioned_at REAL NOT NULL,
            mention_count INTEGER NOT NULL DEFAULT 1,
            domains TEXT NOT NULL DEFAULT '[]'
        );

        CREATE TABLE IF NOT EXISTS consolidation_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            original_rid TEXT NOT NULL,
            replacement_rid TEXT,
            consolidation_type TEXT NOT NULL,
            reason TEXT NOT NULL,
            created_at REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_consol_orig
            ON consolidation_log(original_rid);

        CREATE TABLE IF NOT EXISTS reference_freshness (
            ref_id TEXT PRIMARY KEY,
            freshness_score REAL NOT NULL DEFAULT 1.0,
            auto_detected INTEGER NOT NULL DEFAULT 0,
            positive_reactions INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS memory_tiers (
            rid TEXT PRIMARY KEY,
            importance_tier TEXT NOT NULL,
            assigned_half_life REAL NOT NULL,
            last_reviewed_at REAL NOT NULL
        );

        CREATE TABLE IF NOT EXISTS evolution_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            last_consolidation_at REAL NOT NULL DEFAULT 0.0,
            last_pruning_at REAL NOT NULL DEFAULT 0.0,
            last_reference_decay_at REAL NOT NULL DEFAULT 0.0,
            total_consolidations INTEGER NOT NULL DEFAULT 0,
            total_pruned INTEGER NOT NULL DEFAULT 0
        );
        INSERT OR IGNORE INTO evolution_state (id) VALUES (1);",
    )
    .unwrap_or_else(|e| tracing::warn!("Failed to create evolution tables: {e}"));
}

// ── Types ───────────────────────────────────────────────────────────────────

/// Importance tier for a memory — determines half-life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportanceTier {
    /// Identity facts, key relationships — 365 days.
    Core,
    /// Important events, emotional memories — 60 days.
    Significant,
    /// Default bucket — 7 days.
    Routine,
    /// Low-importance, system-generated — 1 day.
    Ephemeral,
}

impl ImportanceTier {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Significant => "significant",
            Self::Routine => "routine",
            Self::Ephemeral => "ephemeral",
        }
    }

    pub fn half_life(&self, config: &MemoryEvolutionConfig) -> f64 {
        match self {
            Self::Core => config.tier_half_lives.core,
            Self::Significant => config.tier_half_lives.significant,
            Self::Routine => config.tier_half_lives.routine,
            Self::Ephemeral => config.tier_half_lives.ephemeral,
        }
    }
}

/// Result of smart multi-signal recall.
pub struct SmartRecallResult {
    pub primary: Vec<RecallResult>,
    pub context: Vec<RecallResult>,
    pub cross_domain: Vec<RecallResult>,
    pub confidence: f64,
    pub hint: Option<String>,
}

impl SmartRecallResult {
    /// Create from a single primary recall (backward compat fallback).
    pub fn from_primary(memories: Vec<RecallResult>) -> Self {
        let (confidence, hint) = compute_recall_confidence(&memories);
        Self {
            primary: memories,
            context: Vec::new(),
            cross_domain: Vec::new(),
            confidence,
            hint,
        }
    }

    /// Deduplicated union of all recall results, sorted by score descending.
    pub fn all_unique(&self) -> Vec<RecallResult> {
        let mut seen_texts = HashSet::new();
        let mut all = Vec::new();

        for mem in self
            .primary
            .iter()
            .chain(self.context.iter())
            .chain(self.cross_domain.iter())
        {
            // Deduplicate by first 80 chars of text (handles minor variations)
            let key: String = mem.text.chars().take(80).collect();
            if seen_texts.insert(key) {
                all.push(mem.clone());
            }
        }

        all.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        all
    }
}

// ── Domain filtering (V25) ──────────────────────────────────────────────────

/// Check if a memory domain should be excluded from primary recall results.
///
/// Audit logs and system domains pollute recall with operational noise.
/// Self-reflections have their own recall path (Step 3 introspect).
fn is_excluded_domain(domain: &str) -> bool {
    domain.starts_with("audit/")
        || domain.starts_with("system/")
        || domain == "self-reflection"
}

/// Filter out excluded domains from recall results, preserving order.
fn filter_recall_results(results: Vec<RecallResult>) -> Vec<RecallResult> {
    results.into_iter()
        .filter(|r| !is_excluded_domain(&r.domain))
        .collect()
}

// ── Gap 1: Smart Multi-Signal Recall ────────────────────────────────────────

/// Multi-signal recall — replaces single `recall_text(user_text, 5)`.
///
/// Issues 2-3 recall calls: primary query + active topic + active entities.
/// No LLM calls. Fast.
/// V25: Filters out audit/system/self-reflection domains from results.
pub fn smart_recall(
    db: &YantrikDB,
    user_text: &str,
    config: &MemoryEvolutionConfig,
) -> SmartRecallResult {
    // 1. Primary: direct query match (over-fetch by 2 to compensate for filtering)
    let primary = filter_recall_results(
        db.recall_text(user_text, 6).unwrap_or_default()
    ).into_iter().take(4).collect::<Vec<_>>();

    let mut context_memories = Vec::new();
    let mut cross_domain_memories = Vec::new();
    let mut extra_calls = 0;

    // 2. Topic continuity: recall by active conversation topic
    if extra_calls < config.max_extra_recall_calls {
        if let Some(topic) = get_active_topic(db.conn()) {
            // Only if topic differs substantially from user query
            if !text_overlap(&topic, user_text) {
                let topic_results = filter_recall_results(
                    db.recall_text(&topic, 5).unwrap_or_default()
                ).into_iter().take(3).collect::<Vec<_>>();
                context_memories.extend(topic_results);
                extra_calls += 1;
            }
        }
    }

    // 3. Entity bridging: recall by recently-mentioned entities across domains
    if config.cross_domain_enabled && extra_calls < config.max_extra_recall_calls {
        let entities = get_active_entities(db.conn(), 2);
        for entity in &entities {
            if extra_calls >= config.max_extra_recall_calls {
                break;
            }
            let entity_results = filter_recall_results(
                db.recall_text(entity, 4).unwrap_or_default()
            ).into_iter().take(2).collect::<Vec<_>>();
            cross_domain_memories.extend(entity_results);
            extra_calls += 1;
        }
    }

    // Compute confidence from primary results (most relevant signal)
    let (confidence, hint) = compute_recall_confidence(&primary);

    let mut result = SmartRecallResult {
        primary,
        context: context_memories,
        cross_domain: cross_domain_memories,
        confidence,
        hint,
    };

    // Trim total results to max
    let all = result.all_unique();
    let max = config.max_recall_memories;
    if all.len() > max {
        // Re-split: primary gets priority, rest is bonus
        result.primary = all[..max.min(all.len())].to_vec();
        result.context = Vec::new();
        result.cross_domain = Vec::new();
    }

    tracing::debug!(
        primary = result.primary.len(),
        context = result.context.len(),
        cross_domain = result.cross_domain.len(),
        confidence = format!("{:.2}", result.confidence),
        "Smart recall complete"
    );

    result
}

/// Update conversation context tables after an exchange.
///
/// Extracts significant words from user text and memory domains,
/// updates active_entities and conversation_context tables.
pub fn update_conversation_context(
    conn: &Connection,
    user_text: &str,
    memories: &[RecallResult],
) {
    let now = now_ts();

    // Extract significant words from user text (>4 chars, not stop words)
    let user_topics = extract_significant_words(user_text);

    // Update conversation_context with user topics
    for topic in &user_topics {
        let _ = conn.execute(
            "INSERT INTO conversation_context (topic_text, last_mentioned_at, mention_count, active)
             VALUES (?1, ?2, 1, 1)
             ON CONFLICT(topic_text) DO UPDATE SET
                 last_mentioned_at = ?2,
                 mention_count = mention_count + 1,
                 active = 1",
            rusqlite::params![topic, now],
        );
    }

    // Extract entities from recalled memories and update active_entities
    let mut entity_domains: HashMap<String, HashSet<String>> = HashMap::new();
    for mem in memories {
        // Use domain as entity context
        if !mem.domain.is_empty() && mem.domain != "general" {
            // Extract significant words from memory text as entity candidates
            let words = extract_significant_words(&mem.text);
            for word in words {
                entity_domains
                    .entry(word)
                    .or_default()
                    .insert(mem.domain.clone());
            }
        }
    }

    for (entity, domains) in &entity_domains {
        let domains_json = serde_json::to_string(&domains.iter().collect::<Vec<_>>())
            .unwrap_or_else(|_| "[]".to_string());

        // Merge with existing domains
        let existing: String = conn
            .query_row(
                "SELECT domains FROM active_entities WHERE entity_name = ?1",
                [entity.as_str()],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "[]".to_string());

        let mut all_domains: HashSet<String> = serde_json::from_str(&existing).unwrap_or_default();
        all_domains.extend(domains.iter().cloned());
        let merged = serde_json::to_string(&all_domains.iter().collect::<Vec<_>>())
            .unwrap_or_else(|_| domains_json);

        let _ = conn.execute(
            "INSERT INTO active_entities (entity_name, last_mentioned_at, mention_count, domains)
             VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(entity_name) DO UPDATE SET
                 last_mentioned_at = ?2,
                 mention_count = mention_count + 1,
                 domains = ?3",
            rusqlite::params![entity, now, merged],
        );
    }
}

/// Expire stale conversation topics (inactive > 30 minutes).
pub fn expire_stale_context(conn: &Connection) {
    let cutoff = now_ts() - 1800.0; // 30 minutes
    let _ = conn.execute(
        "UPDATE conversation_context SET active = 0 WHERE last_mentioned_at < ?1 AND active = 1",
        [cutoff],
    );
}

// ── Gap 3: Semantic Drift Correction ────────────────────────────────────────

const CONSOLIDATION_PROMPT: &str = r#"You are a memory consolidation assistant. Given two memories about similar topics, decide what to do.

Output valid JSON:
- action: "update" | "merge" | "keep_both"
- merged_text: (only if action is "update" or "merge") the new consolidated text (1-2 sentences max)
- reason: brief explanation (1 sentence)

Rules:
- "update": newer memory supersedes older (e.g., "lives in NYC" → "moved to SF"). Keep only the newer fact.
- "merge": complementary info about same topic. Combine into one richer memory.
- "keep_both": genuinely different memories that happen to share keywords."#;

/// Check if consolidation is due.
pub fn should_consolidate(conn: &Connection, config: &MemoryEvolutionConfig) -> bool {
    let last: f64 = conn
        .query_row(
            "SELECT last_consolidation_at FROM evolution_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0.0);

    now_ts() - last > config.consolidation_interval_hours * 3600.0
}

/// Run semantic drift correction — merge/update similar memories.
///
/// Queries recent semantic memories, finds similar older ones, uses LLM to decide
/// whether to merge, update, or keep both. Tombstones superseded memories.
pub fn run_consolidation(
    db: &YantrikDB,
    llm: &dyn LLMBackend,
    config: &MemoryEvolutionConfig,
) {
    tracing::info!("Starting memory consolidation cycle");
    let conn = db.conn();

    // Find recent semantic memories (last 24h, up to 30)
    let cutoff = now_ts() - 86400.0;
    let recent: Vec<(String, String, String)> = {
        let mut stmt = match conn.prepare(
            "SELECT rid, text, domain FROM memories
             WHERE type = 'semantic'
               AND consolidation_status IS NULL OR consolidation_status = 'active'
               AND created_at > ?1
             ORDER BY created_at DESC LIMIT 30",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };

        stmt.query_map([cutoff], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    };

    if recent.is_empty() {
        update_consolidation_time(conn);
        return;
    }

    let mut merged_count = 0u32;
    let mut updated_count = 0u32;
    let threshold = config.duplicate_similarity_threshold;

    for (rid, text, domain) in &recent {
        // Find similar older memories
        let similar = match db.recall_text(text, 5) {
            Ok(s) => s,
            Err(_) => continue,
        };

        for candidate in &similar {
            // Skip self
            if candidate.text == *text {
                continue;
            }
            // Skip if similarity below threshold
            if candidate.scores.similarity < threshold {
                continue;
            }

            // Ask LLM to decide
            let messages = vec![
                ChatMessage::system(CONSOLIDATION_PROMPT),
                ChatMessage::user(format!(
                    "Memory A (older): {}\nMemory B (newer): {}",
                    candidate.text, text
                )),
            ];

            let gen = GenerationConfig {
                max_tokens: 128,
                temperature: 0.0,
                top_p: None,
                ..Default::default()
            };

            let response = match llm.chat(&messages, &gen) {
                Ok(r) => r,
                Err(_) => continue,
            };

            let json_str = extract_json(&response.text);
            let parsed: serde_json::Value = match serde_json::from_str(&json_str) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let action = parsed
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("keep_both");
            let merged_text = parsed
                .get("merged_text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let reason = parsed
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("LLM decision");

            match action {
                "update" => {
                    // Newer supersedes older — tombstone the older memory
                    if let Some(safe_text) = sanitize::validate_memory_text(merged_text) {
                        tombstone_memory(conn, &candidate.text, rid, "fact_update", reason);
                        // Record updated memory with the new text
                        let _ = db.record_text(
                            &safe_text, "semantic", 0.7, 0.0, 604800.0,
                            &serde_json::json!({"consolidated": true}),
                            "default", 0.9, domain, "companion", None,
                        );
                        updated_count += 1;
                    }
                }
                "merge" => {
                    if let Some(safe_text) = sanitize::validate_memory_text(merged_text) {
                        // Tombstone both originals
                        tombstone_memory(conn, &candidate.text, rid, "merge", reason);
                        tombstone_memory(conn, text, rid, "merge", reason);
                        // Record merged memory
                        let _ = db.record_text(
                            &safe_text, "semantic", 0.7, 0.0, 604800.0,
                            &serde_json::json!({"consolidated": true}),
                            "default", 0.9, domain, "companion", None,
                        );
                        merged_count += 1;
                    }
                }
                _ => {} // keep_both — no action
            }

            // Only process first similar match per memory to avoid cascading
            break;
        }
    }

    update_consolidation_time(conn);

    let _ = conn.execute(
        "UPDATE evolution_state SET total_consolidations = total_consolidations + ?1 WHERE id = 1",
        [merged_count + updated_count],
    );

    tracing::info!(
        merged = merged_count,
        updated = updated_count,
        checked = recent.len(),
        "Consolidation cycle complete"
    );
}

// ── Gap 4: Dynamic Shared References ────────────────────────────────────────

/// Decay freshness scores for all shared references.
pub fn decay_reference_freshness(conn: &Connection, config: &MemoryEvolutionConfig) {
    let now = now_ts();
    let half_life_secs = config.reference_freshness_half_life_days * 86400.0;

    // Ensure all existing shared references have freshness entries
    let _ = conn.execute(
        "INSERT OR IGNORE INTO reference_freshness (ref_id, freshness_score)
         SELECT ref_id, 1.0 FROM shared_references
         WHERE ref_id NOT IN (SELECT ref_id FROM reference_freshness)",
        [],
    );

    // Calculate decay: freshness = max(0.01, 2^(-elapsed_days / half_life_days))
    // We use last_used_at from shared_references if available
    let _ = conn.execute(
        "UPDATE reference_freshness SET freshness_score = MAX(0.01,
            (1.0 / (1.0 + ((?1 - COALESCE(
                (SELECT last_used_at FROM shared_references WHERE shared_references.ref_id = reference_freshness.ref_id),
                ?1
            )) / ?2)))
        )",
        rusqlite::params![now, half_life_secs],
    );
}

/// Auto-detect emerging shared themes from recent positive interactions.
///
/// Scans recent high-valence memories for recurring phrases, asks LLM if they
/// qualify as shared references, and auto-creates them.
pub fn detect_emerging_references(
    db: &YantrikDB,
    conn: &Connection,
    llm: &dyn LLMBackend,
) {
    // Get recent positive memories (last 7 days, valence > 0.3)
    let cutoff = now_ts() - 604800.0;
    let memories: Vec<String> = {
        let mut stmt = match conn.prepare(
            "SELECT text FROM memories
             WHERE valence > 0.3 AND created_at > ?1
             ORDER BY created_at DESC LIMIT 50",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };

        stmt.query_map([cutoff], |row| row.get::<_, String>(0))
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    };

    if memories.len() < 5 {
        return; // Not enough data for pattern detection
    }

    // Find recurring 2-3 word phrases (simple n-gram frequency)
    let mut phrase_counts: HashMap<String, usize> = HashMap::new();
    for text in &memories {
        let words: Vec<&str> = text.split_whitespace().collect();
        for window in words.windows(2) {
            let phrase = window.join(" ").to_lowercase();
            if phrase.len() > 6 {
                *phrase_counts.entry(phrase).or_insert(0) += 1;
            }
        }
        for window in words.windows(3) {
            let phrase = window.join(" ").to_lowercase();
            if phrase.len() > 10 {
                *phrase_counts.entry(phrase).or_insert(0) += 1;
            }
        }
    }

    // Filter to phrases appearing 3+ times
    let candidates: Vec<&String> = phrase_counts
        .iter()
        .filter(|(_, count)| **count >= 3)
        .map(|(phrase, _)| phrase)
        .take(3) // Max 3 candidates per cycle
        .collect();

    if candidates.is_empty() {
        return;
    }

    // Check existing references to avoid duplicates
    let existing: HashSet<String> = {
        let mut stmt = match conn.prepare("SELECT reference_text FROM shared_references") {
            Ok(s) => s,
            Err(_) => return,
        };
        stmt.query_map([], |row| row.get::<_, String>(0))
            .ok()
            .map(|rows| {
                rows.filter_map(|r| r.ok())
                    .map(|t| t.to_lowercase())
                    .collect()
            })
            .unwrap_or_default()
    };

    for candidate in &candidates {
        // Skip if already exists as a reference
        if existing.iter().any(|e| e.contains(candidate.as_str())) {
            continue;
        }

        // Ask LLM if this recurring theme is reference-worthy
        let messages = vec![
            ChatMessage::system(
                "You evaluate recurring conversation themes. \
                 Output JSON: {\"is_reference\": true/false, \"reference_text\": \"...\", \"context\": \"...\"}\n\
                 A shared reference is a recurring positive theme, inside joke, or shared interest \
                 that the user and companion frequently discuss. Not every repeated word qualifies.",
            ),
            ChatMessage::user(format!(
                "Recurring phrase: \"{}\"\nAppeared {} times in recent positive conversations.",
                candidate,
                phrase_counts[candidate.as_str()]
            )),
        ];

        let gen = GenerationConfig {
            max_tokens: 100,
            temperature: 0.0,
            top_p: None,
            ..Default::default()
        };

        let response = match llm.chat(&messages, &gen) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let json_str = extract_json(&response.text);
        let parsed: serde_json::Value = match serde_json::from_str(&json_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let is_ref = parsed
            .get("is_reference")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if is_ref {
            let ref_text = parsed
                .get("reference_text")
                .and_then(|v| v.as_str())
                .unwrap_or(candidate);
            let context = parsed
                .get("context")
                .and_then(|v| v.as_str())
                .unwrap_or("Auto-detected from recurring themes");

            if let Some(safe_text) = sanitize::validate_memory_text(ref_text) {
                let ref_id = Evolution::add_shared_reference(conn, &safe_text, context);
                // Mark as auto-detected in freshness table
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO reference_freshness (ref_id, freshness_score, auto_detected)
                     VALUES (?1, 1.0, 1)",
                    rusqlite::params![ref_id],
                );
                tracing::info!(phrase = candidate.as_str(), "Auto-created shared reference");
            }
        }
    }
}

/// Get shared references ranked by freshness.
pub fn get_fresh_references(conn: &Connection, limit: usize) -> Vec<SharedReference> {
    let mut stmt = match conn.prepare(
        "SELECT s.ref_id, s.reference_text, s.origin_context, s.times_used
         FROM shared_references s
         LEFT JOIN reference_freshness f ON s.ref_id = f.ref_id
         WHERE COALESCE(f.freshness_score, 1.0) > 0.1
         ORDER BY COALESCE(f.freshness_score, 1.0) DESC
         LIMIT ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Evolution::get_shared_references(conn, limit),
    };

    stmt.query_map([limit as i64], |row| {
        Ok(SharedReference {
            ref_id: row.get(0)?,
            reference_text: row.get(1)?,
            origin_context: row.get(2)?,
            times_used: row.get(3)?,
        })
    })
    .ok()
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_else(|| Evolution::get_shared_references(conn, limit))
}

// ── Gap 5: Variable Half-Lives + Pruning ────────────────────────────────────

/// Classify a memory into an importance tier.
/// V25: identity → always Core, preference → always Significant, audit → Ephemeral.
pub fn classify_tier(importance: f64, memory_type: &str, domain: &str) -> ImportanceTier {
    // V25: Domain-based tier overrides
    if domain == "identity" {
        return ImportanceTier::Core; // 365 days — who the user IS never fades
    }
    if domain == "preference" {
        return ImportanceTier::Significant; // 60 days — preferences evolve but matter
    }
    if domain.starts_with("audit/") || domain.starts_with("system/") {
        return ImportanceTier::Ephemeral; // 1 day — operational noise
    }

    // Core: high-importance semantic facts about family, health
    if importance >= 0.8 {
        return ImportanceTier::Core;
    }
    if memory_type == "semantic" && matches!(domain, "family" | "health") {
        return ImportanceTier::Core;
    }

    // Significant: moderate importance or emotionally weighted
    if importance >= 0.5 {
        return ImportanceTier::Significant;
    }

    // Ephemeral: very low importance
    if importance < 0.2 {
        return ImportanceTier::Ephemeral;
    }

    // Default: routine
    ImportanceTier::Routine
}

/// Assign an importance tier and variable half-life to a memory.
///
/// Called after extract_and_learn() records a new memory.
pub fn assign_memory_tier(
    conn: &Connection,
    rid: &str,
    importance: f64,
    memory_type: &str,
    domain: &str,
    config: &MemoryEvolutionConfig,
) {
    if !config.variable_halflife_enabled {
        return;
    }

    let tier = classify_tier(importance, memory_type, domain);
    let half_life = tier.half_life(config);
    let now = now_ts();

    // Update the memory's half-life directly
    let _ = conn.execute(
        "UPDATE memories SET half_life = ?1 WHERE rid = ?2",
        rusqlite::params![half_life, rid],
    );

    // Track in tier table
    let _ = conn.execute(
        "INSERT OR REPLACE INTO memory_tiers (rid, importance_tier, assigned_half_life, last_reviewed_at)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![rid, tier.name(), half_life, now],
    );

    tracing::debug!(
        rid = rid,
        tier = tier.name(),
        half_life_days = half_life / 86400.0,
        "Memory tier assigned"
    );
}

/// One-time migration: assign tiers to existing un-tiered memories.
pub fn backfill_tiers(conn: &Connection, config: &MemoryEvolutionConfig) {
    if !config.variable_halflife_enabled {
        return;
    }

    // Check if backfill is needed
    let tiered_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memory_tiers", [], |row| row.get(0))
        .unwrap_or(0);

    if tiered_count > 0 {
        return; // Already backfilled
    }

    let memories: Vec<(String, f64, String, String)> = {
        let mut stmt = match conn.prepare(
            "SELECT rid, importance, type, domain FROM memories
             WHERE consolidation_status IS NULL OR consolidation_status = 'active'
             LIMIT 500",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };

        stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    };

    if memories.is_empty() {
        return;
    }

    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (rid, importance, memory_type, domain) in &memories {
        let tier = classify_tier(*importance, memory_type, domain);
        let half_life = tier.half_life(config);
        let now = now_ts();

        let _ = conn.execute(
            "UPDATE memories SET half_life = ?1 WHERE rid = ?2",
            rusqlite::params![half_life, rid],
        );
        let _ = conn.execute(
            "INSERT OR REPLACE INTO memory_tiers (rid, importance_tier, assigned_half_life, last_reviewed_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![rid, tier.name(), half_life, now],
        );

        *counts.entry(tier.name()).or_insert(0) += 1;
    }

    tracing::info!(
        total = memories.len(),
        core = counts.get("core").unwrap_or(&0),
        significant = counts.get("significant").unwrap_or(&0),
        routine = counts.get("routine").unwrap_or(&0),
        ephemeral = counts.get("ephemeral").unwrap_or(&0),
        "Backfilled memory tiers"
    );
}

/// Check if pruning is due.
pub fn should_prune(conn: &Connection, config: &MemoryEvolutionConfig) -> bool {
    let last: f64 = conn
        .query_row(
            "SELECT last_pruning_at FROM evolution_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0.0);

    now_ts() - last > config.pruning_interval_hours * 3600.0
}

/// Run pruning cycle — compress stale low-importance memories into summaries.
pub fn run_pruning(
    db: &YantrikDB,
    llm: &dyn LLMBackend,
    config: &MemoryEvolutionConfig,
) {
    tracing::info!("Starting memory pruning cycle");
    let conn = db.conn();
    let cutoff = now_ts() - 30.0 * 86400.0; // 30 days ago

    // Find stale low-importance memories
    let stale: Vec<(String, String, String)> = {
        let mut stmt = match conn.prepare(
            "SELECT rid, text, domain FROM memories
             WHERE importance < 0.3
               AND (consolidation_status IS NULL OR consolidation_status = 'active')
               AND created_at < ?1
             ORDER BY domain, created_at
             LIMIT 100",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!("Pruning query failed: {e}");
                update_pruning_time(conn);
                return;
            }
        };

        stmt.query_map([cutoff], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    };

    if stale.is_empty() {
        update_pruning_time(conn);
        return;
    }

    // Group by domain
    let mut by_domain: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (rid, text, domain) in &stale {
        by_domain
            .entry(domain.clone())
            .or_default()
            .push((rid.clone(), text.clone()));
    }

    let mut pruned = 0u32;
    let mut summaries = 0u32;

    for (domain, memories) in &by_domain {
        if memories.len() < 3 {
            continue; // Not enough to summarize
        }

        // Build summary prompt
        let mem_texts: Vec<&str> = memories.iter().map(|(_, t)| t.as_str()).take(10).collect();
        let messages = vec![
            ChatMessage::system(
                "Summarize these related memories into 2-3 concise sentences. \
                 Preserve key facts and dates. Output only the summary text, no JSON.",
            ),
            ChatMessage::user(format!(
                "Domain: {domain}\nMemories:\n{}",
                mem_texts
                    .iter()
                    .enumerate()
                    .map(|(i, t)| format!("{}. {t}", i + 1))
                    .collect::<Vec<_>>()
                    .join("\n")
            )),
        ];

        let gen = GenerationConfig {
            max_tokens: 150,
            temperature: 0.0,
            top_p: None,
            ..Default::default()
        };

        let response = match llm.chat(&messages, &gen) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let summary = response.text.trim();
        if let Some(safe_summary) = sanitize::validate_memory_text(summary) {
            // Store summary with significant tier half-life
            let _ = db.record_text(
                &safe_summary,
                "semantic",
                0.4,
                0.0,
                config.tier_half_lives.significant,
                &serde_json::json!({"compressed": true, "source_count": memories.len()}),
                "default",
                0.85,
                domain,
                "companion",
                None,
            );
            summaries += 1;

            // Tombstone originals
            for (_, text) in memories.iter().take(10) {
                tombstone_memory(conn, text, "", "compress", "Compressed into summary");
                pruned += 1;
            }
        }
    }

    update_pruning_time(conn);

    let _ = conn.execute(
        "UPDATE evolution_state SET total_pruned = total_pruned + ?1 WHERE id = 1",
        [pruned],
    );

    tracing::info!(
        pruned = pruned,
        summaries = summaries,
        checked = stale.len(),
        "Pruning cycle complete"
    );
}

// ── Memory Graph Weaving (idle-time proactive linking) ──────────────────────

const LINK_DISCOVERY_PROMPT: &str = r#"You are a memory analyst. Given a memory and a list of potentially related memories, identify meaningful relationships.

Output valid JSON:
- links: array of {"target_index": number, "relationship": string, "confidence": 0.0-1.0}

Rules:
- Only create links where there's a genuine semantic relationship (causal, temporal, thematic, entity-based).
- relationship should be a short verb phrase: "caused_by", "leads_to", "same_topic_as", "contradicts", "supports", "involves_same_person", "happened_before", "happened_after", "related_to".
- confidence > 0.7 for strong links, 0.4-0.7 for moderate, skip below 0.4.
- Output empty links array if no real connections exist. Don't force links."#;

/// SQL tables for weaving state tracking.
pub fn ensure_weaving_tables(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS weaving_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            last_weave_at REAL NOT NULL DEFAULT 0.0,
            total_links_created INTEGER NOT NULL DEFAULT 0,
            total_weave_cycles INTEGER NOT NULL DEFAULT 0,
            last_memory_offset INTEGER NOT NULL DEFAULT 0
        );
        INSERT OR IGNORE INTO weaving_state (id) VALUES (1);

        CREATE TABLE IF NOT EXISTS weaving_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_rid TEXT NOT NULL,
            target_rid TEXT NOT NULL,
            relationship TEXT NOT NULL,
            confidence REAL NOT NULL,
            created_at REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_weave_log_source ON weaving_log(source_rid);
        CREATE INDEX IF NOT EXISTS idx_weave_log_target ON weaving_log(target_rid);",
    )
    .unwrap_or_else(|e| tracing::warn!("Failed to create weaving tables: {e}"));
}

/// Check if a weaving cycle should run (based on interval and idle time).
pub fn should_weave(conn: &Connection, config: &MemoryEvolutionConfig) -> bool {
    if !config.weaving_enabled {
        return false;
    }

    let last: f64 = conn
        .query_row(
            "SELECT last_weave_at FROM weaving_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0.0);

    now_ts() - last > config.weaving_interval_hours * 3600.0
}

/// Run a memory graph weaving cycle.
///
/// Scans a batch of memories, uses semantic recall to find related memories,
/// then uses LLM to identify genuine relationships. Creates entity links
/// via `db.relate()` for discovered connections.
///
/// Designed to run incrementally — processes a batch per cycle using a rotating
/// offset so it eventually covers the entire memory store.
pub fn run_weaving_cycle(
    db: &YantrikDB,
    llm: &dyn LLMBackend,
    config: &MemoryEvolutionConfig,
) {
    tracing::info!("Starting memory weaving cycle");
    let conn = db.conn();

    // Get current offset for incremental scanning
    let offset: i64 = conn
        .query_row(
            "SELECT last_memory_offset FROM weaving_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // Fetch a batch of memories to weave (skip ephemeral tier)
    let batch_size = config.weaving_batch_size as i64;
    let batch: Vec<(String, String, String, f64)> = {
        let mut stmt = match conn.prepare(
            "SELECT m.rid, m.text, m.domain, m.importance
             FROM memories m
             LEFT JOIN memory_tiers mt ON m.rid = mt.rid
             WHERE (mt.importance_tier IS NULL OR mt.importance_tier != 'ephemeral')
               AND m.half_life > 10.0
             ORDER BY m.created_at ASC
             LIMIT ?1 OFFSET ?2",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!("Weaving batch query failed: {e}");
                return;
            }
        };

        stmt.query_map(rusqlite::params![batch_size, offset], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
            ))
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    };

    if batch.is_empty() {
        // Wrap around — reset offset to 0 for next cycle
        let _ = conn.execute(
            "UPDATE weaving_state SET last_memory_offset = 0, last_weave_at = ?1 WHERE id = 1",
            [now_ts()],
        );
        tracing::debug!("Weaving: no memories at offset {offset}, wrapping around");
        return;
    }

    let mut total_links = 0u32;
    let gen_config = GenerationConfig {
        max_tokens: 256,
        temperature: 0.0,
        top_p: None,
        ..Default::default()
    };

    for (rid, text, domain, _importance) in &batch {
        // Find semantically related memories
        let related = match db.recall_text(text, 6) {
            Ok(r) => r,
            Err(_) => continue,
        };

        // Filter out self and very similar (those are consolidation's job)
        let candidates: Vec<&yantrikdb_core::types::RecallResult> = related
            .iter()
            .filter(|r| {
                r.text != *text
                    && r.scores.similarity < config.duplicate_similarity_threshold
                    && r.scores.similarity > 0.15 // too low = no real relationship
            })
            .take(4)
            .collect();

        if candidates.is_empty() {
            continue;
        }

        // Check if we already have links for this memory (avoid re-processing)
        let existing_links: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM weaving_log WHERE source_rid = ?1",
                [rid.as_str()],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if existing_links >= 3 {
            continue; // Already well-connected
        }

        // Ask LLM to identify relationships
        let candidate_list: String = candidates
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let sim = (r.scores.similarity * 100.0) as u32;
                format!("{i}. [{}% sim, domain: {}] {}", sim, r.domain, r.text)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let messages = vec![
            ChatMessage::system(LINK_DISCOVERY_PROMPT),
            ChatMessage::user(format!(
                "Source memory (domain: {domain}):\n{text}\n\nCandidate memories:\n{candidate_list}"
            )),
        ];

        let response = match llm.chat(&messages, &gen_config) {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("Weaving LLM call failed: {e}");
                continue;
            }
        };

        // Parse response
        let json_str = extract_json(&response.text);
        let parsed: serde_json::Value = match serde_json::from_str(&json_str) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let links = match parsed.get("links").and_then(|v| v.as_array()) {
            Some(l) => l,
            None => continue,
        };

        for link in links.iter().take(3) {
            let target_idx = match link.get("target_index").and_then(|v| v.as_u64()) {
                Some(i) => i as usize,
                None => continue,
            };
            let relationship = match link.get("relationship").and_then(|v| v.as_str()) {
                Some(r) => r,
                None => continue,
            };
            let confidence = link
                .get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5);

            if confidence < 0.4 || target_idx >= candidates.len() {
                continue;
            }

            // Validate relationship string
            if let Some(rel) = sanitize::validate_entity_field(relationship) {
                let target = &candidates[target_idx];

                // Create the entity link via db.relate()
                // Use memory text snippets as entity names (first 60 chars)
                let source_entity: String = text.chars().take(60).collect();
                let target_entity: String = target.text.chars().take(60).collect();

                match db.relate(&source_entity, &target_entity, rel, confidence) {
                    Ok(_edge_id) => {
                        total_links += 1;

                        // Log the weaving action
                        let _ = conn.execute(
                            "INSERT INTO weaving_log (source_rid, target_rid, relationship, confidence, created_at)
                             VALUES (?1, ?2, ?3, ?4, ?5)",
                            rusqlite::params![rid, target.text, rel, confidence, now_ts()],
                        );
                    }
                    Err(e) => {
                        tracing::debug!("Weaving relate failed: {e}");
                    }
                }
            }
        }
    }

    // Update state: advance offset, record cycle
    let new_offset = offset + batch_size;
    let _ = conn.execute(
        "UPDATE weaving_state SET
            last_weave_at = ?1,
            total_links_created = total_links_created + ?2,
            total_weave_cycles = total_weave_cycles + 1,
            last_memory_offset = ?3
         WHERE id = 1",
        rusqlite::params![now_ts(), total_links, new_offset],
    );

    tracing::info!(
        links_created = total_links,
        batch_size = batch.len(),
        offset = offset,
        "Memory weaving cycle complete"
    );
}

/// Get weaving stats for display/logging.
pub fn weaving_stats(conn: &Connection) -> (i64, i64) {
    conn.query_row(
        "SELECT total_links_created, total_weave_cycles FROM weaving_state WHERE id = 1",
        [],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )
    .unwrap_or((0, 0))
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Get the most active conversation topic.
fn get_active_topic(conn: &Connection) -> Option<String> {
    conn.query_row(
        "SELECT topic_text FROM conversation_context
         WHERE active = 1
         ORDER BY mention_count DESC, last_mentioned_at DESC
         LIMIT 1",
        [],
        |row| row.get(0),
    )
    .ok()
}

/// Get the most recently mentioned active entities.
fn get_active_entities(conn: &Connection, limit: usize) -> Vec<String> {
    let mut stmt = match conn.prepare(
        "SELECT entity_name FROM active_entities
         ORDER BY last_mentioned_at DESC
         LIMIT ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    stmt.query_map([limit as i64], |row| row.get::<_, String>(0))
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// Extract significant words from text (>4 chars, not stop words).
fn extract_significant_words(text: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "about", "after", "again", "being", "below", "between", "could",
        "doing", "during", "every", "first", "found", "going", "great",
        "have", "here", "just", "know", "like", "make", "many", "more",
        "much", "need", "only", "other", "over", "really", "right",
        "said", "same", "should", "since", "some", "still", "such",
        "take", "tell", "than", "that", "their", "them", "then",
        "there", "these", "they", "thing", "think", "this", "those",
        "time", "very", "want", "well", "were", "what", "when",
        "where", "which", "while", "will", "with", "would", "your",
    ];

    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| w.len() > 4 && !STOP_WORDS.contains(&w.as_str()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

/// Check if two text strings have significant overlap (>40% shared words).
fn text_overlap(a: &str, b: &str) -> bool {
    let words_a: HashSet<String> = a
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() > 3)
        .collect();
    let words_b: HashSet<String> = b
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() > 3)
        .collect();

    if words_a.is_empty() || words_b.is_empty() {
        return false;
    }

    let overlap = words_a.intersection(&words_b).count();
    let min_len = words_a.len().min(words_b.len());
    overlap as f64 / min_len as f64 > 0.4
}

/// Tombstone a memory — set extreme decay so it vanishes from recall.
fn tombstone_memory(conn: &Connection, text: &str, replacement_rid: &str, ctype: &str, reason: &str) {
    // Set half_life to 1 second — effectively removes from recall
    let _ = conn.execute(
        "UPDATE memories SET half_life = 1.0 WHERE text = ?1",
        [text],
    );

    // Log the consolidation action
    let _ = conn.execute(
        "INSERT INTO consolidation_log (original_rid, replacement_rid, consolidation_type, reason, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![text, replacement_rid, ctype, reason, now_ts()],
    );
}

fn update_consolidation_time(conn: &Connection) {
    let _ = conn.execute(
        "UPDATE evolution_state SET last_consolidation_at = ?1 WHERE id = 1",
        [now_ts()],
    );
}

fn update_pruning_time(conn: &Connection) {
    let _ = conn.execute(
        "UPDATE evolution_state SET last_pruning_at = ?1 WHERE id = 1",
        [now_ts()],
    );
}

/// Extract JSON from LLM response (handles markdown code block wrapping).
fn extract_json(text: &str) -> String {
    let text = text.trim();
    if text.starts_with("```") {
        text.lines()
            .skip(1)
            .take_while(|l| !l.starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.to_string()
    }
}

/// Compute recall confidence from result scores.
fn compute_recall_confidence(
    memories: &[RecallResult],
) -> (f64, Option<String>) {
    if memories.is_empty() {
        return (0.0, Some("You have no relevant memories for this topic.".into()));
    }

    let n = memories.len() as f64;
    let avg_sim = memories.iter().map(|r| r.scores.similarity).sum::<f64>() / n;
    let best_sim = memories
        .iter()
        .map(|r| r.scores.similarity)
        .fold(0.0_f64, f64::max);

    let worst_score = memories.iter().map(|r| r.score).fold(f64::MAX, f64::min);
    let best_score = memories.iter().map(|r| r.score).fold(0.0_f64, f64::max);
    let gap_penalty = if best_score > 0.0 {
        ((best_score - worst_score) / best_score).min(1.0)
    } else {
        1.0
    };

    let confidence =
        (0.40 * avg_sim + 0.35 * best_sim + 0.25 * (1.0 - gap_penalty)).clamp(0.0, 1.0);

    let hint = if confidence < 0.3 {
        Some(
            "Your memory match is very weak — ask clarifying questions \
             to understand what the user means."
                .into(),
        )
    } else if confidence < 0.5 {
        Some(
            "Your memory match is uncertain — mention what you do remember \
             and ask if that's what they mean."
                .into(),
        )
    } else {
        None
    };

    (confidence, hint)
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
