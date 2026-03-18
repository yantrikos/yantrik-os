//! SQLite schema and queries for the Context Cortex.
//!
//! Four tables:
//! - `cortex_entities` — canonical cross-system identities
//! - `cortex_relationships` — persistent semantic relationships
//! - `cortex_pulses` — event stream from tool calls
//! - `cortex_pulse_entities` — junction table linking pulses to entities

use rusqlite::{params, Connection};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

use super::entity::{EntityType, ResolvedEntity, SystemSource};
use super::pulse::Pulse;

// ── Schema Creation ──────────────────────────────────────────────────

pub fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS cortex_entities (
            id              TEXT PRIMARY KEY,
            display_name    TEXT NOT NULL,
            entity_type     TEXT NOT NULL,
            system_aliases  TEXT NOT NULL DEFAULT '{}',
            first_seen_ts   REAL NOT NULL,
            last_seen_ts    REAL NOT NULL,
            relevance       REAL NOT NULL DEFAULT 0.5,
            attributes      TEXT NOT NULL DEFAULT '{}'
        );

        CREATE TABLE IF NOT EXISTS cortex_relationships (
            source_id       TEXT NOT NULL,
            target_id       TEXT NOT NULL,
            rel_type        TEXT NOT NULL,
            first_observed_ts REAL NOT NULL,
            last_observed_ts  REAL NOT NULL,
            pulse_count     INTEGER NOT NULL DEFAULT 1,
            metadata        TEXT NOT NULL DEFAULT '{}',
            PRIMARY KEY (source_id, target_id, rel_type)
        );

        CREATE TABLE IF NOT EXISTS cortex_pulses (
            id              INTEGER PRIMARY KEY,
            source          TEXT NOT NULL,
            event_type      TEXT NOT NULL,
            summary         TEXT NOT NULL,
            ts              REAL NOT NULL,
            metadata        TEXT NOT NULL DEFAULT '{}'
        );

        CREATE TABLE IF NOT EXISTS cortex_pulse_entities (
            pulse_id        INTEGER NOT NULL,
            entity_id       TEXT NOT NULL,
            role            TEXT NOT NULL,
            PRIMARY KEY (pulse_id, entity_id, role)
        );

        CREATE INDEX IF NOT EXISTS idx_cortex_pulses_ts
            ON cortex_pulses(ts DESC);
        CREATE INDEX IF NOT EXISTS idx_cortex_pulse_entities_entity
            ON cortex_pulse_entities(entity_id);
        CREATE INDEX IF NOT EXISTS idx_cortex_relationships_source
            ON cortex_relationships(source_id);
        CREATE INDEX IF NOT EXISTS idx_cortex_relationships_target
            ON cortex_relationships(target_id);
        CREATE INDEX IF NOT EXISTS idx_cortex_entities_type
            ON cortex_entities(entity_type);
        CREATE INDEX IF NOT EXISTS idx_cortex_entities_relevance
            ON cortex_entities(relevance DESC);

        -- ── Self-Learning Intelligence Tables ──

        -- Baselines: rolling averages for entity+metric pairs.
        -- e.g. entity='person:sarah', metric='emails_per_day', window='day'
        -- The system learns what's 'normal' and flags deviations.
        CREATE TABLE IF NOT EXISTS cortex_baselines (
            entity_id       TEXT NOT NULL,
            metric          TEXT NOT NULL,
            window          TEXT NOT NULL DEFAULT 'day',
            sample_count    INTEGER NOT NULL DEFAULT 0,
            running_mean    REAL NOT NULL DEFAULT 0.0,
            running_m2      REAL NOT NULL DEFAULT 0.0,
            last_value      REAL NOT NULL DEFAULT 0.0,
            last_updated_ts REAL NOT NULL DEFAULT 0.0,
            PRIMARY KEY (entity_id, metric, window)
        );

        -- Co-occurrence patterns: discovered temporal associations.
        -- e.g. 'When you view ticket:yos-142, you email person:sarah within 30min'
        CREATE TABLE IF NOT EXISTS cortex_patterns (
            id              INTEGER PRIMARY KEY,
            pattern_type    TEXT NOT NULL,
            antecedent      TEXT NOT NULL,
            consequent      TEXT NOT NULL,
            time_window_sec REAL NOT NULL DEFAULT 1800.0,
            support         INTEGER NOT NULL DEFAULT 0,
            confidence      REAL NOT NULL DEFAULT 0.0,
            last_observed_ts REAL NOT NULL,
            discovered_ts   REAL NOT NULL,
            suppressed      INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_cortex_patterns_type
            ON cortex_patterns(pattern_type);

        -- Learned rules: dynamic rules generated from baselines + patterns.
        -- Replaces hardcoded rules over time.
        CREATE TABLE IF NOT EXISTS cortex_learned_rules (
            id              INTEGER PRIMARY KEY,
            source          TEXT NOT NULL,
            rule_text       TEXT NOT NULL,
            priority        REAL NOT NULL DEFAULT 0.5,
            fire_count      INTEGER NOT NULL DEFAULT 0,
            dismiss_count   INTEGER NOT NULL DEFAULT 0,
            last_fired_ts   REAL NOT NULL DEFAULT 0.0,
            cooldown_secs   REAL NOT NULL DEFAULT 3600.0,
            created_ts      REAL NOT NULL,
            active          INTEGER NOT NULL DEFAULT 1
        );
        ",
    )?;
    Ok(())
}

// ── Entity Operations ────────────────────────────────────────────────

/// Upsert a canonical entity. Updates last_seen and merges aliases.
pub fn upsert_entity(
    conn: &Connection,
    id: &str,
    display_name: &str,
    entity_type: EntityType,
    source: SystemSource,
    external_id: &str,
) -> Result<()> {
    let now = now_ts();
    let type_str = entity_type.as_str();

    // Try insert
    let existing: Option<String> = conn
        .query_row(
            "SELECT system_aliases FROM cortex_entities WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .ok();

    if let Some(aliases_json) = existing {
        // Merge alias
        let mut aliases: serde_json::Value =
            serde_json::from_str(&aliases_json).unwrap_or(serde_json::json!({}));
        aliases[source.as_str()] = serde_json::Value::String(external_id.to_string());

        conn.execute(
            "UPDATE cortex_entities SET
                display_name = ?1, last_seen_ts = ?2, system_aliases = ?3,
                relevance = MIN(1.0, relevance + 0.1)
             WHERE id = ?4",
            params![display_name, now, aliases.to_string(), id],
        )?;
    } else {
        // Fresh insert
        let aliases = serde_json::json!({ source.as_str(): external_id });
        conn.execute(
            "INSERT INTO cortex_entities (id, display_name, entity_type, system_aliases, first_seen_ts, last_seen_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, display_name, type_str, aliases.to_string(), now],
        )?;
    }
    Ok(())
}

/// Boost relevance for an entity (capped at 1.0).
pub fn boost_relevance(conn: &Connection, entity_id: &str, delta: f64) {
    let _ = conn.execute(
        "UPDATE cortex_entities SET relevance = MIN(1.0, relevance + ?1), last_seen_ts = ?2 WHERE id = ?3",
        params![delta, now_ts(), entity_id],
    );
}

/// Decay all entity relevance scores (half-life ~24h).
///
/// Called once per think cycle. Multiplies relevance by 0.998 (~0.5 after 346 cycles = ~5.8h at 60s).
/// Adjusted: 0.9995 gives half-life ~1386 cycles = ~23h at 60s.
pub fn decay_relevance(conn: &Connection) {
    let _ = conn.execute(
        "UPDATE cortex_entities SET relevance = relevance * 0.9995 WHERE relevance > 0.01",
        [],
    );
}

/// Get entities above a relevance threshold, ordered by relevance.
pub fn get_relevant_entities(conn: &Connection, min_relevance: f64, limit: usize) -> Vec<EntityRow> {
    let mut stmt = conn
        .prepare(
            "SELECT id, display_name, entity_type, system_aliases, relevance, last_seen_ts, attributes
             FROM cortex_entities WHERE relevance >= ?1
             ORDER BY relevance DESC LIMIT ?2",
        )
        .unwrap();
    stmt.query_map(params![min_relevance, limit as i64], |row| {
        Ok(EntityRow {
            id: row.get(0)?,
            display_name: row.get(1)?,
            entity_type: row.get(2)?,
            system_aliases: row.get(3)?,
            relevance: row.get(4)?,
            last_seen_ts: row.get(5)?,
            attributes: row.get(6)?,
        })
    })
    .unwrap()
    .filter_map(|r| r.ok())
    .collect()
}

/// Find an entity by system alias.
pub fn find_entity_by_alias(conn: &Connection, source: SystemSource, external_id: &str) -> Option<String> {
    // Search in JSON: system_aliases contains the key-value pair
    let pattern = format!("\"{}\":\"{}\"", source.as_str(), external_id);
    conn.query_row(
        "SELECT id FROM cortex_entities WHERE system_aliases LIKE ?1",
        params![format!("%{}%", pattern)],
        |row| row.get(0),
    )
    .ok()
}

/// Find an entity by type and display name (fuzzy).
pub fn find_entity_by_name(conn: &Connection, entity_type: EntityType, name: &str) -> Option<String> {
    let normalized = name.trim().to_lowercase();
    conn.query_row(
        "SELECT id FROM cortex_entities WHERE entity_type = ?1 AND LOWER(display_name) = ?2",
        params![entity_type.as_str(), normalized],
        |row| row.get(0),
    )
    .ok()
}

// ── Pulse Operations ─────────────────────────────────────────────────

/// Store a pulse and its entity links.
pub fn store_pulse(
    conn: &Connection,
    pulse: &Pulse,
    resolved_entities: &[ResolvedEntity],
) -> Result<()> {
    conn.execute(
        "INSERT INTO cortex_pulses (source, event_type, summary, ts, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            pulse.source.as_str(),
            pulse.event_type.as_str(),
            pulse.summary,
            pulse.timestamp,
            pulse.metadata.to_string(),
        ],
    )?;

    let pulse_id = conn.last_insert_rowid();

    for re in resolved_entities {
        // Boost entity relevance on each pulse mention
        boost_relevance(conn, &re.entity_id, 0.15);

        conn.execute(
            "INSERT OR IGNORE INTO cortex_pulse_entities (pulse_id, entity_id, role)
             VALUES (?1, ?2, ?3)",
            params![pulse_id, re.entity_id, re.role],
        )?;
    }

    Ok(())
}

/// Derive relationships from a pulse's entity links.
///
/// If a pulse links entity A (role=actor) and entity B (role=target),
/// we create/update a relationship between them.
pub fn derive_relationships(
    conn: &Connection,
    pulse: &Pulse,
    resolved_entities: &[ResolvedEntity],
) -> Result<()> {
    let now = pulse.timestamp;

    // Find actor and target entities
    let actors: Vec<_> = resolved_entities.iter().filter(|e| e.role == "actor").collect();
    let targets: Vec<_> = resolved_entities.iter().filter(|e| e.role == "target").collect();

    // Determine relationship type from pulse event
    let rel_type = pulse.event_type.default_relationship();

    if let Some(rel) = rel_type {
        for actor in &actors {
            for target in &targets {
                conn.execute(
                    "INSERT INTO cortex_relationships (source_id, target_id, rel_type, first_observed_ts, last_observed_ts, pulse_count)
                     VALUES (?1, ?2, ?3, ?4, ?4, 1)
                     ON CONFLICT(source_id, target_id, rel_type) DO UPDATE SET
                         last_observed_ts = ?4,
                         pulse_count = pulse_count + 1",
                    params![actor.entity_id, target.entity_id, rel, now],
                )?;
            }
        }
    }

    // Context entities form weaker "mentions" relationships with actors/targets
    let contexts: Vec<_> = resolved_entities.iter().filter(|e| e.role == "context").collect();
    for ctx_entity in &contexts {
        for other in actors.iter().chain(targets.iter()) {
            conn.execute(
                "INSERT INTO cortex_relationships (source_id, target_id, rel_type, first_observed_ts, last_observed_ts, pulse_count)
                 VALUES (?1, ?2, 'mentions', ?3, ?3, 1)
                 ON CONFLICT(source_id, target_id, rel_type) DO UPDATE SET
                     last_observed_ts = ?3,
                     pulse_count = pulse_count + 1",
                params![ctx_entity.entity_id, other.entity_id, now],
            )?;
        }
    }

    Ok(())
}

// ── Relationship Queries ─────────────────────────────────────────────

/// Get all relationships involving an entity (as source or target).
pub fn get_relationships(conn: &Connection, entity_id: &str) -> Vec<RelationshipRow> {
    let mut stmt = conn
        .prepare(
            "SELECT source_id, target_id, rel_type, last_observed_ts, pulse_count
             FROM cortex_relationships
             WHERE source_id = ?1 OR target_id = ?1
             ORDER BY last_observed_ts DESC",
        )
        .unwrap();
    stmt.query_map(params![entity_id], |row| {
        Ok(RelationshipRow {
            source_id: row.get(0)?,
            target_id: row.get(1)?,
            rel_type: row.get(2)?,
            last_observed_ts: row.get(3)?,
            pulse_count: row.get(4)?,
        })
    })
    .unwrap()
    .filter_map(|r| r.ok())
    .collect()
}

/// Find entities related to a given entity with a specific relationship type.
pub fn find_related(
    conn: &Connection,
    entity_id: &str,
    rel_type: &str,
) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT target_id FROM cortex_relationships
             WHERE source_id = ?1 AND rel_type = ?2
             UNION
             SELECT source_id FROM cortex_relationships
             WHERE target_id = ?1 AND rel_type = ?2",
        )
        .unwrap();
    stmt.query_map(params![entity_id, rel_type], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
}

/// Count pulses of a given type for an entity in the last N seconds.
pub fn count_recent_pulses(
    conn: &Connection,
    entity_id: &str,
    event_type: &str,
    since_ts: f64,
) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM cortex_pulses p
         JOIN cortex_pulse_entities pe ON pe.pulse_id = p.id
         WHERE pe.entity_id = ?1 AND p.event_type = ?2 AND p.ts >= ?3",
        params![entity_id, event_type, since_ts],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

/// Get recent pulses for an entity, newest first.
pub fn get_entity_pulses(conn: &Connection, entity_id: &str, limit: usize) -> Vec<PulseRow> {
    let mut stmt = conn
        .prepare(
            "SELECT p.id, p.source, p.event_type, p.summary, p.ts
             FROM cortex_pulses p
             JOIN cortex_pulse_entities pe ON pe.pulse_id = p.id
             WHERE pe.entity_id = ?1
             ORDER BY p.ts DESC LIMIT ?2",
        )
        .unwrap();
    stmt.query_map(params![entity_id, limit as i64], |row| {
        Ok(PulseRow {
            id: row.get(0)?,
            source: row.get(1)?,
            event_type: row.get(2)?,
            summary: row.get(3)?,
            ts: row.get(4)?,
        })
    })
    .unwrap()
    .filter_map(|r| r.ok())
    .collect()
}

/// Prune old pulses older than `max_age_seconds`.
pub fn prune_old_pulses(conn: &Connection, max_age_seconds: f64) {
    let cutoff = now_ts() - max_age_seconds;
    let _ = conn.execute(
        "DELETE FROM cortex_pulse_entities WHERE pulse_id IN (
            SELECT id FROM cortex_pulses WHERE ts < ?1
         )",
        params![cutoff],
    );
    let _ = conn.execute("DELETE FROM cortex_pulses WHERE ts < ?1", params![cutoff]);
}

// ── Row Types ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct EntityRow {
    pub id: String,
    pub display_name: String,
    pub entity_type: String,
    pub system_aliases: String,
    pub relevance: f64,
    pub last_seen_ts: f64,
    pub attributes: String,
}

#[derive(Debug, Clone)]
pub struct RelationshipRow {
    pub source_id: String,
    pub target_id: String,
    pub rel_type: String,
    pub last_observed_ts: f64,
    pub pulse_count: i64,
}

#[derive(Debug, Clone)]
pub struct PulseRow {
    pub id: i64,
    pub source: String,
    pub event_type: String,
    pub summary: String,
    pub ts: f64,
}

// ── Helpers ──────────────────────────────────────────────────────────

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
