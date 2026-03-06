//! Instagram connector — social graph and interests via Instagram Basic Display API.
//!
//! Pulls: user profile, media (recent posts), followed accounts.
//! Seeds the cortex with interest entities and social connections.
//!
//! Uses Instagram Basic Display API (simpler than Graph API for personal accounts).
//! For business accounts, the Graph API would be used instead.
//!
//! API endpoints:
//! - Profile: https://graph.instagram.com/me
//! - Media: https://graph.instagram.com/me/media
//!
//! Note: Instagram Basic Display API is being deprecated in favor of
//! Instagram API with Instagram Login. The connector uses the newer
//! Instagram API (via Facebook Login) which shares the Meta OAuth flow.

use super::{Connector, SeedEntity};

pub struct InstagramConnector;

impl InstagramConnector {
    pub fn new() -> Self {
        Self
    }
}

impl Connector for InstagramConnector {
    fn name(&self) -> &'static str {
        "Instagram"
    }

    fn service_id(&self) -> &'static str {
        "instagram"
    }

    fn scopes(&self) -> &[&str] {
        &[
            "instagram_basic",
            "instagram_manage_insights",
        ]
    }

    fn auth_url_base(&self) -> &str {
        // Instagram uses Facebook Login OAuth
        "https://www.facebook.com/v19.0/dialog/oauth"
    }

    fn token_url(&self) -> &str {
        "https://graph.facebook.com/v19.0/oauth/access_token"
    }

    fn initial_sync(&self, access_token: &str) -> Result<Vec<SeedEntity>, String> {
        let mut entities = Vec::new();

        // 1. Get Instagram business account ID via Facebook pages
        let ig_user_id = match get_instagram_user_id(access_token) {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(error = %e, "Could not find Instagram account, trying direct media fetch");
                // Fall back to direct media endpoint (works for basic display)
                if let Ok(media) = fetch_media_direct(access_token) {
                    entities.extend(media);
                }
                return Ok(entities);
            }
        };

        // 2. Fetch profile info
        match fetch_profile(access_token, &ig_user_id) {
            Ok(profile) => {
                tracing::info!("Instagram profile synced");
                entities.extend(profile);
            }
            Err(e) => tracing::warn!(error = %e, "Failed to fetch Instagram profile"),
        }

        // 3. Fetch recent media (posts)
        match fetch_media(access_token, &ig_user_id) {
            Ok(media) => {
                tracing::info!(count = media.len(), "Instagram media synced");
                entities.extend(media);
            }
            Err(e) => tracing::warn!(error = %e, "Failed to fetch Instagram media"),
        }

        Ok(entities)
    }

    fn incremental_sync(
        &self,
        access_token: &str,
        _since_ts: f64,
    ) -> Result<Vec<SeedEntity>, String> {
        let mut entities = Vec::new();

        match get_instagram_user_id(access_token) {
            Ok(ig_id) => {
                if let Ok(media) = fetch_media(access_token, &ig_id) {
                    entities.extend(media);
                }
            }
            Err(_) => {
                if let Ok(media) = fetch_media_direct(access_token) {
                    entities.extend(media);
                }
            }
        }

        Ok(entities)
    }
}

// ── Get Instagram User ID via Facebook Pages ──────────────────────

fn get_instagram_user_id(access_token: &str) -> Result<String, String> {
    // Get Facebook pages linked to this account
    let url = format!(
        "https://graph.facebook.com/v19.0/me/accounts?fields=instagram_business_account&access_token={}",
        access_token
    );

    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("Facebook Pages API error: {}", e))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Failed to parse pages response: {}", e))?;

    // Find the first page with an Instagram business account
    if let Some(data) = json["data"].as_array() {
        for page in data {
            if let Some(ig_id) = page["instagram_business_account"]["id"].as_str() {
                return Ok(ig_id.to_string());
            }
        }
    }

    Err("No Instagram business account found linked to Facebook".to_string())
}

// ── Instagram Profile ─────────────────────────────────────────────

fn fetch_profile(access_token: &str, ig_user_id: &str) -> Result<Vec<SeedEntity>, String> {
    let url = format!(
        "https://graph.facebook.com/v19.0/{}?fields=username,name,biography,followers_count,follows_count,media_count&access_token={}",
        ig_user_id, access_token
    );

    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("Instagram Profile API error: {}", e))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Failed to parse Instagram profile: {}", e))?;

    let mut entities = Vec::new();

    // Extract bio keywords as interests
    if let Some(bio) = json["biography"].as_str() {
        if !bio.is_empty() {
            entities.push(SeedEntity {
                entity_type: "interest",
                identifier: "instagram-bio".to_string(),
                display_name: format!("IG Bio: {}", truncate(bio, 50)),
                source_system: "instagram",
                external_id: ig_user_id.to_string(),
                attributes: serde_json::json!({
                    "category": "social_profile",
                    "bio": bio,
                    "followers": json["followers_count"].as_u64().unwrap_or(0),
                    "following": json["follows_count"].as_u64().unwrap_or(0),
                    "posts": json["media_count"].as_u64().unwrap_or(0),
                }),
            });
        }
    }

    Ok(entities)
}

// ── Instagram Media (via Graph API) ───────────────────────────────

fn fetch_media(access_token: &str, ig_user_id: &str) -> Result<Vec<SeedEntity>, String> {
    let url = format!(
        "https://graph.facebook.com/v19.0/{}/media?fields=caption,timestamp,media_type,like_count,comments_count&limit=25&access_token={}",
        ig_user_id, access_token
    );

    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("Instagram Media API error: {}", e))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Failed to parse Instagram media: {}", e))?;

    parse_media_items(&json)
}

// ── Instagram Media (direct, for basic display) ───────────────────

fn fetch_media_direct(access_token: &str) -> Result<Vec<SeedEntity>, String> {
    let url = format!(
        "https://graph.instagram.com/me/media?fields=caption,timestamp,media_type&limit=25&access_token={}",
        access_token
    );

    let resp = ureq::get(&url)
        .call()
        .map_err(|e| format!("Instagram Basic Display API error: {}", e))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Failed to parse Instagram media: {}", e))?;

    parse_media_items(&json)
}

// ── Shared media parser ───────────────────────────────────────────

fn parse_media_items(json: &serde_json::Value) -> Result<Vec<SeedEntity>, String> {
    let mut entities = Vec::new();
    let mut hashtag_counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();

    if let Some(data) = json["data"].as_array() {
        for post in data {
            // Extract hashtags from captions → interest entities
            if let Some(caption) = post["caption"].as_str() {
                for word in caption.split_whitespace() {
                    if word.starts_with('#') && word.len() > 1 {
                        let tag = word[1..].to_lowercase();
                        *hashtag_counts.entry(tag).or_default() += 1;
                    }
                }
            }
        }
    }

    // Convert frequent hashtags to interest entities
    for (tag, count) in &hashtag_counts {
        if *count >= 2 {
            entities.push(SeedEntity {
                entity_type: "interest",
                identifier: format!("ig-hashtag-{}", tag),
                display_name: format!("#{}", tag),
                source_system: "instagram",
                external_id: format!("hashtag:{}", tag),
                attributes: serde_json::json!({
                    "category": "hashtag",
                    "source": "instagram_posts",
                    "usage_count": count,
                }),
            });
        }
    }

    Ok(entities)
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}
