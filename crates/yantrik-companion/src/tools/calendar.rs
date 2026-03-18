//! Calendar tools — list events, create, update, delete via Google Calendar API.
//!
//! Reuses email account OAuth2 tokens for Google Calendar access.

use std::sync::Arc;
use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};
use crate::config::EmailAccountConfig;
use crate::calendar;

/// Register all calendar tools.
pub fn register(reg: &mut ToolRegistry, accounts: Vec<EmailAccountConfig>, cal_account: Option<String>) {
    // Find the calendar account (by name/email match, or first OAuth2 account)
    let account = find_cal_account(&accounts, cal_account.as_deref());
    if account.is_none() {
        tracing::warn!("Calendar enabled but no OAuth2 email account found — skipping");
        return;
    }
    let accounts = Arc::new(accounts);
    reg.register(Box::new(CalendarTodayTool { accounts: accounts.clone() }));
    reg.register(Box::new(CalendarListEventsTool { accounts: accounts.clone() }));
    reg.register(Box::new(CalendarCreateEventTool { accounts: accounts.clone() }));
    reg.register(Box::new(CalendarDeleteEventTool { accounts: accounts.clone() }));
    reg.register(Box::new(CalendarUpdateEventTool { accounts: accounts.clone() }));
}

/// Find the OAuth2 email account to use for calendar.
fn find_cal_account(accounts: &[EmailAccountConfig], preferred: Option<&str>) -> Option<EmailAccountConfig> {
    if let Some(name) = preferred {
        let lower = name.to_lowercase();
        for acc in accounts {
            if acc.auth_method.as_deref() == Some("oauth2")
                && (acc.name.to_lowercase().contains(&lower) || acc.email.to_lowercase().contains(&lower))
            {
                return Some(acc.clone());
            }
        }
    }
    // Fall back to first OAuth2 account
    accounts.iter()
        .find(|a| a.auth_method.as_deref() == Some("oauth2"))
        .cloned()
}

/// Get a fresh OAuth2 token from the first available account.
fn get_token(accounts: &[EmailAccountConfig]) -> Result<String, String> {
    let mut account = accounts.iter()
        .find(|a| a.auth_method.as_deref() == Some("oauth2"))
        .ok_or("No OAuth2 email account configured for calendar")?
        .clone();

    let config_path = std::env::var("YANTRIK_CONFIG").ok()
        .or_else(|| {
            let path = "/opt/yantrik/config.yaml";
            if std::path::Path::new(path).exists() { Some(path.to_string()) } else { None }
        });

    calendar::get_access_token(&mut account, config_path.as_deref())
}

/// Format a CalEvent into a readable string.
fn format_event(e: &calendar::CalEvent) -> String {
    let time = if e.is_all_day {
        format!("{} (all day)", e.start)
    } else {
        // Extract just the time portion from ISO8601
        let start_time = e.start.split('T').nth(1).unwrap_or(&e.start);
        let end_time = e.end.split('T').nth(1).unwrap_or(&e.end);
        // Trim timezone suffix for readability
        let start_clean = start_time.split('+').next().unwrap_or(start_time)
            .split('Z').next().unwrap_or(start_time);
        let end_clean = end_time.split('+').next().unwrap_or(end_time)
            .split('Z').next().unwrap_or(end_time);
        format!("{} - {}", start_clean, end_clean)
    };

    let mut line = format!("[{}] {} | {}", e.id, e.summary, time);
    if let Some(ref loc) = e.location {
        if !loc.is_empty() {
            line.push_str(&format!(" @ {}", loc));
        }
    }
    line
}

/// Format a list of events with just the basic info.
fn format_event_list(events: &[calendar::CalEvent], header: &str) -> String {
    if events.is_empty() {
        return header.replace("0 events", "No events").replace(" (cached)", "");
    }
    let mut result = format!("{}\n\n", header);
    for e in events {
        result.push_str(&format_event(e));
        result.push('\n');
    }
    result
}

/// Format a list of events with descriptions included.
fn format_event_list_detailed(events: &[calendar::CalEvent], header: &str) -> String {
    if events.is_empty() {
        return header.replace("0 found", "none found").replace(" (cached)", "");
    }
    let mut result = format!("{}\n\n", header);
    for e in events {
        result.push_str(&format_event(e));
        if let Some(ref desc) = e.description {
            if !desc.is_empty() {
                let short = if desc.len() > 100 {
                    format!("{}...", &desc[..desc.char_indices().take(97).last().map(|(i,_)| i).unwrap_or(0)])
                } else {
                    desc.clone()
                };
                result.push_str(&format!("  Note: {}", short));
            }
        }
        result.push('\n');
    }
    result
}

// ── calendar_today ──

struct CalendarTodayTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for CalendarTodayTool {
    fn name(&self) -> &'static str { "calendar_today" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "calendar" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "calendar_today",
                "description": "Get today's calendar events; read-only",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let time_min = format!("{}T00:00:00Z", today);
        let time_max = format!("{}T23:59:59Z", today);

        // Try local cache first (< 30 min old)
        let cached = calendar::get_cached_events(ctx.db.conn(), &time_min, &time_max);
        let cache_fresh = calendar::cache_age_secs(ctx.db.conn()) < 1800.0;

        if !cached.is_empty() && cache_fresh {
            return format_event_list(&cached, &format!("Today's events ({}) — {} events (cached):", today, cached.len()));
        }

        // Try API, cache results
        match get_token(&self.accounts) {
            Ok(token) => {
                match calendar::list_events(&token, None, Some(&time_min), Some(&time_max), 20, None) {
                    Ok(events) => {
                        calendar::cache_events(ctx.db.conn(), &events, &time_min, &time_max);
                        format_event_list(&events, &format!("Today's events ({}) — {} events:", today, events.len()))
                    }
                    Err(e) => {
                        // API failed — fall back to cache (even if stale)
                        if !cached.is_empty() {
                            format_event_list(&cached, &format!("Today's events ({}) — {} events (from cache, API unavailable):", today, cached.len()))
                        } else {
                            format!("Failed to fetch today's events: {}", e)
                        }
                    }
                }
            }
            Err(_) => {
                // No token — use cache
                if !cached.is_empty() {
                    format_event_list(&cached, &format!("Today's events ({}) — {} events (offline):", today, cached.len()))
                } else {
                    format!("No events scheduled for today ({}) — calendar not connected.", today)
                }
            }
        }
    }
}

// ── calendar_list_events ──

struct CalendarListEventsTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for CalendarListEventsTool {
    fn name(&self) -> &'static str { "calendar_list_events" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "calendar" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "calendar_list_events",
                "description": "List calendar events within a date range",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "start_date": {
                            "type": "string",
                            "description": "Start date in YYYY-MM-DD format. Defaults to today."
                        },
                        "end_date": {
                            "type": "string",
                            "description": "End date in YYYY-MM-DD format. Defaults to 7 days from start."
                        },
                        "query": {
                            "type": "string",
                            "description": "Search query to filter events by text."
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Maximum number of events to return. Default: 25."
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, ctx: &ToolContext, args: &serde_json::Value) -> String {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let start = args.get("start_date").and_then(|v| v.as_str()).unwrap_or(&today);
        let end = args.get("end_date").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_else(|| {
            // Default: 7 days from start
            if let Ok(d) = chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d") {
                (d + chrono::Duration::days(7)).format("%Y-%m-%d").to_string()
            } else {
                start.to_string()
            }
        });

        let query = args.get("query").and_then(|v| v.as_str());
        let max = args.get("max_results").and_then(|v| v.as_u64()).unwrap_or(25) as usize;

        let time_min = format!("{}T00:00:00Z", start);
        let time_max = format!("{}T23:59:59Z", end);

        // Try local cache first
        let cached = calendar::get_cached_events(ctx.db.conn(), &time_min, &time_max);
        let cache_fresh = calendar::cache_age_secs(ctx.db.conn()) < 1800.0;

        // For queries, always try API (cache doesn't support text search)
        let use_cache = query.is_none() && !cached.is_empty() && cache_fresh;

        if use_cache {
            let events: Vec<_> = cached.into_iter().take(max).collect();
            return format_event_list_detailed(&events, &format!("Events from {} to {} — {} found (cached):", start, end, events.len()));
        }

        match get_token(&self.accounts) {
            Ok(token) => {
                match calendar::list_events(&token, None, Some(&time_min), Some(&time_max), max, query) {
                    Ok(events) => {
                        if query.is_none() {
                            calendar::cache_events(ctx.db.conn(), &events, &time_min, &time_max);
                        }
                        format_event_list_detailed(&events, &format!("Events from {} to {} — {} found:", start, end, events.len()))
                    }
                    Err(e) => {
                        // API failed — fall back to cache
                        if !cached.is_empty() {
                            let events: Vec<_> = cached.into_iter().take(max).collect();
                            format_event_list_detailed(&events, &format!("Events from {} to {} — {} found (from cache, API unavailable):", start, end, events.len()))
                        } else {
                            format!("Failed to list events: {}", e)
                        }
                    }
                }
            }
            Err(_) => {
                if !cached.is_empty() {
                    let events: Vec<_> = cached.into_iter().take(max).collect();
                    format_event_list_detailed(&events, &format!("Events from {} to {} — {} found (offline):", start, end, events.len()))
                } else {
                    format!("No events found between {} and {} — calendar not connected.", start, end)
                }
            }
        }
    }
}

// ── calendar_create_event ──

struct CalendarCreateEventTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for CalendarCreateEventTool {
    fn name(&self) -> &'static str { "calendar_create_event" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "calendar" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "calendar_create_event",
                "description": "Create a new calendar event on Google Calendar",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "summary": {
                            "type": "string",
                            "description": "Event title/name."
                        },
                        "start": {
                            "type": "string",
                            "description": "Start time in ISO8601 format (e.g., '2026-03-10T14:00:00+05:30') or date for all-day (e.g., '2026-03-10')."
                        },
                        "end": {
                            "type": "string",
                            "description": "End time in ISO8601 format or date for all-day events."
                        },
                        "description": {
                            "type": "string",
                            "description": "Event description/notes."
                        },
                        "location": {
                            "type": "string",
                            "description": "Event location."
                        },
                        "all_day": {
                            "type": "boolean",
                            "description": "Whether this is an all-day event. Default: false."
                        }
                    },
                    "required": ["summary", "start", "end"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let token = match get_token(&self.accounts) {
            Ok(t) => t,
            Err(e) => return e,
        };

        let summary = match args.get("summary").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: summary".to_string(),
        };
        let start = match args.get("start").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: start".to_string(),
        };
        let end = match args.get("end").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: end".to_string(),
        };

        let description = args.get("description").and_then(|v| v.as_str());
        let location = args.get("location").and_then(|v| v.as_str());
        let all_day = args.get("all_day").and_then(|v| v.as_bool()).unwrap_or(false);

        match calendar::create_event(&token, None, summary, start, end, description, location, all_day) {
            Ok(event) => {
                let mut result = format!("Event created: {}\n", event.summary);
                result.push_str(&format!("ID: {}\n", event.id));
                result.push_str(&format!("When: {} - {}\n", event.start, event.end));
                if let Some(ref loc) = event.location {
                    result.push_str(&format!("Where: {}\n", loc));
                }
                if let Some(ref link) = event.html_link {
                    result.push_str(&format!("Link: {}\n", link));
                }
                result
            }
            Err(e) => format!("Failed to create event: {}", e),
        }
    }
}

// ── calendar_delete_event ──

struct CalendarDeleteEventTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for CalendarDeleteEventTool {
    fn name(&self) -> &'static str { "calendar_delete_event" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "calendar" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "calendar_delete_event",
                "description": "Delete a calendar event by its ID",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "event_id": {
                            "type": "string",
                            "description": "The event ID to delete."
                        }
                    },
                    "required": ["event_id"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let token = match get_token(&self.accounts) {
            Ok(t) => t,
            Err(e) => return e,
        };

        let event_id = match args.get("event_id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: event_id".to_string(),
        };

        match calendar::delete_event(&token, None, event_id) {
            Ok(()) => format!("Event {} deleted successfully.", event_id),
            Err(e) => format!("Failed to delete event: {}", e),
        }
    }
}

// ── calendar_update_event ──

struct CalendarUpdateEventTool {
    accounts: Arc<Vec<EmailAccountConfig>>,
}

impl Tool for CalendarUpdateEventTool {
    fn name(&self) -> &'static str { "calendar_update_event" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "calendar" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "calendar_update_event",
                "description": "Update an existing calendar event",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "event_id": {
                            "type": "string",
                            "description": "The event ID to update."
                        },
                        "summary": {
                            "type": "string",
                            "description": "New event title."
                        },
                        "start": {
                            "type": "string",
                            "description": "New start time (ISO8601)."
                        },
                        "end": {
                            "type": "string",
                            "description": "New end time (ISO8601)."
                        },
                        "description": {
                            "type": "string",
                            "description": "New description."
                        },
                        "location": {
                            "type": "string",
                            "description": "New location."
                        }
                    },
                    "required": ["event_id"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let token = match get_token(&self.accounts) {
            Ok(t) => t,
            Err(e) => return e,
        };

        let event_id = match args.get("event_id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return "Missing required parameter: event_id".to_string(),
        };

        let summary = args.get("summary").and_then(|v| v.as_str());
        let start = args.get("start").and_then(|v| v.as_str());
        let end = args.get("end").and_then(|v| v.as_str());
        let description = args.get("description").and_then(|v| v.as_str());
        let location = args.get("location").and_then(|v| v.as_str());

        match calendar::update_event(&token, None, event_id, summary, start, end, description, location, None) {
            Ok(event) => {
                let mut result = format!("Event updated: {}\n", event.summary);
                result.push_str(&format!("When: {} - {}\n", event.start, event.end));
                if let Some(ref loc) = event.location {
                    result.push_str(&format!("Where: {}\n", loc));
                }
                result
            }
            Err(e) => format!("Failed to update event: {}", e),
        }
    }
}
