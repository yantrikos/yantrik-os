//! Social & service connectors — OAuth2 API connections to external platforms.
//!
//! Enables Yantrik to connect to Google, Spotify, Facebook, etc. via OAuth2
//! PKCE flow. The user clicks "Connect" → browser opens → they approve →
//! we get tokens → background sync pulls their data into the cortex.
//!
//! Architecture:
//! ```
//! Settings UI → "Connect Google" → ConnectorManager::start_auth()
//!                                      ↓
//!                              Opens browser with OAuth URL
//!                                      ↓
//!                              User approves in browser
//!                                      ↓
//!                              Redirect to localhost:9876/callback
//!                                      ↓
//!                              OAuthCallback catches code
//!                                      ↓
//!                              Exchange code → tokens
//!                                      ↓
//!                              Store refresh token in DB
//!                                      ↓
//!                              Initial sync → seed cortex entities
//!                                      ↓
//!                              Background polling every N minutes
//! ```

pub mod oauth;
pub mod google;
pub mod spotify;
pub mod facebook;
pub mod instagram;
pub mod calendar;
pub mod events;
pub mod news;
pub mod weather;

use rusqlite::Connection;

// Re-export connector types from core
pub use yantrik_companion_core::connectors::{Connector, SeedEntity};

// ── Token Storage ────────────────────────────────────────────────────

/// Store OAuth tokens in the database.
pub fn store_tokens(
    conn: &Connection,
    service: &str,
    access_token: &str,
    refresh_token: &str,
    expires_at: f64,
) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS connector_tokens (
            service         TEXT PRIMARY KEY,
            access_token    TEXT NOT NULL,
            refresh_token   TEXT NOT NULL,
            expires_at      REAL NOT NULL,
            last_sync_ts    REAL NOT NULL DEFAULT 0.0,
            connected_at    REAL NOT NULL
        )",
    );

    let now = now_ts();
    let _ = conn.execute(
        "INSERT OR REPLACE INTO connector_tokens
            (service, access_token, refresh_token, expires_at, connected_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![service, access_token, refresh_token, expires_at, now],
    );
}

/// Get stored tokens for a service.
pub fn get_tokens(conn: &Connection, service: &str) -> Option<StoredTokens> {
    conn.query_row(
        "SELECT access_token, refresh_token, expires_at, last_sync_ts
         FROM connector_tokens WHERE service = ?1",
        rusqlite::params![service],
        |row| {
            Ok(StoredTokens {
                access_token: row.get(0)?,
                refresh_token: row.get(1)?,
                expires_at: row.get(2)?,
                last_sync_ts: row.get(3)?,
            })
        },
    )
    .ok()
}

/// Update last sync timestamp.
pub fn update_last_sync(conn: &Connection, service: &str) {
    let _ = conn.execute(
        "UPDATE connector_tokens SET last_sync_ts = ?1 WHERE service = ?2",
        rusqlite::params![now_ts(), service],
    );
}

/// Update access token after refresh.
pub fn update_access_token(conn: &Connection, service: &str, access_token: &str, expires_at: f64) {
    let _ = conn.execute(
        "UPDATE connector_tokens SET access_token = ?1, expires_at = ?2 WHERE service = ?3",
        rusqlite::params![access_token, expires_at, service],
    );
}

/// Check if a service is connected.
pub fn is_connected(conn: &Connection, service: &str) -> bool {
    get_tokens(conn, service).is_some()
}

/// List all connected services.
pub fn list_connected(conn: &Connection) -> Vec<String> {
    let mut stmt = match conn.prepare("SELECT service FROM connector_tokens") {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map([], |row| row.get::<_, String>(0))
        .ok()
        .map(|r| r.filter_map(|x| x.ok()).collect())
        .unwrap_or_default()
}

/// Remove a service connection.
pub fn disconnect(conn: &Connection, service: &str) {
    let _ = conn.execute(
        "DELETE FROM connector_tokens WHERE service = ?1",
        rusqlite::params![service],
    );
}

#[derive(Debug, Clone)]
pub struct StoredTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: f64,
    pub last_sync_ts: f64,
}

impl StoredTokens {
    pub fn is_expired(&self) -> bool {
        now_ts() >= self.expires_at - 60.0 // 1min buffer
    }
}

// ── Connector Manager ────────────────────────────────────────────────

/// Manages all connectors — auth flows, sync scheduling, token refresh.
pub struct ConnectorManager {
    connectors: Vec<Box<dyn Connector>>,
}

impl ConnectorManager {
    pub fn new() -> Self {
        Self {
            connectors: Vec::new(),
        }
    }

    /// Register a connector.
    pub fn register(&mut self, connector: Box<dyn Connector>) {
        self.connectors.push(connector);
    }

    /// Get a reference to registered connectors.
    pub fn connectors_ref(&self) -> &[Box<dyn Connector>] {
        &self.connectors
    }

    /// Get available (registered) connectors.
    pub fn available(&self) -> Vec<&str> {
        self.connectors.iter().map(|c| c.service_id()).collect()
    }

    /// Start OAuth flow for a service. Returns the authorization URL
    /// that should be opened in the browser.
    pub fn start_auth(&self, service: &str, client_id: &str, redirect_port: u16) -> Option<(String, String)> {
        let connector = self.connectors.iter().find(|c| c.service_id() == service)?;

        let (auth_url, code_verifier) = oauth::build_auth_url(
            connector.auth_url_base(),
            client_id,
            &format!("http://127.0.0.1:{}/callback", redirect_port),
            connector.scopes(),
        );

        Some((auth_url, code_verifier))
    }

    /// Complete OAuth flow — exchange auth code for tokens.
    pub fn complete_auth(
        &self,
        conn: &Connection,
        service: &str,
        client_id: &str,
        client_secret: Option<&str>,
        auth_code: &str,
        code_verifier: &str,
        redirect_port: u16,
    ) -> Result<(), String> {
        let connector = self.connectors.iter()
            .find(|c| c.service_id() == service)
            .ok_or_else(|| format!("Unknown service: {}", service))?;

        let tokens = oauth::exchange_code(
            connector.token_url(),
            client_id,
            auth_code,
            code_verifier,
            &format!("http://127.0.0.1:{}/callback", redirect_port),
            client_secret,
        )?;

        store_tokens(
            conn,
            service,
            &tokens.access_token,
            &tokens.refresh_token,
            tokens.expires_at,
        );

        tracing::info!(service, "OAuth tokens stored successfully");
        Ok(())
    }

    /// Run initial sync for a newly connected service.
    pub fn initial_sync(
        &self,
        conn: &Connection,
        service: &str,
    ) -> Result<Vec<SeedEntity>, String> {
        let connector = self.connectors.iter()
            .find(|c| c.service_id() == service)
            .ok_or_else(|| format!("Unknown service: {}", service))?;

        let tokens = get_tokens(conn, service)
            .ok_or_else(|| format!("{} not connected", service))?;

        // Refresh token if expired
        let access_token = if tokens.is_expired() {
            self.refresh_token(conn, service, &tokens)?
        } else {
            tokens.access_token.clone()
        };

        let entities = connector.initial_sync(&access_token)?;
        update_last_sync(conn, service);

        tracing::info!(
            service,
            entity_count = entities.len(),
            "Initial sync complete"
        );

        Ok(entities)
    }

    /// Run incremental sync for a connected service.
    pub fn incremental_sync(
        &self,
        conn: &Connection,
        service: &str,
    ) -> Result<Vec<SeedEntity>, String> {
        let connector = self.connectors.iter()
            .find(|c| c.service_id() == service)
            .ok_or_else(|| format!("Unknown service: {}", service))?;

        let tokens = get_tokens(conn, service)
            .ok_or_else(|| format!("{} not connected", service))?;

        let access_token = if tokens.is_expired() {
            self.refresh_token(conn, service, &tokens)?
        } else {
            tokens.access_token.clone()
        };

        let entities = connector.incremental_sync(&access_token, tokens.last_sync_ts)?;
        update_last_sync(conn, service);

        Ok(entities)
    }

    /// Refresh an expired access token.
    fn refresh_token(
        &self,
        conn: &Connection,
        service: &str,
        tokens: &StoredTokens,
    ) -> Result<String, String> {
        let connector = self.connectors.iter()
            .find(|c| c.service_id() == service)
            .ok_or_else(|| format!("Unknown service: {}", service))?;

        // Need client_id from config — for now we'll store it with the tokens
        // In practice, this comes from CompanionConfig
        let new_tokens = oauth::refresh_token(
            connector.token_url(),
            &tokens.refresh_token,
        )?;

        update_access_token(conn, service, &new_tokens.access_token, new_tokens.expires_at);
        Ok(new_tokens.access_token)
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
