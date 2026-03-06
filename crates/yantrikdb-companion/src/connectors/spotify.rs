//! Spotify connector — music preferences, listening patterns.
//!
//! Pulls: top artists, recently played tracks, saved playlists.
//! Seeds the cortex with interest entities and listening patterns.
//!
//! API endpoints:
//! - Top Artists: https://api.spotify.com/v1/me/top/artists
//! - Recently Played: https://api.spotify.com/v1/me/player/recently-played
//! - User Profile: https://api.spotify.com/v1/me

use super::{Connector, SeedEntity};

pub struct SpotifyConnector;

impl SpotifyConnector {
    pub fn new() -> Self {
        Self
    }
}

impl Connector for SpotifyConnector {
    fn name(&self) -> &'static str {
        "Spotify"
    }

    fn service_id(&self) -> &'static str {
        "spotify"
    }

    fn scopes(&self) -> &[&str] {
        &[
            "user-read-recently-played",
            "user-top-read",
            "user-read-currently-playing",
            "playlist-read-private",
        ]
    }

    fn auth_url_base(&self) -> &str {
        "https://accounts.spotify.com/authorize"
    }

    fn token_url(&self) -> &str {
        "https://accounts.spotify.com/api/token"
    }

    fn initial_sync(&self, access_token: &str) -> Result<Vec<SeedEntity>, String> {
        let mut entities = Vec::new();

        // 1. Top artists (long-term preferences)
        match fetch_top_artists(access_token, "long_term") {
            Ok(artists) => {
                tracing::info!(count = artists.len(), "Spotify top artists synced");
                entities.extend(artists);
            }
            Err(e) => tracing::warn!(error = %e, "Failed to fetch Spotify top artists"),
        }

        // 2. Recently played (current mood/activity)
        match fetch_recently_played(access_token) {
            Ok(tracks) => {
                tracing::info!(count = tracks.len(), "Spotify recent tracks synced");
                entities.extend(tracks);
            }
            Err(e) => tracing::warn!(error = %e, "Failed to fetch Spotify recent tracks"),
        }

        Ok(entities)
    }

    fn incremental_sync(
        &self,
        access_token: &str,
        _since_ts: f64,
    ) -> Result<Vec<SeedEntity>, String> {
        // Incremental: just recently played
        fetch_recently_played(access_token)
    }
}

// ── Spotify Top Artists ──────────────────────────────────────────────

fn fetch_top_artists(access_token: &str, time_range: &str) -> Result<Vec<SeedEntity>, String> {
    let url = format!(
        "https://api.spotify.com/v1/me/top/artists?limit=20&time_range={}",
        time_range
    );

    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {}", access_token))
        .call()
        .map_err(|e| format!("Spotify API error: {}", e))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Failed to parse Spotify response: {}", e))?;

    let mut entities = Vec::new();
    if let Some(items) = json["items"].as_array() {
        for (rank, artist) in items.iter().enumerate() {
            let name = match artist["name"].as_str() {
                Some(n) => n,
                None => continue,
            };

            let genres: Vec<String> = artist["genres"]
                .as_array()
                .map(|g| g.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();

            let artist_id = artist["id"].as_str().unwrap_or("").to_string();

            // Create an interest entity for each top artist
            entities.push(SeedEntity {
                entity_type: "interest",
                identifier: format!("music-artist-{}", name.to_lowercase().replace(' ', "-")),
                display_name: format!("Music: {}", name),
                source_system: "spotify",
                external_id: artist_id,
                attributes: serde_json::json!({
                    "category": "music",
                    "subcategory": "artist",
                    "artist_name": name,
                    "genres": genres,
                    "rank": rank + 1,
                    "time_range": time_range,
                    "popularity": artist["popularity"].as_u64().unwrap_or(0),
                }),
            });

            // Also create interest entities for genres (if not already)
            for genre in &genres {
                entities.push(SeedEntity {
                    entity_type: "interest",
                    identifier: format!("music-genre-{}", genre.to_lowercase().replace(' ', "-")),
                    display_name: format!("Genre: {}", genre),
                    source_system: "spotify",
                    external_id: genre.clone(),
                    attributes: serde_json::json!({
                        "category": "music",
                        "subcategory": "genre",
                    }),
                });
            }
        }
    }

    Ok(entities)
}

// ── Spotify Recently Played ──────────────────────────────────────────

fn fetch_recently_played(access_token: &str) -> Result<Vec<SeedEntity>, String> {
    let url = "https://api.spotify.com/v1/me/player/recently-played?limit=50";

    let resp = ureq::get(url)
        .set("Authorization", &format!("Bearer {}", access_token))
        .call()
        .map_err(|e| format!("Spotify API error: {}", e))?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("Failed to parse Spotify response: {}", e))?;

    let mut entities = Vec::new();
    let mut seen_artists: std::collections::HashSet<String> = std::collections::HashSet::new();

    if let Some(items) = json["items"].as_array() {
        for item in items {
            let track = &item["track"];
            let artist_name = track["artists"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|a| a["name"].as_str())
                .unwrap_or("Unknown");

            let track_name = track["name"].as_str().unwrap_or("Unknown");
            let played_at = item["played_at"].as_str().unwrap_or("");

            // Deduplicate by artist for entity seeding
            let artist_key = artist_name.to_lowercase();
            if seen_artists.insert(artist_key.clone()) {
                entities.push(SeedEntity {
                    entity_type: "interest",
                    identifier: format!("music-recent-{}", artist_key.replace(' ', "-")),
                    display_name: format!("Recently: {} - {}", artist_name, track_name),
                    source_system: "spotify",
                    external_id: track["id"].as_str().unwrap_or("").to_string(),
                    attributes: serde_json::json!({
                        "category": "music",
                        "subcategory": "recent_listen",
                        "artist": artist_name,
                        "track": track_name,
                        "played_at": played_at,
                    }),
                });
            }
        }
    }

    Ok(entities)
}
