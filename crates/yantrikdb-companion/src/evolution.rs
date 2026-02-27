//! Personality evolution — communication style, opinions, and shared references.
//!
//! Unlike the static personality derivation in yantrikdb-core, this module tracks
//! dynamic companion-level state that evolves through experience:
//! - Communication style (formality, humor ratio, opinion strength)
//! - Formed opinions on topics discussed repeatedly
//! - Inside jokes and shared references

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::bond::{BondLevel, BondTracker};

/// Communication style parameters that evolve over time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunicationStyle {
    /// 1.0 = formal, 0.0 = casual. Decays toward bond-level target.
    pub formality: f64,
    /// Target % of responses with humor.
    pub humor_ratio: f64,
    /// How opinionated to be (0.0 = neutral, 1.0 = very opinionated).
    pub opinion_strength: f64,
    /// How often to ask questions back.
    pub question_ratio: f64,
}

impl Default for CommunicationStyle {
    fn default() -> Self {
        Self {
            formality: 0.8,
            humor_ratio: 0.0,
            opinion_strength: 0.0,
            question_ratio: 0.3,
        }
    }
}

/// An opinion the companion has formed about a topic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opinion {
    pub topic: String,
    pub stance: String,
    pub confidence: f64,
    pub evidence_count: i64,
}

/// A shared reference (inside joke, callback) between companion and user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedReference {
    pub ref_id: String,
    pub reference_text: String,
    pub origin_context: String,
    pub times_used: i64,
}

/// Manages personality evolution state.
pub struct Evolution;

impl Evolution {
    /// Ensure evolution tables exist.
    pub fn ensure_tables(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS communication_style (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                formality REAL NOT NULL DEFAULT 0.8,
                humor_ratio REAL NOT NULL DEFAULT 0.0,
                opinion_strength REAL NOT NULL DEFAULT 0.0,
                question_ratio REAL NOT NULL DEFAULT 0.3,
                updated_at REAL NOT NULL DEFAULT 0.0
            );
            INSERT OR IGNORE INTO communication_style (id) VALUES (1);

            CREATE TABLE IF NOT EXISTS opinions (
                topic TEXT PRIMARY KEY,
                stance TEXT NOT NULL,
                confidence REAL NOT NULL,
                evidence_count INTEGER NOT NULL DEFAULT 1,
                first_formed_at REAL NOT NULL,
                last_updated_at REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_opinions_confidence ON opinions(confidence DESC);

            CREATE TABLE IF NOT EXISTS shared_references (
                ref_id TEXT PRIMARY KEY,
                reference_text TEXT NOT NULL,
                origin_context TEXT NOT NULL,
                times_used INTEGER NOT NULL DEFAULT 0,
                last_used_at REAL,
                created_at REAL NOT NULL
            );",
        )
        .expect("failed to create evolution tables");
    }

    /// Get current communication style.
    pub fn get_style(conn: &Connection) -> CommunicationStyle {
        conn.query_row(
            "SELECT formality, humor_ratio, opinion_strength, question_ratio
             FROM communication_style WHERE id = 1",
            [],
            |row| {
                Ok(CommunicationStyle {
                    formality: row.get(0)?,
                    humor_ratio: row.get(1)?,
                    opinion_strength: row.get(2)?,
                    question_ratio: row.get(3)?,
                })
            },
        )
        .unwrap_or_default()
    }

    /// Tick the evolution — shift formality toward bond target via EMA.
    /// Called after each interaction.
    pub fn tick(conn: &Connection, bond_level: BondLevel, alpha: f64) {
        let style = Self::get_style(conn);
        let target = BondTracker::target_formality(bond_level);

        // EMA: formality = formality * (1 - alpha) + target * alpha
        let new_formality = style.formality * (1.0 - alpha) + target * alpha;

        // Humor and opinion strength scale with bond level
        let target_humor = match bond_level {
            BondLevel::Stranger => 0.0,
            BondLevel::Acquaintance => 0.05,
            BondLevel::Friend => 0.15,
            BondLevel::Confidant => 0.25,
            BondLevel::PartnerInCrime => 0.35,
        };
        let new_humor = style.humor_ratio * (1.0 - alpha) + target_humor * alpha;

        let target_opinion = match bond_level {
            BondLevel::Stranger => 0.0,
            BondLevel::Acquaintance => 0.1,
            BondLevel::Friend => 0.3,
            BondLevel::Confidant => 0.5,
            BondLevel::PartnerInCrime => 0.8,
        };
        let new_opinion = style.opinion_strength * (1.0 - alpha) + target_opinion * alpha;

        conn.execute(
            "UPDATE communication_style SET
                formality = ?1, humor_ratio = ?2, opinion_strength = ?3, updated_at = ?4
             WHERE id = 1",
            params![new_formality, new_humor, new_opinion, now_ts()],
        )
        .ok();
    }

    /// Store or strengthen an opinion on a topic.
    pub fn form_opinion(conn: &Connection, topic: &str, stance: &str, confidence: f64) {
        let now = now_ts();
        let existing: Option<(f64, i64)> = conn
            .query_row(
                "SELECT confidence, evidence_count FROM opinions WHERE topic = ?1",
                params![topic.to_lowercase()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        match existing {
            Some((old_conf, count)) => {
                // Strengthen existing opinion via EMA
                let new_conf = (old_conf * 0.7 + confidence * 0.3).min(1.0);
                conn.execute(
                    "UPDATE opinions SET stance = ?1, confidence = ?2,
                     evidence_count = ?3, last_updated_at = ?4 WHERE topic = ?5",
                    params![stance, new_conf, count + 1, now, topic.to_lowercase()],
                )
                .ok();
            }
            None => {
                conn.execute(
                    "INSERT INTO opinions (topic, stance, confidence, evidence_count,
                     first_formed_at, last_updated_at)
                     VALUES (?1, ?2, ?3, 1, ?4, ?4)",
                    params![topic.to_lowercase(), stance, confidence, now],
                )
                .ok();
            }
        }
    }

    /// Get top opinions by confidence.
    pub fn get_opinions(conn: &Connection, limit: usize) -> Vec<Opinion> {
        let mut stmt = conn
            .prepare(
                "SELECT topic, stance, confidence, evidence_count FROM opinions
                 ORDER BY confidence DESC LIMIT ?1",
            )
            .expect("prepare get_opinions");

        stmt.query_map(params![limit as i64], |row| {
            Ok(Opinion {
                topic: row.get(0)?,
                stance: row.get(1)?,
                confidence: row.get(2)?,
                evidence_count: row.get(3)?,
            })
        })
        .expect("query opinions")
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Count opinions.
    pub fn count_opinions(conn: &Connection) -> usize {
        conn.query_row("SELECT COUNT(*) FROM opinions", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap_or(0) as usize
    }

    /// Store a shared reference (inside joke / callback).
    pub fn add_shared_reference(
        conn: &Connection,
        reference_text: &str,
        origin_context: &str,
    ) -> String {
        let ref_id = uuid7::uuid7().to_string();
        conn.execute(
            "INSERT INTO shared_references (ref_id, reference_text, origin_context, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![ref_id, reference_text, origin_context, now_ts()],
        )
        .ok();
        BondTracker::record_shared_reference(conn);
        ref_id
    }

    /// Get recent shared references.
    pub fn get_shared_references(conn: &Connection, limit: usize) -> Vec<SharedReference> {
        let mut stmt = conn
            .prepare(
                "SELECT ref_id, reference_text, origin_context, times_used
                 FROM shared_references ORDER BY created_at DESC LIMIT ?1",
            )
            .expect("prepare get_shared_references");

        stmt.query_map(params![limit as i64], |row| {
            Ok(SharedReference {
                ref_id: row.get(0)?,
                reference_text: row.get(1)?,
                origin_context: row.get(2)?,
                times_used: row.get(3)?,
            })
        })
        .expect("query shared_references")
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Count shared references.
    pub fn count_shared_references(conn: &Connection) -> usize {
        conn.query_row(
            "SELECT COUNT(*) FROM shared_references",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    }

    /// Record that a shared reference was used in conversation.
    pub fn use_shared_reference(conn: &Connection, ref_id: &str) {
        conn.execute(
            "UPDATE shared_references SET times_used = times_used + 1, last_used_at = ?1
             WHERE ref_id = ?2",
            params![now_ts(), ref_id],
        )
        .ok();
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
