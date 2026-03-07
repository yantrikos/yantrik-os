//! Bridge utilities — read skill state from SQLite for companion integration.
//!
//! The companion worker thread reads skill states at startup to determine
//! which services, instincts, and cortex rules should be active.

use std::collections::HashSet;
use std::path::Path;

use rusqlite::Connection;

use super::registry::SkillRegistry;

/// Snapshot of enabled skill state, computed from the SkillRegistry.
/// Used by the companion to configure tools, instincts, and cortex rules.
#[derive(Debug, Clone)]
pub struct SkillSnapshot {
    /// Service IDs from enabled skills (replaces config.enabled_services).
    pub enabled_services: Vec<String>,
    /// Instinct IDs from enabled skills.
    pub enabled_instincts: HashSet<String>,
    /// Cortex rule IDs from enabled skills.
    pub enabled_cortex_rules: HashSet<String>,
    /// Tool IDs that should be in CORE_TOOLS (from enabled skills).
    pub extra_core_tools: Vec<String>,
    /// All tool IDs from enabled skills (for registry filtering).
    pub enabled_tool_ids: HashSet<String>,
}

/// Load skill snapshot from the skills.db file.
/// Falls back to empty snapshot if the DB doesn't exist.
/// Optionally auto-enables skills matching config services on first run.
pub fn load_skill_snapshot(skills_dir: &Path) -> SkillSnapshot {
    load_skill_snapshot_with_services(skills_dir, &[])
}

/// Load skill snapshot, auto-enabling skills for given services on first run.
pub fn load_skill_snapshot_with_services(skills_dir: &Path, config_services: &[String]) -> SkillSnapshot {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let db_path = format!("{}/.config/yantrik/skills.db", home);

    let conn = match Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Cannot open skills.db — using empty skill snapshot");
            return SkillSnapshot::empty();
        }
    };

    let mut registry = SkillRegistry::init(&conn, skills_dir);
    if !config_services.is_empty() {
        registry.auto_enable_for_services(&conn, config_services);
    }

    SkillSnapshot {
        enabled_services: registry.enabled_services(),
        enabled_instincts: registry.enabled_instincts(),
        enabled_cortex_rules: registry.enabled_cortex_rules(),
        extra_core_tools: registry.enabled_core_tools(),
        enabled_tool_ids: registry.enabled_tool_ids(),
    }
}

impl SkillSnapshot {
    /// Empty snapshot (no skills enabled).
    pub fn empty() -> Self {
        Self {
            enabled_services: Vec::new(),
            enabled_instincts: HashSet::new(),
            enabled_cortex_rules: HashSet::new(),
            extra_core_tools: Vec::new(),
            enabled_tool_ids: HashSet::new(),
        }
    }

    /// Check if a specific instinct ID is enabled via skills.
    pub fn has_instinct(&self, id: &str) -> bool {
        self.enabled_instincts.contains(id)
    }

    /// Check if a specific cortex rule is enabled via skills.
    pub fn has_cortex_rule(&self, id: &str) -> bool {
        self.enabled_cortex_rules.contains(id)
    }
}
