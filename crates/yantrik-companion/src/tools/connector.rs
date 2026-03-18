//! Connector tools — LLM-accessible OAuth connector management.
//!
//! Tools:
//! - `list_connections` — show available and connected services
//! - `connect_service` — start OAuth flow (returns URL immediately, background listener completes)
//! - `sync_service` — trigger sync for a connected service
//! - `disconnect_service` — remove a service connection
//!
//! The connect flow is non-blocking: returns the auth URL so the LLM can send it
//! to the user (e.g. via Telegram), then a background thread listens for the
//! OAuth callback and completes token exchange + initial sync automatically.

use std::sync::{Arc, Mutex};

use super::{PermissionLevel, Tool, ToolContext, ToolRegistry};
use crate::config::ConnectorsConfig;
use crate::connectors::{self, ConnectorManager, SeedEntity};

/// Shared state for connector tools.
pub struct ConnectorState {
    pub manager: ConnectorManager,
    pub config: ConnectorsConfig,
    /// Path to memory.db — needed for background thread DB access.
    pub db_path: String,
    /// Pending OAuth flow.
    pub pending_auth: Option<PendingAuth>,
}

#[derive(Clone)]
pub struct PendingAuth {
    pub service: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub code_verifier: String,
    pub port: u16,
}

pub fn register(reg: &mut ToolRegistry, state: Arc<Mutex<ConnectorState>>) {
    reg.register(Box::new(ListConnectionsTool {
        state: state.clone(),
    }));
    reg.register(Box::new(ConnectServiceTool {
        state: state.clone(),
    }));
    reg.register(Box::new(SyncServiceTool {
        state: state.clone(),
    }));
    reg.register(Box::new(DisconnectServiceTool {
        state: state.clone(),
    }));
}

// ── List Connections ──

struct ListConnectionsTool {
    state: Arc<Mutex<ConnectorState>>,
}

impl Tool for ListConnectionsTool {
    fn name(&self) -> &'static str {
        "list_connections"
    }
    fn permission(&self) -> PermissionLevel {
        PermissionLevel::Safe
    }
    fn category(&self) -> &'static str {
        "connector"
    }
    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_connections",
                "description": "List available and connected external services (Google",
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }
        })
    }
    fn execute(&self, ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let state = match self.state.lock() {
            Ok(s) => s,
            Err(e) => return format!("Lock error: {}", e),
        };

        let available = state.manager.available();
        let connected = connectors::list_connected(ctx.db.conn());

        let mut lines = Vec::new();
        lines.push("Available connectors:".to_string());

        for svc in &available {
            let is_connected = connected.contains(&svc.to_string());
            let status = if is_connected {
                let last_sync = connectors::get_tokens(ctx.db.conn(), svc)
                    .map(|t| {
                        if t.last_sync_ts > 0.0 {
                            format_ago(t.last_sync_ts)
                        } else {
                            "never synced".to_string()
                        }
                    })
                    .unwrap_or_default();
                format!("connected (last sync: {})", last_sync)
            } else {
                "not connected".to_string()
            };
            lines.push(format!("  {} — {}", svc, status));
        }

        if connected.is_empty() {
            lines.push("\nNo services connected yet. Use connect_service to link an account.".to_string());
        }

        lines.join("\n")
    }
}

// ── Connect Service ──

struct ConnectServiceTool {
    state: Arc<Mutex<ConnectorState>>,
}

impl Tool for ConnectServiceTool {
    fn name(&self) -> &'static str {
        "connect_service"
    }
    fn permission(&self) -> PermissionLevel {
        PermissionLevel::Standard
    }
    fn category(&self) -> &'static str {
        "connector"
    }
    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "connect_service",
                "description": "Start OAuth2 connection for an external service",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "service": {
                            "type": "string",
                            "description": "Service to connect: 'google', 'spotify', 'facebook', or 'instagram'",
                            "enum": ["google", "spotify", "facebook", "instagram"]
                        }
                    },
                    "required": ["service"]
                }
            }
        })
    }
    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let service = match args["service"].as_str() {
            Some(s) => s,
            None => return "Missing 'service' parameter".to_string(),
        };

        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(e) => return format!("Lock error: {}", e),
        };

        // Check if already connected
        if connectors::is_connected(ctx.db.conn(), service) {
            return format!("{} is already connected. Use sync_service to sync or disconnect_service to remove.", service);
        }

        // Get client ID from config
        let client_id = match service {
            "google" => state.config.google_client_id.as_deref(),
            "spotify" => state.config.spotify_client_id.as_deref(),
            "facebook" => state.config.facebook_app_id.as_deref(),
            "instagram" => state.config.instagram_app_id.as_deref()
                .or(state.config.facebook_app_id.as_deref()),
            _ => return format!("Unknown service: {}", service),
        };

        let config_key = match service {
            "instagram" => "facebook_app_id (Instagram uses same Meta App ID)",
            _ => service,
        };

        let client_id = match client_id {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => return format!(
                "No client ID configured for {}. Add 'connectors.{}_client_id' to config.yaml.",
                service, config_key
            ),
        };

        // Resolve client secret
        let client_secret: Option<String> = match service {
            "spotify" => state.config.spotify_client_secret.clone(),
            "facebook" | "instagram" => state.config.facebook_app_secret.clone(),
            _ => None,
        };

        let port = state.config.callback_port;

        // Build auth URL
        let (auth_url, code_verifier) = match state.manager.start_auth(service, &client_id, port) {
            Some(pair) => pair,
            None => return format!("Service '{}' is not registered as a connector", service),
        };

        // Store pending auth state
        state.pending_auth = Some(PendingAuth {
            service: service.to_string(),
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            code_verifier: code_verifier.clone(),
            port,
        });

        // Spawn background thread to listen for the OAuth callback
        let state_arc = self.state.clone();
        let db_path = state.db_path.clone();
        let svc = service.to_string();
        std::thread::spawn(move || {
            tracing::info!(service = %svc, "Background OAuth listener started on port {}", port);

            match connectors::oauth::wait_for_callback(port, 300) {
                Ok(auth_code) => {
                    tracing::info!(service = %svc, "OAuth callback received, exchanging tokens...");

                    // Open a fresh DB connection for this thread
                    let conn = match rusqlite::Connection::open(&db_path) {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!(service = %svc, error = %e, "Failed to open DB for OAuth completion");
                            return;
                        }
                    };

                    let state = match state_arc.lock() {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::error!(service = %svc, error = %e, "Lock error during OAuth completion");
                            return;
                        }
                    };

                    let pending = match &state.pending_auth {
                        Some(p) if p.service == svc => p.clone(),
                        _ => {
                            tracing::error!(service = %svc, "No pending auth found");
                            return;
                        }
                    };

                    // Exchange code for tokens
                    let connector = state.manager.connectors_ref().iter()
                        .find(|c| c.service_id() == svc.as_str());
                    let connector = match connector {
                        Some(c) => c,
                        None => {
                            tracing::error!(service = %svc, "Connector not found");
                            return;
                        }
                    };

                    let token_url = connector.token_url().to_string();
                    let redirect_uri = format!("http://127.0.0.1:{}/callback", pending.port);

                    // Drop the lock before the network call
                    drop(state);

                    match connectors::oauth::exchange_code(
                        &token_url,
                        &pending.client_id,
                        &auth_code,
                        &pending.code_verifier,
                        &redirect_uri,
                        pending.client_secret.as_deref(),
                    ) {
                        Ok(tokens) => {
                            connectors::store_tokens(
                                &conn,
                                &svc,
                                &tokens.access_token,
                                &tokens.refresh_token,
                                tokens.expires_at,
                            );
                            tracing::info!(service = %svc, "OAuth tokens stored — connection complete!");

                            // Clear pending auth
                            if let Ok(mut s) = state_arc.lock() {
                                s.pending_auth = None;
                            }
                        }
                        Err(e) => {
                            tracing::error!(service = %svc, error = %e, "Token exchange failed");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(service = %svc, error = %e, "OAuth callback listener timed out or failed");
                    // Clear pending auth on failure
                    if let Ok(mut s) = state_arc.lock() {
                        if s.pending_auth.as_ref().map(|p| p.service.as_str()) == Some(svc.as_str()) {
                            s.pending_auth = None;
                        }
                    }
                }
            }
        });

        // Return immediately with the auth URL
        format!(
            "Open this URL to connect {}:\n\n{}\n\n\
             After you approve access, the connection will complete automatically. \
             The callback listener is running on port {}.",
            service, auth_url, port
        )
    }
}

// ── Sync Service ──

struct SyncServiceTool {
    state: Arc<Mutex<ConnectorState>>,
}

impl Tool for SyncServiceTool {
    fn name(&self) -> &'static str {
        "sync_service"
    }
    fn permission(&self) -> PermissionLevel {
        PermissionLevel::Safe
    }
    fn category(&self) -> &'static str {
        "connector"
    }
    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "sync_service",
                "description": "Trigger a sync for a connected service",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "service": {
                            "type": "string",
                            "description": "Service to sync: 'google', 'spotify', or 'all'",
                        }
                    },
                    "required": ["service"]
                }
            }
        })
    }
    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let service = match args["service"].as_str() {
            Some(s) => s,
            None => return "Missing 'service' parameter".to_string(),
        };

        let state = match self.state.lock() {
            Ok(s) => s,
            Err(e) => return format!("Lock error: {}", e),
        };

        if service == "all" {
            let connected = connectors::list_connected(ctx.db.conn());
            if connected.is_empty() {
                return "No services connected. Use connect_service first.".to_string();
            }
            let mut results = Vec::new();
            for svc in &connected {
                match state.manager.incremental_sync(ctx.db.conn(), svc) {
                    Ok(entities) => {
                        results.push(format!("{}: {} entities synced", svc, entities.len()));
                    }
                    Err(e) => {
                        results.push(format!("{}: sync failed — {}", svc, e));
                    }
                }
            }
            return results.join("\n");
        }

        if !connectors::is_connected(ctx.db.conn(), service) {
            return format!("{} is not connected. Use connect_service first.", service);
        }

        match state.manager.incremental_sync(ctx.db.conn(), service) {
            Ok(entities) => {
                let summary = summarize_seed_entities(&entities);
                format!("Synced {}: {} entities updated.\n{}", service, entities.len(), summary)
            }
            Err(e) => format!("Sync failed for {}: {}", service, e),
        }
    }
}

// ── Disconnect Service ──

struct DisconnectServiceTool {
    state: Arc<Mutex<ConnectorState>>,
}

impl Tool for DisconnectServiceTool {
    fn name(&self) -> &'static str {
        "disconnect_service"
    }
    fn permission(&self) -> PermissionLevel {
        PermissionLevel::Standard
    }
    fn category(&self) -> &'static str {
        "connector"
    }
    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "disconnect_service",
                "description": "Disconnect an external service and remove stored tokens",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "service": {
                            "type": "string",
                            "description": "Service to disconnect: 'google' or 'spotify'",
                        }
                    },
                    "required": ["service"]
                }
            }
        })
    }
    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let service = match args["service"].as_str() {
            Some(s) => s,
            None => return "Missing 'service' parameter".to_string(),
        };

        if !connectors::is_connected(ctx.db.conn(), service) {
            return format!("{} is not connected.", service);
        }

        connectors::disconnect(ctx.db.conn(), service);
        format!("{} disconnected. Tokens removed.", service)
    }
}

// ── Helpers ──

fn summarize_seed_entities(entities: &[SeedEntity]) -> String {
    if entities.is_empty() {
        return "No entities found.".to_string();
    }

    let mut by_type: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for e in entities {
        *by_type.entry(e.entity_type).or_default() += 1;
    }

    let mut parts: Vec<String> = by_type
        .iter()
        .map(|(t, c)| format!("{} {}s", c, t))
        .collect();
    parts.sort();

    let preview: Vec<String> = entities
        .iter()
        .take(5)
        .map(|e| format!("  • {} ({})", e.display_name, e.entity_type))
        .collect();

    let mut result = format!("Types: {}", parts.join(", "));
    if !preview.is_empty() {
        result.push_str("\nSample:\n");
        result.push_str(&preview.join("\n"));
        if entities.len() > 5 {
            result.push_str(&format!("\n  ...and {} more", entities.len() - 5));
        }
    }
    result
}

fn format_ago(ts: f64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let delta = now - ts;
    if delta < 60.0 {
        "just now".to_string()
    } else if delta < 3600.0 {
        format!("{}m ago", (delta / 60.0) as u64)
    } else if delta < 86400.0 {
        format!("{}h ago", (delta / 3600.0) as u64)
    } else {
        format!("{}d ago", (delta / 86400.0) as u64)
    }
}
