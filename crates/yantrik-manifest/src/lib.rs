//! Yantrik Package Manifest — describes services, apps, skills, and widgets.
//!
//! Each installable package has a `yantrik.toml` manifest declaring its
//! identity, roles, permissions, and dependencies.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// A parsed package manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageManifest {
    pub package: PackageInfo,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub service: Option<ServiceConfig>,
    #[serde(default)]
    pub ui_app: Option<UiAppConfig>,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// Core package identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub roles: Vec<PackageRole>,
    #[serde(default)]
    pub icon: Option<String>,
}

/// What role(s) this package fulfills.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PackageRole {
    Service,
    UiApp,
    SearchProvider,
    WidgetProvider,
    CompanionTool,
    CompanionSkill,
}

/// A declared permission/capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permission {
    pub capability: String,
    #[serde(default)]
    pub reason: Option<String>,
}

/// A dependency on another package or service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub id: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub optional: bool,
}

/// Service-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    /// Binary name (relative to package dir).
    pub binary: String,
    /// IPC methods this service exposes.
    #[serde(default)]
    pub methods: Vec<String>,
    /// Auto-start on boot.
    #[serde(default)]
    pub autostart: bool,
}

/// UI app-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiAppConfig {
    /// Binary name.
    pub binary: String,
    /// Screen ID to register (for built-in apps).
    #[serde(default)]
    pub screen_id: Option<i32>,
    /// App categories for launcher.
    #[serde(default)]
    pub categories: Vec<String>,
}

impl PackageManifest {
    /// Load a manifest from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        Self::from_str(&content)
    }

    /// Parse a manifest from a TOML string.
    pub fn from_str(toml_str: &str) -> Result<Self, String> {
        toml::from_str(toml_str).map_err(|e| format!("Manifest parse error: {}", e))
    }

    /// Check if this package has a given role.
    pub fn has_role(&self, role: &PackageRole) -> bool {
        self.package.roles.contains(role)
    }

    /// Check if this package declares a given capability.
    pub fn has_permission(&self, capability: &str) -> bool {
        self.permissions.iter().any(|p| p.capability == capability)
    }
}
