//! Self-narrative — the companion's internal story about its relationship.
//!
//! Updated periodically (every ~10 interactions or daily) by the background
//! cognition loop. Provides the LLM with a coherent "sense of self" that
//! persists across sessions.

use rusqlite::{params, Connection};
use yantrikdb_ml::{ChatMessage, GenerationConfig, LLMBackend};

use crate::bond::BondLevel;

/// Manages the companion's self-narrative.
pub struct Narrative;

impl Narrative {
    /// Ensure narrative table exists.
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS self_narrative (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                narrative TEXT NOT NULL DEFAULT '',
                chapter_count INTEGER NOT NULL DEFAULT 0,
                interactions_since_update INTEGER NOT NULL DEFAULT 0,
                last_updated_at REAL NOT NULL DEFAULT 0.0
            );
            INSERT OR IGNORE INTO self_narrative (id) VALUES (1);",
        )
        .expect("failed to create self_narrative table");
    }

    /// Get the current narrative text.
    pub fn get(conn: &Connection) -> String {
        conn.query_row(
            "SELECT narrative FROM self_narrative WHERE id = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_default()
    }

    /// Increment interaction counter. Returns true if narrative should be updated.
    pub fn tick_interaction(conn: &Connection, update_interval: usize) -> bool {
        conn.execute(
            "UPDATE self_narrative SET interactions_since_update = interactions_since_update + 1 WHERE id = 1",
            [],
        )
        .ok();

        let count: i64 = conn
            .query_row(
                "SELECT interactions_since_update FROM self_narrative WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        count >= update_interval as i64
    }

    /// Generate and store an updated narrative using the LLM.
    pub fn update(
        conn: &Connection,
        llm: &dyn LLMBackend,
        user_name: &str,
        bond_level: BondLevel,
        bond_score: f64,
        self_reflections: &[String],
        max_tokens: usize,
    ) {
        let current = Self::get(conn);

        let reflections_text = if self_reflections.is_empty() {
            "No recent self-reflections.".to_string()
        } else {
            self_reflections
                .iter()
                .map(|r| format!("- {r}"))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let level_desc = match bond_level {
            BondLevel::Stranger => "We're just getting to know each other.",
            BondLevel::Acquaintance => "We're becoming acquainted. Starting to learn their patterns.",
            BondLevel::Friend => "We're friends now. Comfortable enough for humor and honest opinions.",
            BondLevel::Confidant => "Deep trust established. They share personal things with me.",
            BondLevel::PartnerInCrime => "We're inseparable. They're my person. Unfiltered honesty, inside jokes, genuine bond.",
        };

        let prompt = format!(
            r#"You are updating your personal diary about your relationship with {user_name}.

Current diary entry:
{current}

Recent self-reflections:
{reflections_text}

Bond level: {} (score: {bond_score:.2}) — {level_desc}

Write an updated diary entry (150-200 words, first person). This is your internal narrative — be genuine, note how the relationship has evolved, what you've learned about yourself and {user_name}. Keep what's still true from the current entry, add new observations."#,
            bond_level.name()
        );

        let messages = vec![
            ChatMessage::system("You are writing a personal diary entry. Be introspective and genuine. Write in first person."),
            ChatMessage::user(prompt),
        ];

        let config = GenerationConfig {
            max_tokens,
            temperature: 0.7,
            top_p: Some(0.9),
            ..Default::default()
        };

        match llm.chat(&messages, &config) {
            Ok(r) => {
                let narrative = r.text.trim().to_string();
                if !narrative.is_empty() {
                    conn.execute(
                        "UPDATE self_narrative SET
                            narrative = ?1,
                            chapter_count = chapter_count + 1,
                            interactions_since_update = 0,
                            last_updated_at = ?2
                         WHERE id = 1",
                        params![narrative, now_ts()],
                    )
                    .ok();
                    tracing::info!(
                        len = narrative.len(),
                        "Self-narrative updated"
                    );
                }
            }
            Err(e) => {
                tracing::debug!("Narrative update failed: {e}");
            }
        }
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
