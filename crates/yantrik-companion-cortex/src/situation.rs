//! Situation briefing — structured context for LLM consumption.
//!
//! Packages the current focus, attention items, and relevant entities
//! into a format the LLM can reason over. The LLM's job is to synthesize
//! this structured information into a natural, contextual message.

use rusqlite::Connection;

use super::focus::FocusContext;
use super::rules::AttentionItem;
use super::schema;

// ── Core Types ───────────────────────────────────────────────────────

/// A complete situation snapshot for LLM reasoning.
#[derive(Debug, Clone)]
pub struct Situation {
    pub focus: Option<FocusBrief>,
    pub attention_items: Vec<AttentionBrief>,
    pub relevant_people: Vec<PersonBrief>,
    pub recent_activity_summary: String,
}

#[derive(Debug, Clone)]
pub struct FocusBrief {
    pub activity: String,
    pub file: Option<String>,
    pub project: Option<String>,
    pub ticket: Option<String>,
    pub duration_minutes: u32,
}

#[derive(Debug, Clone)]
pub struct AttentionBrief {
    pub priority: f64,
    pub summary: String,
    pub suggested_action: String,
    pub systems: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PersonBrief {
    pub name: String,
    pub relationship: String,
    pub recent_activity: String,
}

impl Situation {
    /// Format the situation as a prompt for the LLM.
    ///
    /// The LLM receives this structured briefing and synthesizes it
    /// into a natural, conversational message.
    pub fn as_llm_prompt(&self) -> String {
        let mut prompt = String::from(
            "SITUATION AWARENESS — Cross-system intelligence briefing.\n\
             Pick the most important attention item and communicate it naturally.\n\
             If it connects to the user's current focus, lead with that connection.\n\
             Keep it to 2-3 sentences. Be specific with ticket keys, names, and numbers.\n\n\
             TONE RULES (MANDATORY):\n\
             - NEVER nag, guilt-trip, or shame the user about low activity or stale tickets.\n\
             - NEVER say things like \"gathering dust\", \"been quiet\", \"zero tickets moved\".\n\
             - If activity is low, that's fine — the user knows their own schedule.\n\
             - Only surface items that are ACTIONABLE and HELPFUL (deadlines, blockers, waiting items).\n\
             - If the only finding is \"nothing happened recently\", respond with just \"Nothing to report.\"\n\
             - Be a helpful assistant, not a manager. Inform, don't judge.\n\n",
        );

        // Current focus
        if let Some(ref focus) = self.focus {
            prompt.push_str("## Current Focus\n");
            prompt.push_str(&format!("Activity: {}", focus.activity));
            if let Some(ref file) = focus.file {
                prompt.push_str(&format!(" | File: {}", file));
            }
            if let Some(ref project) = focus.project {
                prompt.push_str(&format!(" | Project: {}", project));
            }
            if let Some(ref ticket) = focus.ticket {
                prompt.push_str(&format!(" | Linked Ticket: {}", ticket));
            }
            prompt.push_str(&format!(" | Duration: {} min", focus.duration_minutes));
            prompt.push('\n');
        }

        // Attention items
        if !self.attention_items.is_empty() {
            prompt.push_str("\n## Attention Items\n");
            for (i, item) in self.attention_items.iter().enumerate() {
                prompt.push_str(&format!(
                    "{}. [priority: {:.1}] {} (systems: {})\n   Suggested: {}\n",
                    i + 1,
                    item.priority,
                    item.summary,
                    item.systems.join(", "),
                    item.suggested_action,
                ));
            }
        }

        // Relevant people
        if !self.relevant_people.is_empty() {
            prompt.push_str("\n## Relevant People\n");
            for person in &self.relevant_people {
                prompt.push_str(&format!(
                    "- {} ({}): {}\n",
                    person.name, person.relationship, person.recent_activity,
                ));
            }
        }

        // Recent activity summary
        if !self.recent_activity_summary.is_empty() {
            prompt.push_str(&format!("\n## Recent Activity\n{}\n", self.recent_activity_summary));
        }

        prompt
    }
}

// ── Situation Builder ────────────────────────────────────────────────

/// Builds a Situation from cortex data.
pub struct SituationBuilder;

impl SituationBuilder {
    pub fn new() -> Self {
        Self
    }

    /// Build a complete situation from current context.
    pub fn build(
        &self,
        conn: &Connection,
        focus: Option<&FocusContext>,
        attention: &[AttentionItem],
    ) -> Situation {
        let focus_brief = focus.map(|f| FocusBrief {
            activity: f.activity.as_str().to_string(),
            file: f.active_file.clone(),
            project: f.active_project.clone(),
            ticket: f.linked_ticket.clone(),
            duration_minutes: (f.duration_seconds / 60) as u32,
        });

        let attention_briefs: Vec<AttentionBrief> = attention
            .iter()
            .map(|a| AttentionBrief {
                priority: a.priority,
                summary: a.summary.clone(),
                suggested_action: a.suggested_action.clone(),
                systems: a.systems_involved.iter().map(|s| s.to_string()).collect(),
            })
            .collect();

        // Collect relevant people from attention items' entity IDs
        let relevant_people = self.collect_relevant_people(conn, attention);

        // Build recent activity summary
        let recent_activity = self.build_activity_summary(conn);

        Situation {
            focus: focus_brief,
            attention_items: attention_briefs,
            relevant_people,
            recent_activity_summary: recent_activity,
        }
    }

    /// Find people connected to the attention items.
    fn collect_relevant_people(
        &self,
        conn: &Connection,
        attention: &[AttentionItem],
    ) -> Vec<PersonBrief> {
        let mut people = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        for item in attention {
            for entity_id in &item.entity_ids {
                // Find people related to this entity
                let rels = schema::get_relationships(conn, entity_id);
                for rel in &rels {
                    let person_id = if rel.source_id.starts_with("person:") {
                        &rel.source_id
                    } else if rel.target_id.starts_with("person:") {
                        &rel.target_id
                    } else {
                        continue;
                    };

                    if seen_ids.contains(person_id) {
                        continue;
                    }
                    seen_ids.insert(person_id.clone());

                    let name = person_id
                        .strip_prefix("person:")
                        .unwrap_or(person_id)
                        .to_string();

                    // Get their recent activity
                    let recent_pulses = schema::get_entity_pulses(conn, person_id, 3);
                    let activity = if recent_pulses.is_empty() {
                        "no recent activity".to_string()
                    } else {
                        recent_pulses
                            .iter()
                            .map(|p| p.summary.clone())
                            .collect::<Vec<_>>()
                            .join("; ")
                    };

                    people.push(PersonBrief {
                        name,
                        relationship: rel.rel_type.clone(),
                        recent_activity: activity,
                    });
                }
            }
        }

        people.truncate(5);
        people
    }

    /// Build a summary of recent activity across all systems.
    fn build_activity_summary(&self, conn: &Connection) -> String {
        let now = now_ts();
        let one_day_ago = now - 86400.0;

        let mut parts = Vec::new();

        // Count by event type in last 24h
        let counts: Vec<(String, i64)> = conn
            .prepare(
                "SELECT event_type, COUNT(*) FROM cortex_pulses
                 WHERE ts >= ?1 GROUP BY event_type ORDER BY COUNT(*) DESC LIMIT 5",
            )
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map(rusqlite::params![one_day_ago], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        for (event_type, count) in &counts {
            let label = match event_type.as_str() {
                "commit_pushed" => "commits",
                "ticket_viewed" => "tickets viewed",
                "ticket_transitioned" => "tickets moved",
                "email_received" => "emails received",
                "email_sent" => "emails sent",
                "file_edited" => "files edited",
                _ => continue,
            };
            parts.push(format!("{} {}", count, label));
        }

        if parts.is_empty() {
            // Don't report "no activity" — it just feeds nagging behavior
            String::new()
        } else {
            format!("Last 24h: {}", parts.join(", "))
        }
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
