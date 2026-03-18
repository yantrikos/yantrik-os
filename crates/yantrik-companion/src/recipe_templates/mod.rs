//! Recipe Templates — pre-built recipes for common scenarios.
//!
//! These templates help smaller models (4B) by providing ready-made
//! multi-step workflows that only need variable substitution instead
//! of dynamic composition. ~50 templates covering daily routines,
//! communication, research, system administration, and personal tasks.

mod communication;
mod personal;
mod research;
mod routines;
mod system;

use crate::recipe::{RecipeStep, RecipeStore, TriggerType};
use rusqlite::Connection;

/// A recipe template definition.
pub struct RecipeTemplate {
    /// Fixed ID like "builtin_morning_briefing"
    pub id: &'static str,
    /// Human-readable name
    pub name: &'static str,
    /// Description of what this recipe does
    pub description: &'static str,
    /// Category for grouping
    pub category: &'static str,
    /// Keywords for intent matching (lowercase)
    pub keywords: &'static [&'static str],
    /// Variables the user must provide (name, description)
    pub required_vars: &'static [(&'static str, &'static str)],
    /// The recipe steps
    pub steps: fn() -> Vec<RecipeStep>,
    /// Optional trigger
    pub trigger: Option<fn() -> TriggerType>,
}

/// Get all built-in recipe templates.
pub fn all_templates() -> Vec<RecipeTemplate> {
    let mut templates = Vec::new();
    templates.extend(routines::templates());
    templates.extend(communication::templates());
    templates.extend(research::templates());
    templates.extend(system::templates());
    templates.extend(personal::templates());
    templates
}

/// Register all built-in recipe templates in the database.
pub fn register_all(conn: &Connection) {
    let templates = all_templates();
    let count = templates.len();
    for template in templates {
        let steps = (template.steps)();
        RecipeStore::ensure_builtin(conn, template.id, template.name, template.description, &steps);
    }
    tracing::info!(count, "Registered built-in recipe templates");
}

/// Match user intent to recipe templates using keyword scoring.
/// Returns (template_id, template_name, score) sorted by descending score.
pub fn match_intent(query: &str, limit: usize) -> Vec<(&'static str, &'static str, f64)> {
    let query_lower = query.to_lowercase();
    let query_words: Vec<&str> = query_lower.split_whitespace().collect();

    let mut scores: Vec<(&str, &str, f64)> = all_templates()
        .into_iter()
        .map(|t| {
            let mut score = 0.0;

            // Keyword matching (highest weight)
            for kw in t.keywords {
                if query_lower.contains(kw) {
                    score += 2.0;
                }
                // Partial word match
                for word in &query_words {
                    if word.len() >= 3 && (kw.contains(word) || word.contains(kw)) {
                        score += 0.5;
                    }
                }
            }

            // Name matching
            let name_lower = t.name.to_lowercase();
            for word in &query_words {
                if word.len() >= 3 && name_lower.contains(word) {
                    score += 1.0;
                }
            }

            // Description matching
            let desc_lower = t.description.to_lowercase();
            for word in &query_words {
                if word.len() >= 3 && desc_lower.contains(word) {
                    score += 0.3;
                }
            }

            (t.id, t.name, score)
        })
        .filter(|(_, _, score)| *score > 1.0)
        .collect();

    scores.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    scores.truncate(limit);
    scores
}

/// Get a template by ID.
pub fn get_template(id: &str) -> Option<RecipeTemplate> {
    all_templates().into_iter().find(|t| t.id == id)
}

/// List all template IDs and names grouped by category.
pub fn catalog_summary() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    let templates = all_templates();
    let mut categories: std::collections::BTreeMap<&str, Vec<(&str, &str)>> =
        std::collections::BTreeMap::new();
    for t in &templates {
        categories
            .entry(t.category)
            .or_default()
            .push((t.id, t.name));
    }
    categories.into_iter().collect()
}
