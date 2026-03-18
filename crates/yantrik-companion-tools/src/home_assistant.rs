//! Home Assistant tools — control smart home via HA REST API.
//!
//! Tools: ha_get_state, ha_call_service, ha_list_entities.
//! Only registered if `home_assistant.enabled = true` in config.
//! Uses `curl` for HTTP requests (no new Rust deps).

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

/// Register Home Assistant tools (only if enabled in config).
pub fn register(reg: &mut ToolRegistry, base_url: &str, token: &str) {
    let base = base_url.trim_end_matches('/').to_string();
    let tok = token.to_string();

    reg.register(Box::new(HaGetStateTool { base_url: base.clone(), token: tok.clone() }));
    reg.register(Box::new(HaCallServiceTool { base_url: base.clone(), token: tok.clone() }));
    reg.register(Box::new(HaListEntitiesTool { base_url: base, token: tok }));
}

/// Validate entity_id: only allow alphanum, dots, underscores.
fn validate_entity_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() < 128
        && id.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '_')
}

/// Run curl against HA API and return response body.
fn ha_curl(base_url: &str, token: &str, method: &str, path: &str, body: Option<&str>) -> String {
    let url = format!("{}{}", base_url, path);
    let mut cmd = std::process::Command::new("curl");
    cmd.arg("-s")
        .arg("-X").arg(method)
        .arg("-H").arg(format!("Authorization: Bearer {}", token))
        .arg("-H").arg("Content-Type: application/json")
        .arg("--connect-timeout").arg("5")
        .arg("--max-time").arg("10");

    if let Some(b) = body {
        cmd.arg("-d").arg(b);
    }

    cmd.arg(&url);

    match cmd.output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.len() > 3000 {
                format!("{}\n[Truncated — {} bytes]", &stdout[..stdout.floor_char_boundary(3000)], stdout.len())
            } else {
                stdout.to_string()
            }
        }
        Err(e) => format!("Error calling HA API: {e}"),
    }
}

// ── Get State ──

pub struct HaGetStateTool {
    base_url: String,
    token: String,
}

impl Tool for HaGetStateTool {
    fn name(&self) -> &'static str { "ha_get_state" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "home_assistant" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "ha_get_state",
                "description": "Get the current state of a Home Assistant entity (e",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entity_id": {"type": "string", "description": "HA entity ID (e.g. 'light.living_room')"}
                    },
                    "required": ["entity_id"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let entity_id = args.get("entity_id").and_then(|v| v.as_str()).unwrap_or_default();
        if !validate_entity_id(entity_id) {
            return "Error: invalid entity_id".to_string();
        }
        ha_curl(&self.base_url, &self.token, "GET", &format!("/api/states/{}", entity_id), None)
    }
}

// ── Call Service ──

pub struct HaCallServiceTool {
    base_url: String,
    token: String,
}

impl Tool for HaCallServiceTool {
    fn name(&self) -> &'static str { "ha_call_service" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Sensitive }
    fn category(&self) -> &'static str { "home_assistant" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "ha_call_service",
                "description": "Call a Home Assistant service (e",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "domain": {"type": "string", "description": "Service domain (e.g. 'light', 'switch')"},
                        "service": {"type": "string", "description": "Service name (e.g. 'turn_on', 'turn_off')"},
                        "entity_id": {"type": "string", "description": "Target entity ID"},
                        "data": {"type": "string", "description": "Optional JSON data (e.g. '{\"brightness\": 128}')"}
                    },
                    "required": ["domain", "service", "entity_id"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let domain = args.get("domain").and_then(|v| v.as_str()).unwrap_or_default();
        let service = args.get("service").and_then(|v| v.as_str()).unwrap_or_default();
        let entity_id = args.get("entity_id").and_then(|v| v.as_str()).unwrap_or_default();
        let extra_data = args.get("data").and_then(|v| v.as_str()).unwrap_or("{}");

        if domain.is_empty() || service.is_empty() || entity_id.is_empty() {
            return "Error: domain, service, and entity_id are required".to_string();
        }
        if !validate_entity_id(entity_id) {
            return "Error: invalid entity_id".to_string();
        }
        // Validate domain/service: alphanum + underscore only
        if !domain.chars().all(|c| c.is_alphanumeric() || c == '_')
            || !service.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            return "Error: invalid domain or service name".to_string();
        }

        // Build request body: merge entity_id into extra data
        let body = if extra_data == "{}" {
            format!("{{\"entity_id\": \"{}\"}}", entity_id)
        } else {
            // Parse extra data and inject entity_id
            match serde_json::from_str::<serde_json::Value>(extra_data) {
                Ok(mut v) => {
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("entity_id".to_string(), serde_json::json!(entity_id));
                    }
                    v.to_string()
                }
                Err(_) => format!("{{\"entity_id\": \"{}\"}}", entity_id),
            }
        };

        let path = format!("/api/services/{}/{}", domain, service);
        ha_curl(&self.base_url, &self.token, "POST", &path, Some(&body))
    }
}

// ── List Entities ──

pub struct HaListEntitiesTool {
    base_url: String,
    token: String,
}

impl Tool for HaListEntitiesTool {
    fn name(&self) -> &'static str { "ha_list_entities" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "home_assistant" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "ha_list_entities",
                "description": "List Home Assistant entities, optionally filtered by domain",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "domain": {"type": "string", "description": "Filter by domain (optional, e.g. 'light')"}
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let domain_filter = args.get("domain").and_then(|v| v.as_str()).unwrap_or("");

        let response = ha_curl(&self.base_url, &self.token, "GET", "/api/states", None);

        // Parse JSON response and filter/format
        match serde_json::from_str::<serde_json::Value>(&response) {
            Ok(serde_json::Value::Array(entities)) => {
                let filtered: Vec<String> = entities
                    .iter()
                    .filter_map(|e| {
                        let eid = e.get("entity_id")?.as_str()?;
                        if !domain_filter.is_empty() && !eid.starts_with(&format!("{}.", domain_filter)) {
                            return None;
                        }
                        let state = e.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");
                        let name = e.get("attributes")
                            .and_then(|a| a.get("friendly_name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or(eid);
                        Some(format!("- {} ({}) = {}", name, eid, state))
                    })
                    .take(50)
                    .collect();

                if filtered.is_empty() {
                    if domain_filter.is_empty() {
                        "No entities found.".to_string()
                    } else {
                        format!("No entities found in domain '{domain_filter}'.")
                    }
                } else {
                    format!("Entities ({}):\n{}", filtered.len(), filtered.join("\n"))
                }
            }
            Ok(_) => response, // Return raw if not array
            Err(_) => response, // Return raw on parse error
        }
    }
}
