//! Connector trait and shared types for external service integrations.

/// A platform connector that can authenticate and sync data.
pub trait Connector: Send {
    /// Human-readable name (e.g., "Google", "Spotify").
    fn name(&self) -> &'static str;

    /// Service identifier used in config/db (e.g., "google", "spotify").
    fn service_id(&self) -> &'static str;

    /// OAuth2 scopes this connector needs.
    fn scopes(&self) -> &[&str];

    /// OAuth2 authorization URL base.
    fn auth_url_base(&self) -> &str;

    /// OAuth2 token exchange URL.
    fn token_url(&self) -> &str;

    /// Perform initial sync after first authorization.
    /// Returns seed entities for the cortex.
    fn initial_sync(
        &self,
        access_token: &str,
    ) -> Result<Vec<SeedEntity>, String>;

    /// Periodic sync — pull updates since last sync.
    fn incremental_sync(
        &self,
        access_token: &str,
        since_ts: f64,
    ) -> Result<Vec<SeedEntity>, String>;
}

/// An entity to seed into the cortex from a connector.
#[derive(Debug, Clone)]
pub struct SeedEntity {
    pub entity_type: &'static str,  // "person", "event", "interest", "project"
    pub identifier: String,          // canonical ID component
    pub display_name: String,
    pub source_system: &'static str, // "google", "spotify", "facebook"
    pub external_id: String,         // platform-specific ID
    pub attributes: serde_json::Value,
}
