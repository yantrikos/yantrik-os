//! Facebook connector — friends, events, interests via Meta Graph API.
//!
//! Pulls: friends list, upcoming events, liked pages/interests.
//! Seeds the cortex with people, events, and interest entities.
//!
//! Requires a Meta App with Facebook Login.
//! Uses OAuth2 — no PKCE (Meta doesn't support it), but code+redirect flow.
//!
//! API endpoints (Graph API v19.0):
//! - Friends: https://graph.facebook.com/v19.0/me/friends
//! - Events: https://graph.facebook.com/v19.0/me/events
//! - Likes: https://graph.facebook.com/v19.0/me/likes
//! - Profile: https://graph.facebook.com/v19.0/me

use super::{Connector, SeedEntity};

pub struct FacebookConnector;

impl FacebookConnector {
    pub fn new() -> Self {
        Self
    }
}

impl Connector for FacebookConnector {
    fn name(&self) -> &'static str {
        "Facebook"
    }

    fn service_id(&self) -> &'static str {
        "facebook"
    }

    fn scopes(&self) -> &[&str] {
        &[
            "public_profile",
            "user_friends",
            "user_events",
            "user_likes",
            "user_birthday",
            "user_hometown",
        ]
    }

    fn auth_url_base(&self) -> &str {
        "https://www.facebook.com/v19.0/dialog/oauth"
    }

    fn token_url(&self) -> &str {
        "https://graph.facebook.com/v19.0/oauth/access_token"
    }

    fn initial_sync(&self, access_token: &str) -> Result<Vec<SeedEntity>, String> {
        let mut entities = Vec::new();

        // 1. User profile
        match fetch_profile(access_token) {
            Ok(profile) => {
                tracing::info!("Facebook profile synced");
                entities.extend(profile);
            }
            Err(e) => tracing::warn!(error = %e, "Failed to fetch Facebook profile"),
        }

        // 2. Friends list
        match fetch_friends(access_token) {
            Ok(friends) => {
                tracing::info!(count = friends.len(), "Facebook friends synced");
                entities.extend(friends);
            }
            Err(e) => tracing::warn!(error = %e, "Failed to fetch Facebook friends"),
        }

        // 3. Events
        match fetch_events(access_token) {
            Ok(events) => {
                tracing::info!(count = events.len(), "Facebook events synced");
                entities.extend(events);
            }
            Err(e) => tracing::warn!(error = %e, "Failed to fetch Facebook events"),
        }

        // 4. Liked pages (interests)
        match fetch_likes(access_token) {
            Ok(likes) => {
                tracing::info!(count = likes.len(), "Facebook likes synced");
                entities.extend(likes);
            }
            Err(e) => tracing::warn!(error = %e, "Failed to fetch Facebook likes"),
        }

        Ok(entities)
    }

    fn incremental_sync(
        &self,
        access_token: &str,
        _since_ts: f64,
    ) -> Result<Vec<SeedEntity>, String> {
        // Incremental: just events and recent likes
        let mut entities = Vec::new();
        if let Ok(events) = fetch_events(access_token) {
            entities.extend(events);
        }
        if let Ok(likes) = fetch_likes(access_token) {
            entities.extend(likes);
        }
        Ok(entities)
    }
}

// ── Facebook Profile ──────────────────────────────────────────────

fn fetch_profile(access_token: &str) -> Result<Vec<SeedEntity>, String> {
    let url = format!(
        "https://graph.facebook.com/v19.0/me?fields=name,birthday,hometown,location&access_token={}",
        access_token
    );

    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("Facebook Graph API error: {}", e))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Failed to parse Facebook profile: {}", e))?;

    let mut entities = Vec::new();

    // Hometown as interest
    if let Some(hometown) = json["hometown"]["name"].as_str() {
        entities.push(SeedEntity {
            entity_type: "interest",
            identifier: format!("place-{}", hometown.to_lowercase().replace(' ', "-")),
            display_name: format!("Hometown: {}", hometown),
            source_system: "facebook",
            external_id: json["hometown"]["id"].as_str().unwrap_or("").to_string(),
            attributes: serde_json::json!({
                "category": "place",
                "subcategory": "hometown",
            }),
        });
    }

    // Current location as interest
    if let Some(location) = json["location"]["name"].as_str() {
        entities.push(SeedEntity {
            entity_type: "interest",
            identifier: format!("place-{}", location.to_lowercase().replace(' ', "-")),
            display_name: format!("Location: {}", location),
            source_system: "facebook",
            external_id: json["location"]["id"].as_str().unwrap_or("").to_string(),
            attributes: serde_json::json!({
                "category": "place",
                "subcategory": "current_location",
            }),
        });
    }

    Ok(entities)
}

// ── Facebook Friends ──────────────────────────────────────────────

fn fetch_friends(access_token: &str) -> Result<Vec<SeedEntity>, String> {
    let url = format!(
        "https://graph.facebook.com/v19.0/me/friends?fields=name,id&limit=200&access_token={}",
        access_token
    );

    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("Facebook Friends API error: {}", e))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Failed to parse Facebook friends: {}", e))?;

    let mut entities = Vec::new();
    if let Some(data) = json["data"].as_array() {
        for friend in data {
            let name = match friend["name"].as_str() {
                Some(n) => n,
                None => continue,
            };

            let fb_id = friend["id"].as_str().unwrap_or("").to_string();

            entities.push(SeedEntity {
                entity_type: "person",
                identifier: name.to_lowercase().replace(' ', "-"),
                display_name: name.to_string(),
                source_system: "facebook",
                external_id: fb_id,
                attributes: serde_json::json!({
                    "source": "facebook",
                    "relation": "friend",
                }),
            });
        }
    }

    Ok(entities)
}

// ── Facebook Events ───────────────────────────────────────────────

fn fetch_events(access_token: &str) -> Result<Vec<SeedEntity>, String> {
    let url = format!(
        "https://graph.facebook.com/v19.0/me/events?fields=name,start_time,end_time,place,description&limit=50&access_token={}",
        access_token
    );

    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("Facebook Events API error: {}", e))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Failed to parse Facebook events: {}", e))?;

    let mut entities = Vec::new();
    if let Some(data) = json["data"].as_array() {
        for event in data {
            let name = match event["name"].as_str() {
                Some(n) => n,
                None => continue,
            };

            let event_id = event["id"].as_str().unwrap_or("").to_string();
            let start_time = event["start_time"].as_str().unwrap_or("");
            let place_name = event["place"]["name"].as_str().unwrap_or("");

            entities.push(SeedEntity {
                entity_type: "event",
                identifier: format!("fb-event-{}", &event_id[..event_id.len().min(16)]),
                display_name: name.to_string(),
                source_system: "facebook",
                external_id: event_id,
                attributes: serde_json::json!({
                    "start_time": start_time,
                    "location": place_name,
                    "description": event["description"].as_str().unwrap_or(""),
                }),
            });
        }
    }

    Ok(entities)
}

// ── Facebook Likes (Interests) ────────────────────────────────────

fn fetch_likes(access_token: &str) -> Result<Vec<SeedEntity>, String> {
    let url = format!(
        "https://graph.facebook.com/v19.0/me/likes?fields=name,category,id&limit=100&access_token={}",
        access_token
    );

    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("Facebook Likes API error: {}", e))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Failed to parse Facebook likes: {}", e))?;

    let mut entities = Vec::new();
    if let Some(data) = json["data"].as_array() {
        for page in data {
            let name = match page["name"].as_str() {
                Some(n) => n,
                None => continue,
            };

            let category = page["category"].as_str().unwrap_or("general");
            let page_id = page["id"].as_str().unwrap_or("").to_string();

            entities.push(SeedEntity {
                entity_type: "interest",
                identifier: format!("fb-like-{}", name.to_lowercase().replace(' ', "-")),
                display_name: name.to_string(),
                source_system: "facebook",
                external_id: page_id,
                attributes: serde_json::json!({
                    "category": category,
                    "source": "facebook_likes",
                }),
            });
        }
    }

    Ok(entities)
}
