//! Calendar service — event CRUD via filesystem-backed JSON storage.
//!
//! Events stored as individual `.json` files in `~/.local/share/yantrik/calendar/`.
//!
//! Methods:
//!   calendar.events       { start_date, end_date }                              → Vec<CalendarEvent>
//!   calendar.create_event { title, start, end, description?, location?, color? } → CalendarEvent
//!   calendar.update_event { id, title?, start?, end?, description?, location? }  → ()
//!   calendar.delete_event { id }                                                 → ()

use std::path::PathBuf;

use chrono::NaiveDateTime;
use yantrik_ipc_contracts::calendar::CalendarEvent;
use yantrik_service_sdk::prelude::*;

fn main() {
    std::fs::create_dir_all(calendar_dir()).ok();

    ServiceBuilder::new("calendar")
        .handler(CalendarHandler { dir: calendar_dir() })
        .run();
}

fn calendar_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local/share/yantrik/calendar")
    } else {
        PathBuf::from("/tmp/yantrik-calendar")
    }
}

struct CalendarHandler {
    dir: PathBuf,
}

impl ServiceHandler for CalendarHandler {
    fn service_id(&self) -> &str {
        "calendar"
    }

    fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        match method {
            "calendar.events" => {
                let start_date = require_str(&params, "start_date")?;
                let end_date = require_str(&params, "end_date")?;
                let events = self.list_events(start_date, end_date)?;
                Ok(serde_json::to_value(events).unwrap())
            }
            "calendar.create_event" => {
                let title = require_str(&params, "title")?;
                let start = require_str(&params, "start")?;
                let end = require_str(&params, "end")?;
                let description = params["description"].as_str().unwrap_or("");
                let location = params["location"].as_str().map(String::from);
                let color = params["color"].as_str().unwrap_or("");
                let event = self.create_event(title, start, end, description, location, color)?;
                Ok(serde_json::to_value(event).unwrap())
            }
            "calendar.update_event" => {
                let id = require_str(&params, "id")?;
                self.update_event(id, &params)?;
                Ok(serde_json::json!(null))
            }
            "calendar.delete_event" => {
                let id = require_str(&params, "id")?;
                self.delete_event(id)?;
                Ok(serde_json::json!(null))
            }
            _ => Err(ServiceError {
                code: -1,
                message: format!("Unknown method: {method}"),
            }),
        }
    }
}

fn require_str<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, ServiceError> {
    params[key].as_str().ok_or_else(|| ServiceError {
        code: -32602,
        message: format!("Missing '{key}' parameter"),
    })
}

// ── CRUD implementation ──────────────────────────────────────────────

impl CalendarHandler {
    fn event_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn read_event(&self, path: &std::path::Path) -> Option<CalendarEvent> {
        let data = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    fn write_event(&self, event: &CalendarEvent) -> Result<(), ServiceError> {
        let path = self.event_path(&event.id);
        let data = serde_json::to_string_pretty(event).map_err(|e| ServiceError {
            code: -32000,
            message: format!("Failed to serialize event: {e}"),
        })?;
        std::fs::write(&path, data).map_err(|e| ServiceError {
            code: -32000,
            message: format!("Failed to write event: {e}"),
        })
    }

    fn list_events(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<CalendarEvent>, ServiceError> {
        let entries = std::fs::read_dir(&self.dir).map_err(|e| ServiceError {
            code: -32000,
            message: format!("Cannot read calendar dir: {e}"),
        })?;

        let range_start = parse_iso_datetime(start_date);
        let range_end = parse_iso_datetime(end_date);

        let mut events = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Some(event) = self.read_event(&path) {
                // Filter by date range if both bounds parsed successfully.
                if let (Some(rs), Some(re)) = (range_start, range_end) {
                    let ev_start = parse_iso_datetime(&event.start);
                    let ev_end = parse_iso_datetime(&event.end);
                    // Include if event overlaps with [range_start, range_end].
                    let start_ok = ev_end.map(|e| e >= rs).unwrap_or(true);
                    let end_ok = ev_start.map(|s| s <= re).unwrap_or(true);
                    if start_ok && end_ok {
                        events.push(event);
                    }
                } else {
                    // If range is unparseable, return all events.
                    events.push(event);
                }
            }
        }

        // Sort by start time ascending.
        events.sort_by(|a, b| a.start.cmp(&b.start));
        Ok(events)
    }

    fn create_event(
        &self,
        title: &str,
        start: &str,
        end: &str,
        description: &str,
        location: Option<String>,
        _color: &str,
    ) -> Result<CalendarEvent, ServiceError> {
        let id = uuid7::uuid7().to_string();
        let event = CalendarEvent {
            id,
            title: title.to_string(),
            description: description.to_string(),
            start: start.to_string(),
            end: end.to_string(),
            is_all_day: false,
            location,
            attendees: Vec::new(),
            recurrence: None,
            calendar_id: "default".to_string(),
            remote_id: None,
        };
        self.write_event(&event)?;
        tracing::info!(id = %event.id, title = %event.title, "Created event");
        Ok(event)
    }

    fn update_event(
        &self,
        id: &str,
        params: &serde_json::Value,
    ) -> Result<(), ServiceError> {
        let path = self.event_path(id);
        let mut event = self.read_event(&path).ok_or_else(|| ServiceError {
            code: -32000,
            message: format!("Event not found: {id}"),
        })?;

        if let Some(v) = params["title"].as_str() {
            event.title = v.to_string();
        }
        if let Some(v) = params["start"].as_str() {
            event.start = v.to_string();
        }
        if let Some(v) = params["end"].as_str() {
            event.end = v.to_string();
        }
        if let Some(v) = params["description"].as_str() {
            event.description = v.to_string();
        }
        if let Some(v) = params["location"].as_str() {
            event.location = Some(v.to_string());
        }

        self.write_event(&event)?;
        tracing::info!(id = %id, "Updated event");
        Ok(())
    }

    fn delete_event(&self, id: &str) -> Result<(), ServiceError> {
        let path = self.event_path(id);
        let _ = std::fs::remove_file(&path);
        tracing::info!(id = %id, "Deleted event");
        Ok(())
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Parse an ISO 8601 datetime string into NaiveDateTime.
/// Accepts formats like "2026-03-18T10:00:00" or "2026-03-18".
fn parse_iso_datetime(s: &str) -> Option<NaiveDateTime> {
    // Try full datetime first.
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(dt);
    }
    // Try date-only (treat as start of day).
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return d.and_hms_opt(0, 0, 0);
    }
    None
}
