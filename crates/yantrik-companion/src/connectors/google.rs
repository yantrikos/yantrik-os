//! Google connector — Calendar, Contacts, Gmail via Google APIs.
//!
//! Pulls: contacts (people), calendar events, email threads.
//! Seeds the cortex with people, events, and relationships.
//!
//! Requires a Google Cloud OAuth2 client ID (set in config.yaml).
//! Uses PKCE — no client secret needed.
//!
//! API endpoints:
//! - People API: https://people.googleapis.com/v1/people/me/connections
//! - Calendar API: https://www.googleapis.com/calendar/v3/calendars/primary/events
//! - Gmail API: https://gmail.googleapis.com/gmail/v1/users/me/messages

use super::{Connector, SeedEntity};

pub struct GoogleConnector;

impl GoogleConnector {
    pub fn new() -> Self {
        Self
    }
}

impl Connector for GoogleConnector {
    fn name(&self) -> &'static str {
        "Google"
    }

    fn service_id(&self) -> &'static str {
        "google"
    }

    fn scopes(&self) -> &[&str] {
        &[
            "https://www.googleapis.com/auth/contacts.readonly",
            "https://www.googleapis.com/auth/calendar.readonly",
            "https://www.googleapis.com/auth/gmail.readonly",
            "https://www.googleapis.com/auth/userinfo.profile",
        ]
    }

    fn auth_url_base(&self) -> &str {
        "https://accounts.google.com/o/oauth2/v2/auth"
    }

    fn token_url(&self) -> &str {
        "https://oauth2.googleapis.com/token"
    }

    fn initial_sync(&self, access_token: &str) -> Result<Vec<SeedEntity>, String> {
        let mut entities = Vec::new();

        // 1. Fetch contacts
        match fetch_contacts(access_token) {
            Ok(contacts) => {
                tracing::info!(count = contacts.len(), "Google contacts synced");
                entities.extend(contacts);
            }
            Err(e) => tracing::warn!(error = %e, "Failed to fetch Google contacts"),
        }

        // 2. Fetch upcoming calendar events
        match fetch_calendar_events(access_token) {
            Ok(events) => {
                tracing::info!(count = events.len(), "Google calendar events synced");
                entities.extend(events);
            }
            Err(e) => tracing::warn!(error = %e, "Failed to fetch Google calendar"),
        }

        // 3. Fetch recent email senders (top contacts from Gmail)
        match fetch_email_contacts(access_token) {
            Ok(contacts) => {
                tracing::info!(count = contacts.len(), "Gmail contacts synced");
                entities.extend(contacts);
            }
            Err(e) => tracing::warn!(error = %e, "Failed to fetch Gmail contacts"),
        }

        Ok(entities)
    }

    fn incremental_sync(
        &self,
        access_token: &str,
        _since_ts: f64,
    ) -> Result<Vec<SeedEntity>, String> {
        // For incremental: only fetch upcoming events and recent emails
        let mut entities = Vec::new();

        if let Ok(events) = fetch_calendar_events(access_token) {
            entities.extend(events);
        }
        if let Ok(contacts) = fetch_email_contacts(access_token) {
            entities.extend(contacts);
        }

        Ok(entities)
    }
}

// ── Google People API ────────────────────────────────────────────────

fn fetch_contacts(access_token: &str) -> Result<Vec<SeedEntity>, String> {
    let url = "https://people.googleapis.com/v1/people/me/connections\
               ?personFields=names,emailAddresses,phoneNumbers,organizations,relations\
               &pageSize=100&sortOrder=LAST_MODIFIED_DESCENDING";

    let resp = ureq::get(url)
        .set("Authorization", &format!("Bearer {}", access_token))
        .call()
        .map_err(|e| format!("Google People API error: {}", e))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Failed to parse contacts: {}", e))?;

    let mut entities = Vec::new();
    if let Some(connections) = json["connections"].as_array() {
        for person in connections {
            let name = match person["names"]
                .as_array()
                .and_then(|names| names.first())
                .and_then(|n| n["displayName"].as_str())
            {
                Some(n) => n,
                None => continue,
            };

            let email = person["emailAddresses"]
                .as_array()
                .and_then(|emails| emails.first())
                .and_then(|e| e["value"].as_str())
                .unwrap_or("");

            let org = person["organizations"]
                .as_array()
                .and_then(|orgs| orgs.first())
                .and_then(|o| o["name"].as_str())
                .unwrap_or("");

            // Determine relationship type from Google's relation field
            let relation = person["relations"]
                .as_array()
                .and_then(|rels| rels.first())
                .and_then(|r| r["type"].as_str())
                .unwrap_or("contact");

            let identifier = if !email.is_empty() {
                email.to_lowercase()
            } else {
                name.to_lowercase().replace(' ', "-")
            };

            entities.push(SeedEntity {
                entity_type: "person",
                identifier,
                display_name: name.to_string(),
                source_system: "google",
                external_id: person["resourceName"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                attributes: serde_json::json!({
                    "email": email,
                    "organization": org,
                    "relation": relation,
                }),
            });
        }
    }

    Ok(entities)
}

// ── Google Calendar API ──────────────────────────────────────────────

fn fetch_calendar_events(access_token: &str) -> Result<Vec<SeedEntity>, String> {
    let now = chrono::Utc::now();
    let time_min = now.to_rfc3339();
    let time_max = (now + chrono::Duration::days(14)).to_rfc3339();

    let url = format!(
        "https://www.googleapis.com/calendar/v3/calendars/primary/events\
         ?timeMin={}&timeMax={}&singleEvents=true&orderBy=startTime&maxResults=50",
        urlencod(&time_min),
        urlencod(&time_max),
    );

    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {}", access_token))
        .call()
        .map_err(|e| format!("Google Calendar API error: {}", e))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Failed to parse calendar: {}", e))?;

    let mut entities = Vec::new();
    if let Some(items) = json["items"].as_array() {
        for event in items {
            let summary = match event["summary"].as_str() {
                Some(s) => s,
                None => continue,
            };

            let event_id = event["id"].as_str().unwrap_or("").to_string();

            let start_time = event["start"]["dateTime"]
                .as_str()
                .or_else(|| event["start"]["date"].as_str())
                .unwrap_or("");

            // Extract attendees
            let attendees: Vec<String> = event["attendees"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|a| a["email"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            entities.push(SeedEntity {
                entity_type: "event",
                identifier: format!("gcal-{}", &event_id[..event_id.len().min(16)]),
                display_name: summary.to_string(),
                source_system: "google",
                external_id: event_id,
                attributes: serde_json::json!({
                    "start_time": start_time,
                    "attendees": attendees,
                    "location": event["location"].as_str().unwrap_or(""),
                    "description": event["description"].as_str().unwrap_or(""),
                }),
            });
        }
    }

    Ok(entities)
}

// ── Gmail API (recent senders) ───────────────────────────────────────

fn fetch_email_contacts(access_token: &str) -> Result<Vec<SeedEntity>, String> {
    // Fetch recent message metadata to discover frequent contacts
    let url = "https://gmail.googleapis.com/gmail/v1/users/me/messages\
               ?maxResults=50&q=newer_than:7d";

    let resp = ureq::get(url)
        .set("Authorization", &format!("Bearer {}", access_token))
        .call()
        .map_err(|e| format!("Gmail API error: {}", e))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Failed to parse Gmail list: {}", e))?;

    let mut sender_counts: std::collections::HashMap<String, (String, u32)> = std::collections::HashMap::new();

    if let Some(messages) = json["messages"].as_array() {
        // Fetch headers for each message (batch would be better, but keep it simple)
        for msg in messages.iter().take(30) {
            let msg_id = match msg["id"].as_str() {
                Some(id) => id,
                None => continue,
            };

            let detail_url = format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}?format=metadata&metadataHeaders=From",
                msg_id
            );

            if let Ok(detail_resp) = ureq::get(&detail_url)
                .set("Authorization", &format!("Bearer {}", access_token))
                .call()
            {
                if let Ok(detail) = detail_resp.into_json::<serde_json::Value>() {
                    if let Some(headers) = detail["payload"]["headers"].as_array() {
                        for header in headers {
                            if header["name"].as_str() == Some("From") {
                                if let Some(from) = header["value"].as_str() {
                                    let (name, email) = parse_email_header(from);
                                    let entry = sender_counts.entry(email.clone()).or_insert((name, 0));
                                    entry.1 += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Convert frequent senders to entities
    let mut entities: Vec<SeedEntity> = sender_counts
        .into_iter()
        .filter(|(_, (_, count))| *count >= 2) // At least 2 emails
        .map(|(email, (name, count))| SeedEntity {
            entity_type: "person",
            identifier: email.to_lowercase(),
            display_name: if name.is_empty() {
                email.clone()
            } else {
                name
            },
            source_system: "google",
            external_id: email.clone(),
            attributes: serde_json::json!({
                "email": email,
                "email_frequency_7d": count,
                "source": "gmail",
            }),
        })
        .collect();

    // Sort by frequency
    entities.sort_by(|a, b| {
        let freq_a = a.attributes["email_frequency_7d"].as_u64().unwrap_or(0);
        let freq_b = b.attributes["email_frequency_7d"].as_u64().unwrap_or(0);
        freq_b.cmp(&freq_a)
    });
    entities.truncate(20); // Top 20 contacts

    Ok(entities)
}

/// Parse "Name <email@example.com>" or "email@example.com" format.
fn parse_email_header(header: &str) -> (String, String) {
    if let Some(start) = header.find('<') {
        if let Some(end) = header.find('>') {
            let email = header[start + 1..end].trim().to_string();
            let name = header[..start].trim().trim_matches('"').to_string();
            return (name, email);
        }
    }
    // No angle brackets — assume it's just an email
    (String::new(), header.trim().to_string())
}

fn urlencod(s: &str) -> String {
    let mut result = String::new();
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

