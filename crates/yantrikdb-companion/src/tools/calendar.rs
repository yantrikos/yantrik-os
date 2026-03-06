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
                "description": "Get today's calendar events. Quick way to see what's on the schedule for today.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, _args: &serde_json::Value) -> String {
        let token = match get_token(&self.accounts) {
            Ok(t) => t,
            Err(e) => return e,
        };

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let time_min = format!("{}T00:00:00Z", today);
        let time_max = format!("{}T23:59:59Z", today);

        match calendar::list_events(&token, None, Some(&time_min), Some(&time_max), 20, None) {
            Ok(events) => {
                if events.is_empty() {
                    format!("No events scheduled for today ({}).", today)
                } else {
                    let mut result = format!("Today's events ({}) — {} events:\n\n", today, events.len());
                    for e in &events {
                        result.push_str(&format_event(e));
                        result.push('\n');
                    }
                    result
                }
            }
            Err(e) => format!("Failed to fetch today's events: {}", e),
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
                "description": "List calendar events within a date range. Use to check upcoming events, search for specific events, or review a week/month.",
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

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let token = match get_token(&self.accounts) {
            Ok(t) => t,
            Err(e) => return e,
        };

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

        match calendar::list_events(&token, None, Some(&time_min), Some(&time_max), max, query) {
            Ok(events) => {
                if events.is_empty() {
                    format!("No events found between {} and {}.", start, end)
                } else {
                    let mut result = format!("Events from {} to {} — {} found:\n\n", start, end, events.len());
                    for e in &events {
                        result.push_str(&format_event(e));
                        if let Some(ref desc) = e.description {
                            if !desc.is_empty() {
                                let short = if desc.len() > 100 {
                                    format!("{}...", &desc[..desc.floor_char_boundary(97)])
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
            }
            Err(e) => format!("Failed to list events: {}", e),
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
                "description": "Create a new calendar event on Google Calendar. Supports timed and all-day events.",
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
                "description": "Delete a calendar event by its ID. Get the ID from calendar_list_events or calendar_today first.",
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
                "description": "Update an existing calendar event. Only provide the fields you want to change.",
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
