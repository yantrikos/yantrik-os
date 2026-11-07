//! Recipe Templates — pre-built recipes for common scenarios.
//!
//! These templates help smaller models (4B) by providing ready-made
//! multi-step workflows that only need variable substitution instead
//! of dynamic composition. ~50 templates covering daily routines,
//! communication, research, system administration, and personal tasks.

mod communication;
pub mod formations;
mod personal;
mod research;
mod routines;
mod system;

use crate::recipe::{hands_off, roles_named, Leave, Recipe, RecipeStep, RecipeStore, TriggerType};
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
    templates.extend(formations::templates());
    templates
}

/// The inputs a built-in fills in by itself when a run is not given them — a formation's seats —
/// as (name, value, what it is for). Empty for most.
pub fn defaults(template_id: &str) -> &'static [(&'static str, &'static str, &'static str)] {
    formations::defaults(template_id)
}

/// The inputs a built-in needs that `vars` does not give, or gives blank: (name, what it is).
pub fn missing_inputs(
    template_id: &str,
    vars: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Vec<(&'static str, &'static str)> {
    let Some(template) = get_template(template_id) else { return Vec::new() };
    template
        .required_vars
        .iter()
        .filter(|(name, _)| {
            !vars.and_then(|v| v.get(*name)).is_some_and(|v| match v {
                serde_json::Value::Null => false,
                serde_json::Value::String(s) => !s.trim().is_empty(),
                _ => true,
            })
        })
        .copied()
        .collect()
}

/// The roles a run of `id_or_name` given `vars` would hand work to — its Agent steps' roles with
/// the inputs, and the template's defaults for those not given, filled in — so the door starting
/// it can record the definition of each the person agrees to. Empty for a recipe with no Agent
/// step.
pub fn roles_for(
    conn: &Connection,
    id_or_name: &str,
    vars: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Result<Vec<String>, String> {
    let recipe = RecipeStore::get(conn, id_or_name)
        .or_else(|| RecipeStore::find_by_name(conn, id_or_name))
        .ok_or_else(|| format!("Recipe not found: {id_or_name}"))?;
    let steps: Vec<RecipeStep> = RecipeStore::get_steps(conn, &recipe.id).into_iter().map(|s| s.step).collect();
    let mut all: std::collections::HashMap<String, serde_json::Value> =
        defaults(&recipe.id).iter().map(|(k, v, _)| (k.to_string(), serde_json::Value::String(v.to_string()))).collect();
    for (k, v) in vars.into_iter().flatten() {
        if !v.is_null() {
            all.insert(k.clone(), v.clone());
        }
    }
    Ok(roles_named(&steps, &all))
}

/// Start a run of a recipe — by id or by name — with its inputs, for a door that may or may not
/// have the person's leave for agents.
///
/// A recipe that hands work to agents (a formation) starts only with `leave`, and only with every
/// input it needs: the run is then allowed its agents ([`RecipeStore::allow_agents`]). Anything
/// else starts as `run_recipe` always started it. The shell's `run_recipe` (graded sensitive, so
/// the person is asked) and the Recipes screen's Start pass a leave; the companion's own
/// `run_recipe` tool does not.
pub fn start(
    conn: &Connection,
    id_or_name: &str,
    vars: Option<&serde_json::Map<String, serde_json::Value>>,
    leave: Option<&Leave>,
) -> Result<(Recipe, String), String> {
    let recipe = RecipeStore::get(conn, id_or_name)
        .or_else(|| RecipeStore::find_by_name(conn, id_or_name))
        .ok_or_else(|| format!("Recipe not found: {id_or_name}"))?;
    let steps: Vec<RecipeStep> = RecipeStore::get_steps(conn, &recipe.id).into_iter().map(|s| s.step).collect();
    if !hands_off(&steps) {
        return RecipeStore::start_run(conn, &recipe.id, vars);
    }
    let Some(leave) = leave else {
        return Err(format!(
            "'{}' hands work to agents from the catalog, and starting agents needs the person's \
             leave: start it from the Recipes screen, or with the shell's run_recipe, which is \
             graded sensitive and asks them first.",
            recipe.name
        ));
    };
    let missing = missing_inputs(&recipe.id, vars);
    if !missing.is_empty() {
        let list: Vec<String> = missing.iter().map(|(name, what)| format!("`{name}` ({what})")).collect();
        return Err(format!("'{}' needs {} to start.", recipe.name, list.join(" and ")));
    }
    let (template, run) = RecipeStore::start_run(conn, &recipe.id, vars)?;
    RecipeStore::allow_agents(conn, &run, leave);
    tracing::info!(recipe_id = %run, name = %template.name, by = %leave.by, agent = ?leave.agent, "A formation was started");
    Ok((template, run))
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
