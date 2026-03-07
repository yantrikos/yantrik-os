//! Google Calendar API client — REST via ureq with OAuth2.
//!
//! Reuses the email account's OAuth2 tokens (same Google Cloud project).
//! All operations are blocking — runs on companion worker thread.

use crate::config::EmailAccountConfig;
use crate::email;
use serde::{Deserialize, Serialize};

const CALENDAR_API: &str = "https://www.googleapis.com/calendar/v3";

/// A calendar event from the Google Calendar API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalEvent {
    pub id: String,
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: String,      // ISO8601 or date string
    pub end: String,
    pub is_all_day: bool,
    pub status: String,     // "confirmed", "tentative", "cancelled"
    pub html_link: Option<String>,
}

/// A calendar list entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarInfo {
    pub id: String,
    pub summary: String,
    pub primary: bool,
}

/// Get a fresh OAuth2 access token from the email account config.
pub fn get_access_token(account: &mut EmailAccountConfig, config_path: Option<&str>) -> Result<String, String> {
    email::ensure_fresh_token(account, config_path)?;
    account.oauth_access_token.clone()
        .ok_or_else(|| "No OAuth access token available".to_string())
}

/// List user's calendars.
pub fn list_calendars(token: &str) -> Result<Vec<CalendarInfo>, String> {
    let url = format!("{}/users/me/calendarList", CALENDAR_API);
    let resp: serde_json::Value = ureq::get(&url)
        .set("Authorization", &format!("Bearer {}", token))
        .call()
        .map_err(|e| format!("Calendar list request failed: {}", e))?
        .into_json()
        .map_err(|e| format!("Calendar list parse failed: {}", e))?;

    let items = resp["items"].as_array()
        .ok_or("No items in calendar list response")?;

    Ok(items.iter().map(|item| {
        CalendarInfo {
            id: item["id"].as_str().unwrap_or("").to_string(),
            summary: item["summary"].as_str().unwrap_or("(untitled)").to_string(),
            primary: item["primary"].as_bool().unwrap_or(false),
        }
    }).collect())
}

/// List events from a calendar.
///
/// `time_min`/`time_max` are RFC3339 timestamps (e.g., "2026-03-05T00:00:00Z").
/// `calendar_id` defaults to "primary".
pub fn list_events(
    token: &str,
    calendar_id: Option<&str>,
    time_min: Option<&str>,
    time_max: Option<&str>,
    max_results: usize,
    query: Option<&str>,
) -> Result<Vec<CalEvent>, String> {
    let cal_id = calendar_id.unwrap_or("primary");
    let encoded_id = urlencoding(cal_id);
    let mut url = format!("{}/calendars/{}/events?singleEvents=true&orderBy=startTime&maxResults={}",
        CALENDAR_API, encoded_id, max_results);

    if let Some(tmin) = time_min {
        url.push_str(&format!("&timeMin={}", urlencoding(tmin)));
    }
    if let Some(tmax) = time_max {
        url.push_str(&format!("&timeMax={}", urlencoding(tmax)));
    }
    if let Some(q) = query {
        url.push_str(&format!("&q={}", urlencoding(q)));
    }

    let resp: serde_json::Value = ureq::get(&url)
        .set("Authorization", &format!("Bearer {}", token))
        .call()
        .map_err(|e| format!("Calendar events request failed: {}", e))?
        .into_json()
        .map_err(|e| format!("Calendar events parse failed: {}", e))?;

    let items = resp["items"].as_array()
        .ok_or("No items in events response")?;

    Ok(items.iter().filter_map(|item| {
        let status = item["status"].as_str().unwrap_or("confirmed");
        if status == "cancelled" { return None; }

        let (start_str, is_all_day) = parse_event_time(&item["start"]);
        let (end_str, _) = parse_event_time(&item["end"]);

        Some(CalEvent {
            id: item["id"].as_str().unwrap_or("").to_string(),
            summary: item["summary"].as_str().unwrap_or("(no title)").to_string(),
            description: item["description"].as_str().map(|s| s.to_string()),
            location: item["location"].as_str().map(|s| s.to_string()),
            start: start_str,
            end: end_str,
            is_all_day,
            status: status.to_string(),
            html_link: item["htmlLink"].as_str().map(|s| s.to_string()),
        })
    }).collect())
}

/// Create a new event.
pub fn create_event(
    token: &str,
    calendar_id: Option<&str>,
    summary: &str,
    start: &str,
    end: &str,
    description: Option<&str>,
    location: Option<&str>,
    is_all_day: bool,
) -> Result<CalEvent, String> {
    let cal_id = calendar_id.unwrap_or("primary");
    let encoded_id = urlencoding(cal_id);
    let url = format!("{}/calendars/{}/events", CALENDAR_API, encoded_id);

    let mut body = serde_json::json!({
        "summary": summary,
    });

    if is_all_day {
        body["start"] = serde_json::json!({"date": start});
        body["end"] = serde_json::json!({"date": end});
    } else {
        body["start"] = serde_json::json!({"dateTime": start});
        body["end"] = serde_json::json!({"dateTime": end});
    }

    if let Some(desc) = description {
        body["description"] = serde_json::json!(desc);
    }
    if let Some(loc) = location {
        body["location"] = serde_json::json!(loc);
    }

    let resp: serde_json::Value = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", token))
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| format!("Create event failed: {}", e))?
        .into_json()
        .map_err(|e| format!("Create event parse failed: {}", e))?;

    let (start_str, all_day) = parse_event_time(&resp["start"]);
    let (end_str, _) = parse_event_time(&resp["end"]);

    Ok(CalEvent {
        id: resp["id"].as_str().unwrap_or("").to_string(),
        summary: resp["summary"].as_str().unwrap_or(summary).to_string(),
        description: resp["description"].as_str().map(|s| s.to_string()),
        location: resp["location"].as_str().map(|s| s.to_string()),
        start: start_str,
        end: end_str,
        is_all_day: all_day,
        status: resp["status"].as_str().unwrap_or("confirmed").to_string(),
        html_link: resp["htmlLink"].as_str().map(|s| s.to_string()),
    })
}

/// Delete an event.
pub fn delete_event(
    token: &str,
    calendar_id: Option<&str>,
    event_id: &str,
) -> Result<(), String> {
    let cal_id = calendar_id.unwrap_or("primary");
    let encoded_id = urlencoding(cal_id);
    let url = format!("{}/calendars/{}/events/{}", CALENDAR_API, encoded_id, event_id);

    ureq::delete(&url)
        .set("Authorization", &format!("Bearer {}", token))
        .call()
        .map_err(|e| format!("Delete event failed: {}", e))?;

    Ok(())
}

/// Update an existing event (partial update via PATCH).
pub fn update_event(
    token: &str,
    calendar_id: Option<&str>,
    event_id: &str,
    summary: Option<&str>,
    start: Option<&str>,
    end: Option<&str>,
    description: Option<&str>,
    location: Option<&str>,
    is_all_day: Option<bool>,
) -> Result<CalEvent, String> {
    let cal_id = calendar_id.unwrap_or("primary");
    let encoded_id = urlencoding(cal_id);
    let url = format!("{}/calendars/{}/events/{}", CALENDAR_API, encoded_id, event_id);

    let mut body = serde_json::json!({});

    if let Some(s) = summary {
        body["summary"] = serde_json::json!(s);
    }
    if let Some(desc) = description {
        body["description"] = serde_json::json!(desc);
    }
    if let Some(loc) = location {
        body["location"] = serde_json::json!(loc);
    }

    let all_day = is_all_day.unwrap_or(false);
    if let Some(s) = start {
        if all_day {
            body["start"] = serde_json::json!({"date": s});
        } else {
            body["start"] = serde_json::json!({"dateTime": s});
        }
    }
    if let Some(e) = end {
        if all_day {
            body["end"] = serde_json::json!({"date": e});
        } else {
            body["end"] = serde_json::json!({"dateTime": e});
        }
    }

    let resp: serde_json::Value = ureq::request("PATCH", &url)
        .set("Authorization", &format!("Bearer {}", token))
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| format!("Update event failed: {}", e))?
        .into_json()
        .map_err(|e| format!("Update event parse failed: {}", e))?;

    let (start_str, ad) = parse_event_time(&resp["start"]);
    let (end_str, _) = parse_event_time(&resp["end"]);

    Ok(CalEvent {
        id: resp["id"].as_str().unwrap_or("").to_string(),
        summary: resp["summary"].as_str().unwrap_or("").to_string(),
        description: resp["description"].as_str().map(|s| s.to_string()),
        location: resp["location"].as_str().map(|s| s.to_string()),
        start: start_str,
        end: end_str,
        is_all_day: ad,
        status: resp["status"].as_str().unwrap_or("confirmed").to_string(),
        html_link: resp["htmlLink"].as_str().map(|s| s.to_string()),
    })
}

// ── Local SQLite Cache ──

/// Create the calendar_events table for local caching.
pub fn ensure_table(conn: &rusqlite::Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS calendar_events (
            id TEXT PRIMARY KEY,
            summary TEXT NOT NULL,
            description TEXT,
            location TEXT,
            start TEXT NOT NULL,
            end TEXT NOT NULL,
            is_all_day INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'confirmed',
            html_link TEXT,
            cached_at REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_calendar_events_start ON calendar_events(start);"
    ).ok();
}

/// Cache a list of events into the local SQLite table.
/// Replaces all events within the given date range.
pub fn cache_events(conn: &rusqlite::Connection, events: &[CalEvent], time_min: &str, time_max: &str) {
    // Delete existing events in this range
    conn.execute(
        "DELETE FROM calendar_events WHERE start >= ?1 AND start <= ?2",
        rusqlite::params![time_min, time_max],
    ).ok();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    for e in events {
        conn.execute(
            "INSERT OR REPLACE INTO calendar_events (id, summary, description, location, start, end, is_all_day, status, html_link, cached_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                e.id, e.summary, e.description, e.location,
                e.start, e.end, e.is_all_day as i32, e.status,
                e.html_link, now,
            ],
        ).ok();
    }
}

/// Read cached events from the local SQLite table within a date range.
pub fn get_cached_events(
    conn: &rusqlite::Connection,
    time_min: &str,
    time_max: &str,
) -> Vec<CalEvent> {
    let mut stmt = match conn.prepare(
        "SELECT id, summary, description, location, start, end, is_all_day, status, html_link
         FROM calendar_events
         WHERE start >= ?1 AND start <= ?2 AND status != 'cancelled'
         ORDER BY start ASC"
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    stmt.query_map(rusqlite::params![time_min, time_max], |row| {
        Ok(CalEvent {
            id: row.get(0)?,
            summary: row.get(1)?,
            description: row.get(2)?,
            location: row.get(3)?,
            start: row.get(4)?,
            end: row.get(5)?,
            is_all_day: row.get::<_, i32>(6)? != 0,
            status: row.get(7)?,
            html_link: row.get(8)?,
        })
    })
    .ok()
    .map(|rows| rows.flatten().collect())
    .unwrap_or_default()
}

/// Get the timestamp of the most recent cache update.
pub fn cache_age_secs(conn: &rusqlite::Connection) -> f64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    let cached_at: f64 = conn.query_row(
        "SELECT MAX(cached_at) FROM calendar_events",
        [],
        |row| row.get(0),
    ).unwrap_or(0.0);

    if cached_at == 0.0 { f64::MAX } else { now - cached_at }
}

// ── Helpers ──

fn parse_event_time(val: &serde_json::Value) -> (String, bool) {
    if let Some(dt) = val["dateTime"].as_str() {
        (dt.to_string(), false)
    } else if let Some(d) = val["date"].as_str() {
        (d.to_string(), true)
    } else {
        ("".to_string(), false)
    }
}

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                String::from(b as char)
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}
