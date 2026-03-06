//! Skill manifest — data-driven skill definitions loaded from YAML files.
//!
//! Each skill declares what it provides (tools, instincts, cortex rules)
//! and what it requires (services, other skills, config).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A skill manifest loaded from `skill.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    /// Unique skill identifier (slug), e.g. "email", "jira", "weather".
    pub id: String,
    /// Human-readable name shown in Skill Store UI.
    pub name: String,
    /// Short description (1 line, shown on card).
    pub description: String,
    /// Longer description shown in detail panel.
    #[serde(default)]
    pub long_description: String,
    /// Semantic version string.
    #[serde(default = "default_version")]
    pub version: String,
    /// Author name or organization.
    #[serde(default = "default_author")]
    pub author: String,
    /// Icon — emoji string for built-in skills, path for external.
    #[serde(default = "default_icon")]
    pub icon: String,
    /// Category for UI grouping.
    #[serde(default)]
    pub category: SkillCategory,
    /// Tags for search (beyond category).
    #[serde(default)]
    pub tags: Vec<String>,

    // ── What this skill provides ──

    /// Tool IDs this skill registers (must match tool names in the tool registry).
    /// For built-in skills, these are Rust tool names.
    /// For external skills, tools come from the MCP server.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Tool IDs that should be added to CORE_TOOLS when this skill is enabled.
    /// These tools get included in every LLM call (not just via discover_tools).
    #[serde(default)]
    pub core_tools: Vec<String>,
    /// Instinct IDs this skill activates.
    #[serde(default)]
    pub instincts: Vec<String>,
    /// Cortex rule IDs this skill enables.
    #[serde(default)]
    pub cortex_rules: Vec<String>,
    /// Service IDs this skill registers (added to enabled_services for rule gating).
    #[serde(default)]
    pub services: Vec<String>,

    // ── What this skill requires ──

    /// Other skill IDs that must be enabled first.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Config sections this skill needs (e.g. "email", "home_assistant").
    /// Used to check if the user has configured the necessary settings.
    #[serde(default)]
    pub requires_config: Vec<String>,
    /// Permissions this skill requests.
    #[serde(default)]
    pub permissions: Vec<SkillPermission>,

    // ── User-configurable settings ──

    /// JSON Schema for skill-specific settings the user can configure.
    /// Rendered as form fields in the Skill Store detail panel.
    #[serde(default)]
    pub config_schema: HashMap<String, serde_json::Value>,

    // ── External skill (MCP) ──

    /// MCP server configuration for external skills.
    #[serde(default)]
    pub mcp_server: Option<McpServerConfig>,
}

/// Skill category for UI grouping and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillCategory {
    Productivity,
    Communication,
    Development,
    Entertainment,
    SmartHome,
    Finance,
    Health,
    News,
    System,
    Search,
    Utility,
    Intelligence,
}

impl Default for SkillCategory {
    fn default() -> Self {
        Self::Utility
    }
}

impl SkillCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Productivity => "Productivity",
            Self::Communication => "Communication",
            Self::Development => "Development",
            Self::Entertainment => "Entertainment",
            Self::SmartHome => "Smart Home",
            Self::Finance => "Finance",
            Self::Health => "Health",
            Self::News => "News",
            Self::System => "System",
            Self::Search => "Search",
            Self::Utility => "Utility",
            Self::Intelligence => "Intelligence",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Productivity => "briefcase",
            Self::Communication => "chat",
            Self::Development => "code",
            Self::Entertainment => "music",
            Self::SmartHome => "home",
            Self::Finance => "dollar",
            Self::Health => "heart",
            Self::News => "newspaper",
            Self::System => "gear",
            Self::Search => "search",
            Self::Utility => "wrench",
            Self::Intelligence => "brain",
        }
    }

    /// All categories in display order.
    pub fn all() -> &'static [SkillCategory] {
        &[
            Self::Intelligence,
            Self::Productivity,
            Self::Communication,
            Self::Development,
            Self::Entertainment,
            Self::SmartHome,
            Self::Finance,
            Self::Health,
            Self::News,
            Self::System,
            Self::Search,
            Self::Utility,
        ]
    }
}

impl std::fmt::Display for SkillCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Permissions a skill can request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillPermission {
    /// Read/write filesystem access.
    Filesystem,
    /// Network access (HTTP, sockets).
    Network,
    /// Shell command execution.
    Shell,
    /// Send notifications to user.
    Notifications,
    /// Access to user's memory/knowledge graph.
    Memory,
    /// Background execution (timers, polling).
    Background,
    /// Send messages via external channels (Telegram, email).
    ExternalMessaging,
}

/// MCP server config for external skills.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Command to start the MCP server (e.g. "python", "node").
    pub command: String,
    /// Arguments (e.g. ["skill_server.py"]).
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Working directory for the server process.
    #[serde(default)]
    pub cwd: Option<String>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}
fn default_author() -> String {
    "Yantrik".to_string()
}
fn default_icon() -> String {
    "puzzle".to_string()
}

impl SkillManifest {
    /// Load a manifest from a YAML string.
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    /// Load a manifest from a file path.
    pub fn from_file(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        Ok(Self::from_yaml(&content)?)
    }

    /// Whether this is an external (MCP-based) skill.
    pub fn is_external(&self) -> bool {
        self.mcp_server.is_some()
    }
}
