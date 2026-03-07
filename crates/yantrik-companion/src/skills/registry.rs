//! Skill registry — loads manifests, manages enabled/disabled state,
//! and provides queries for the Skill Store UI.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::manifest::{SkillCategory, SkillManifest};

/// Persistent skill state in SQLite.
#[derive(Debug, Clone)]
pub struct SkillState {
    pub skill_id: String,
    pub enabled: bool,
    /// User-configured settings (JSON).
    pub user_config: serde_json::Value,
}

/// A skill entry combining manifest + runtime state.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    pub manifest: SkillManifest,
    pub enabled: bool,
    pub user_config: serde_json::Value,
}

/// The skill registry — loads manifests, tracks state, provides queries.
pub struct SkillRegistry {
    /// All known skills keyed by ID.
    skills: HashMap<String, SkillEntry>,
    /// Ordered list of skill IDs for stable iteration.
    skill_order: Vec<String>,
    /// Skills directory path.
    skills_dir: PathBuf,
}

impl SkillRegistry {
    /// Initialize the registry: create SQLite table, load manifests, restore state.
    pub fn init(conn: &Connection, skills_dir: &Path) -> Self {
        Self::ensure_table(conn);
        let mut registry = Self {
            skills: HashMap::new(),
            skill_order: Vec::new(),
            skills_dir: skills_dir.to_path_buf(),
        };
        registry.load_manifests();
        registry.restore_state(conn);
        registry
    }

    /// Create the skill_states table if it doesn't exist.
    fn ensure_table(conn: &Connection) {
        let _ = conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS skill_states (
                skill_id TEXT PRIMARY KEY,
                enabled INTEGER NOT NULL DEFAULT 0,
                user_config TEXT NOT NULL DEFAULT '{}',
                updated_at REAL NOT NULL DEFAULT (unixepoch('now'))
            );",
        );
    }

    /// Load all skill.yaml files from the skills directory.
    fn load_manifests(&mut self) {
        let entries = match std::fs::read_dir(&self.skills_dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(dir = %self.skills_dir.display(), error = %e, "Cannot read skills directory");
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();

            // Accept both skill.yaml inside a directory and top-level .yaml files
            let manifest_path = if path.is_dir() {
                let yaml = path.join("skill.yaml");
                let yml = path.join("skill.yml");
                if yaml.exists() {
                    yaml
                } else if yml.exists() {
                    yml
                } else {
                    continue;
                }
            } else if path.extension().map_or(false, |e| e == "yaml" || e == "yml") {
                path.clone()
            } else {
                continue;
            };

            match SkillManifest::from_file(&manifest_path) {
                Ok(manifest) => {
                    let id = manifest.id.clone();
                    tracing::debug!(skill = %id, "Loaded skill manifest");
                    self.skill_order.push(id.clone());
                    self.skills.insert(
                        id,
                        SkillEntry {
                            manifest,
                            enabled: false, // Will be restored from DB
                            user_config: serde_json::json!({}),
                        },
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        file = %manifest_path.display(),
                        error = %e,
                        "Failed to parse skill manifest"
                    );
                }
            }
        }

        tracing::info!(count = self.skills.len(), "Skill manifests loaded");
    }

    /// Restore enabled/disabled state from SQLite.
    fn restore_state(&mut self, conn: &Connection) {
        let mut stmt = match conn.prepare(
            "SELECT skill_id, enabled, user_config FROM skill_states",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };

        let rows = match stmt.query_map([], |row| {
            Ok(SkillState {
                skill_id: row.get(0)?,
                enabled: row.get::<_, i64>(1)? != 0,
                user_config: row
                    .get::<_, String>(2)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or(serde_json::json!({})),
            })
        }) {
            Ok(r) => r,
            Err(_) => return,
        };

        for row in rows.flatten() {
            if let Some(entry) = self.skills.get_mut(&row.skill_id) {
                entry.enabled = row.enabled;
                entry.user_config = row.user_config;
            }
        }
    }

    // ── Queries ──

    /// List all skills (manifest + state).
    pub fn list_all(&self) -> Vec<&SkillEntry> {
        self.skill_order
            .iter()
            .filter_map(|id| self.skills.get(id))
            .collect()
    }

    /// List enabled skills only.
    pub fn list_enabled(&self) -> Vec<&SkillEntry> {
        self.list_all()
            .into_iter()
            .filter(|e| e.enabled)
            .collect()
    }

    /// Get a specific skill by ID.
    pub fn get(&self, skill_id: &str) -> Option<&SkillEntry> {
        self.skills.get(skill_id)
    }

    /// Filter skills by category.
    pub fn filter_by_category(&self, category: SkillCategory) -> Vec<&SkillEntry> {
        self.list_all()
            .into_iter()
            .filter(|e| e.manifest.category == category)
            .collect()
    }

    /// Search skills by query string (matches name, description, tags).
    pub fn search(&self, query: &str) -> Vec<&SkillEntry> {
        let q = query.to_lowercase();
        self.list_all()
            .into_iter()
            .filter(|e| {
                e.manifest.name.to_lowercase().contains(&q)
                    || e.manifest.description.to_lowercase().contains(&q)
                    || e.manifest.tags.iter().any(|t| t.to_lowercase().contains(&q))
                    || e.manifest.id.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// Get all categories that have at least one skill.
    pub fn active_categories(&self) -> Vec<SkillCategory> {
        let mut cats: HashSet<SkillCategory> = HashSet::new();
        for entry in self.skills.values() {
            cats.insert(entry.manifest.category);
        }
        let mut result: Vec<_> = cats.into_iter().collect();
        // Sort by the canonical order
        let order = SkillCategory::all();
        result.sort_by_key(|c| order.iter().position(|o| o == c).unwrap_or(usize::MAX));
        result
    }

    // ── Mutations ──

    /// Enable a skill. Returns list of dependency skill IDs that were auto-enabled.
    pub fn enable(&mut self, conn: &Connection, skill_id: &str) -> Vec<String> {
        let mut auto_enabled = Vec::new();

        // First, resolve dependencies
        let deps = if let Some(entry) = self.skills.get(skill_id) {
            entry.manifest.requires.clone()
        } else {
            return auto_enabled;
        };

        for dep_id in &deps {
            if let Some(dep) = self.skills.get(dep_id) {
                if !dep.enabled {
                    self.set_enabled(conn, dep_id, true);
                    auto_enabled.push(dep_id.clone());
                }
            }
        }

        // Enable the skill itself
        self.set_enabled(conn, skill_id, true);
        auto_enabled
    }

    /// Disable a skill.
    pub fn disable(&mut self, conn: &Connection, skill_id: &str) {
        self.set_enabled(conn, skill_id, false);
    }

    /// Toggle a skill. Returns (new_enabled_state, auto_enabled_deps).
    pub fn toggle(&mut self, conn: &Connection, skill_id: &str) -> (bool, Vec<String>) {
        let currently_enabled = self
            .skills
            .get(skill_id)
            .map(|e| e.enabled)
            .unwrap_or(false);

        if currently_enabled {
            self.disable(conn, skill_id);
            (false, Vec::new())
        } else {
            let deps = self.enable(conn, skill_id);
            (true, deps)
        }
    }

    fn set_enabled(&mut self, conn: &Connection, skill_id: &str, enabled: bool) {
        if let Some(entry) = self.skills.get_mut(skill_id) {
            entry.enabled = enabled;
        }
        let _ = conn.execute(
            "INSERT INTO skill_states (skill_id, enabled, updated_at)
             VALUES (?1, ?2, unixepoch('now'))
             ON CONFLICT(skill_id) DO UPDATE SET enabled = ?2, updated_at = unixepoch('now')",
            rusqlite::params![skill_id, enabled as i64],
        );
    }

    /// Update user config for a skill.
    pub fn set_config(
        &mut self,
        conn: &Connection,
        skill_id: &str,
        config: serde_json::Value,
    ) {
        if let Some(entry) = self.skills.get_mut(skill_id) {
            entry.user_config = config.clone();
        }
        let config_str = serde_json::to_string(&config).unwrap_or_default();
        let _ = conn.execute(
            "INSERT INTO skill_states (skill_id, enabled, user_config, updated_at)
             VALUES (?1, 1, ?2, unixepoch('now'))
             ON CONFLICT(skill_id) DO UPDATE SET user_config = ?2, updated_at = unixepoch('now')",
            rusqlite::params![skill_id, config_str],
        );
    }

    /// Auto-enable skills whose services match the given list.
    /// Only runs if no skills have been toggled yet (empty skill_states).
    /// This ensures a fresh install enables skills matching config.enabled_services.
    pub fn auto_enable_for_services(&mut self, conn: &Connection, services: &[String]) {
        // Check if skill_states has any rows — if so, user has already configured
        let has_rows: bool = conn
            .query_row("SELECT COUNT(*) FROM skill_states", [], |row| row.get::<_, i64>(0))
            .unwrap_or(0)
            > 0;
        if has_rows {
            return;
        }

        let svc_set: std::collections::HashSet<String> =
            services.iter().map(|s| s.to_lowercase()).collect();
        let mut enabled_ids = Vec::new();

        for (id, entry) in &self.skills {
            // Enable if any of the skill's services match config
            let matches = entry.manifest.services.iter().any(|s| svc_set.contains(&s.to_lowercase()));
            if matches {
                enabled_ids.push(id.clone());
            }
        }

        for id in &enabled_ids {
            self.set_enabled(conn, id, true);
        }

        if !enabled_ids.is_empty() {
            tracing::info!(
                count = enabled_ids.len(),
                skills = ?enabled_ids,
                "Auto-enabled skills matching configured services"
            );
        }
    }

    // ── Aggregation queries (used by companion integration) ──

    /// Collect all tool IDs from enabled skills that should be in CORE_TOOLS.
    pub fn enabled_core_tools(&self) -> Vec<String> {
        self.list_enabled()
            .iter()
            .flat_map(|e| e.manifest.core_tools.iter().cloned())
            .collect()
    }

    /// Collect all service IDs from enabled skills.
    pub fn enabled_services(&self) -> Vec<String> {
        let mut services: HashSet<String> = HashSet::new();
        for entry in self.list_enabled() {
            for svc in &entry.manifest.services {
                services.insert(svc.clone());
            }
        }
        services.into_iter().collect()
    }

    /// Collect all instinct IDs from enabled skills.
    pub fn enabled_instincts(&self) -> HashSet<String> {
        self.list_enabled()
            .iter()
            .flat_map(|e| e.manifest.instincts.iter().cloned())
            .collect()
    }

    /// Collect all cortex rule IDs from enabled skills.
    pub fn enabled_cortex_rules(&self) -> HashSet<String> {
        self.list_enabled()
            .iter()
            .flat_map(|e| e.manifest.cortex_rules.iter().cloned())
            .collect()
    }

    /// Collect all tool IDs from enabled skills (for registry filtering).
    pub fn enabled_tool_ids(&self) -> HashSet<String> {
        self.list_enabled()
            .iter()
            .flat_map(|e| {
                e.manifest.tools.iter().chain(e.manifest.core_tools.iter()).cloned()
            })
            .collect()
    }

    /// Get the skill count.
    pub fn count(&self) -> usize {
        self.skills.len()
    }

    /// Get enabled count.
    pub fn enabled_count(&self) -> usize {
        self.skills.values().filter(|e| e.enabled).count()
    }
}
