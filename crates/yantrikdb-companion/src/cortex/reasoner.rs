//! LLM Reasoner — periodic deep reflection on the entity graph.
//!
//! Unlike baselines (statistics) and patterns (association rules),
//! the reasoner uses the LLM to find insights that no heuristic can.
//!
//! Runs every few hours (not every think cycle). Generates an EXECUTE
//! urge with a structured prompt containing the entity graph snapshot.
//!
//! Examples of things only an LLM can catch:
//! - "Sarah mentioned budget concerns in her last 3 emails — might affect your proposal"
//! - "You've been switching between 4 projects rapidly — might want to focus"
//! - "The client meeting is tomorrow but the deliverable ticket is still in 'To Do'"
//!
//! The reasoner doesn't call the LLM directly — it produces an EXECUTE
//! urge that flows through the existing pipeline (synthesis gate, etc.)

use rusqlite::{params, Connection};

use super::focus::FocusContext;
use super::schema;

// ── Reasoner ─────────────────────────────────────────────────────────

pub struct LlmReasoner {
    /// Last time the reasoner ran (Unix timestamp).
    last_run_ts: f64,
    /// Minimum interval between runs (seconds). Default: 4 hours.
    interval_secs: f64,
}

impl LlmReasoner {
    pub fn new() -> Self {
        Self {
            last_run_ts: 0.0,
            interval_secs: 4.0 * 3600.0,
        }
    }

    /// Check if it's time to run the reasoner.
    pub fn should_run(&self) -> bool {
        let now = now_ts();
        now - self.last_run_ts >= self.interval_secs
    }

    /// Build a deep reflection prompt from the entity graph.
    ///
    /// Returns an EXECUTE instruction string if there's enough data
    /// to reason about. Returns None if the graph is too sparse.
    pub fn build_reflection_prompt(
        &mut self,
        conn: &Connection,
        focus: Option<&FocusContext>,
    ) -> Option<String> {
        let now = now_ts();

        // Don't run too frequently
        if now - self.last_run_ts < self.interval_secs {
            return None;
        }
        self.last_run_ts = now;

        // Gather entity graph snapshot
        let entities = schema::get_relevant_entities(conn, 0.1, 30);
        if entities.len() < 3 {
            return None; // Not enough data to reason about
        }

        // Build entity summary
        let mut entity_lines = Vec::new();
        for e in &entities {
            let rels = schema::get_relationships(conn, &e.id);
            let rel_summary: Vec<String> = rels
                .iter()
                .take(5)
                .map(|r| {
                    let other = if r.source_id == e.id {
                        &r.target_id
                    } else {
                        &r.source_id
                    };
                    format!("  {} → {} ({}x)", r.rel_type, other, r.pulse_count)
                })
                .collect();

            let recent_pulses = schema::get_entity_pulses(conn, &e.id, 3);
            let pulse_summary: Vec<String> = recent_pulses
                .iter()
                .map(|p| format!("  [{}] {}", p.event_type, p.summary))
                .collect();

            entity_lines.push(format!(
                "• {} ({}, relevance={:.2})\n{}\n{}",
                e.display_name,
                e.entity_type,
                e.relevance,
                if rel_summary.is_empty() {
                    "  (no relationships)".to_string()
                } else {
                    rel_summary.join("\n")
                },
                if pulse_summary.is_empty() {
                    "  (no recent activity)".to_string()
                } else {
                    pulse_summary.join("\n")
                },
            ));
        }

        // Build learned patterns summary
        let patterns = get_top_patterns(conn, 10);
        let pattern_lines: Vec<String> = patterns
            .iter()
            .map(|(ante, cons, conf, sup)| {
                format!(
                    "• After {}, usually {} ({:.0}% confidence, {}x observed)",
                    ante, cons, conf * 100.0, sup
                )
            })
            .collect();

        // Build focus context
        let focus_text = if let Some(fc) = focus {
            format!(
                "Currently: {} on {} for {}min",
                fc.activity.as_str(),
                fc.active_project.as_deref().unwrap_or("unknown"),
                fc.duration_seconds / 60,
            )
        } else {
            "No focus data available.".to_string()
        };

        // Build the prompt
        let prompt = format!(
            "EXECUTE You are analyzing the user's cross-system activity graph. \
             Look for patterns, risks, conflicts, and opportunities that only \
             a reasoning AI would notice. Focus on actionable insights.\n\n\
             CURRENT FOCUS:\n{}\n\n\
             ENTITIES (people, tickets, projects, files in your awareness):\n{}\n\n\
             {}\
             INSTRUCTIONS:\n\
             - Find ONE insight that connects information across 2+ systems\n\
             - It should be something the user probably hasn't noticed\n\
             - Be specific and actionable, not vague\n\
             - If nothing interesting stands out, respond with exactly: nothing notable\n\
             - Keep it to 1-2 sentences, natural and conversational\n\
             - Do NOT list or enumerate — just share the insight naturally",
            focus_text,
            entity_lines.join("\n\n"),
            if pattern_lines.is_empty() {
                String::new()
            } else {
                format!(
                    "LEARNED PATTERNS:\n{}\n\n",
                    pattern_lines.join("\n")
                )
            },
        );

        Some(prompt)
    }
}

/// Get top patterns from the database.
fn get_top_patterns(conn: &Connection, limit: usize) -> Vec<(String, String, f64, i64)> {
    let mut stmt = match conn.prepare(
        "SELECT antecedent, consequent, confidence, support
         FROM cortex_patterns
         WHERE suppressed = 0 AND confidence >= 0.4
         ORDER BY confidence DESC, support DESC
         LIMIT ?1",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    stmt.query_map(params![limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })
    .ok()
    .map(|r| r.filter_map(|x| x.ok()).collect())
    .unwrap_or_default()
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
