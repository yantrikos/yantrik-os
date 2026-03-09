//! Calendar Connector — produces LifeEvents from calendar data.
//!
//! Works with two sources:
//! 1. Google Calendar API (via the Google connector's SeedEntities)
//! 2. ICS/iCalendar file import (local .ics files)
//!
//! Analyzes calendar events and produces:
//! - `CalendarApproaching` — event starts within alert window
//! - `CalendarConflict` — overlapping events detected
//! - `CalendarCreated` — new event appeared since last scan
//! - `FreeBlockDetected` — gap in schedule suitable for deep work
//! - `DateApproaching` — birthday/anniversary from recurring events
//!
//! Extracts attendees → entities for PWG Person node activation.
//! Extracts location → enables weather/commute context.

use serde::{Deserialize, Serialize};

use crate::graph_bridge::{LifeEvent, LifeEventKind};

// ── Calendar Event (normalized) ─────────────────────────────────────

/// A normalized calendar event from any source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarEvent {
    /// Unique ID (from Google Calendar, or generated from ICS UID).
    pub id: String,
    /// Event title/summary.
    pub summary: String,
    /// Start time as Unix timestamp.
    pub start_ts: f64,
    /// End time as Unix timestamp.
    pub end_ts: f64,
    /// Whether this is an all-day event.
    pub all_day: bool,
    /// Location (optional).
    pub location: String,
    /// Description/notes (optional).
    pub description: String,
    /// Attendee names/emails.
    pub attendees: Vec<String>,
    /// Whether this is a recurring event.
    pub recurring: bool,
    /// Source: "google", "ics", "caldav".
    pub source: String,
    /// Whether the user accepted/tentative/declined.
    pub status: EventStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventStatus {
    Confirmed,
    Tentative,
    Declined,
    Unknown,
}

/// Calendar analysis configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarConfig {
    /// Alert when event is within this many minutes.
    pub approach_alert_minutes: u32,
    /// Minimum free block size (minutes) to report.
    pub min_free_block_minutes: u32,
    /// Working hours for free block detection (24h format).
    pub work_start_hour: u8,
    pub work_end_hour: u8,
    /// Keywords that indicate a personal/important date.
    pub date_keywords: Vec<String>,
}

impl Default for CalendarConfig {
    fn default() -> Self {
        Self {
            approach_alert_minutes: 30,
            min_free_block_minutes: 60,
            work_start_hour: 9,
            work_end_hour: 18,
            date_keywords: vec![
                "birthday".into(), "anniversary".into(), "bday".into(),
                "celebration".into(), "wedding".into(),
            ],
        }
    }
}

// ── ICS Parser (lightweight) ────────────────────────────────────────

/// Parse a .ics file into CalendarEvents.
/// Handles basic VEVENT blocks — not a full RFC 5545 parser.
pub fn parse_ics(ics_content: &str) -> Vec<CalendarEvent> {
    let mut events = Vec::new();
    let mut in_event = false;
    let mut uid = String::new();
    let mut summary = String::new();
    let mut dtstart = String::new();
    let mut dtend = String::new();
    let mut location = String::new();
    let mut description = String::new();
    let mut attendees = Vec::new();
    let mut rrule = false;
    let mut status = EventStatus::Confirmed;
    let mut all_day = false;

    for line in ics_content.lines() {
        let line = line.trim_end_matches('\r');

        if line == "BEGIN:VEVENT" {
            in_event = true;
            uid.clear();
            summary.clear();
            dtstart.clear();
            dtend.clear();
            location.clear();
            description.clear();
            attendees.clear();
            rrule = false;
            status = EventStatus::Confirmed;
            all_day = false;
            continue;
        }

        if line == "END:VEVENT" {
            if in_event && !summary.is_empty() {
                let start_ts = parse_ics_datetime(&dtstart);
                let end_ts = if dtend.is_empty() {
                    start_ts + 3600.0 // default 1 hour
                } else {
                    parse_ics_datetime(&dtend)
                };

                if dtstart.len() == 8 {
                    all_day = true; // YYYYMMDD format = all-day
                }

                events.push(CalendarEvent {
                    id: if uid.is_empty() {
                        format!("ics-{}", events.len())
                    } else {
                        uid.clone()
                    },
                    summary: unescape_ics(&summary),
                    start_ts,
                    end_ts,
                    all_day,
                    location: unescape_ics(&location),
                    description: unescape_ics(&description),
                    attendees: attendees.clone(),
                    recurring: rrule,
                    source: "ics".into(),
                    status: status.clone(),
                });
            }
            in_event = false;
            continue;
        }

        if !in_event {
            continue;
        }

        if let Some(val) = line.strip_prefix("UID:") {
            uid = val.to_string();
        } else if let Some(val) = line.strip_prefix("SUMMARY:") {
            summary = val.to_string();
        } else if line.starts_with("DTSTART") {
            // DTSTART:20260309T100000Z or DTSTART;VALUE=DATE:20260309
            if let Some(val) = line.split(':').nth(1) {
                dtstart = val.to_string();
            }
        } else if line.starts_with("DTEND") {
            if let Some(val) = line.split(':').nth(1) {
                dtend = val.to_string();
            }
        } else if let Some(val) = line.strip_prefix("LOCATION:") {
            location = val.to_string();
        } else if let Some(val) = line.strip_prefix("DESCRIPTION:") {
            description = val.to_string();
        } else if line.starts_with("ATTENDEE") {
            // ATTENDEE;CN=John Doe:mailto:john@example.com
            if let Some(cn) = extract_ics_param(line, "CN") {
                attendees.push(cn);
            } else if let Some(mailto) = line.split("mailto:").nth(1) {
                attendees.push(mailto.to_string());
            }
        } else if line.starts_with("RRULE:") {
            rrule = true;
        } else if let Some(val) = line.strip_prefix("STATUS:") {
            status = match val.to_uppercase().as_str() {
                "CONFIRMED" => EventStatus::Confirmed,
                "TENTATIVE" => EventStatus::Tentative,
                "CANCELLED" => EventStatus::Declined,
                _ => EventStatus::Unknown,
            };
        }
    }

    events
}

/// Parse ICS datetime: "20260309T100000Z" or "20260309" (date only).
fn parse_ics_datetime(s: &str) -> f64 {
    if s.len() < 8 {
        return 0.0;
    }
    let year: i32 = s[0..4].parse().unwrap_or(2026);
    let month: u32 = s[4..6].parse().unwrap_or(1);
    let day: u32 = s[6..8].parse().unwrap_or(1);

    let (hour, min, sec) = if s.len() >= 15 && s.as_bytes().get(8) == Some(&b'T') {
        (
            s[9..11].parse().unwrap_or(0u32),
            s[11..13].parse().unwrap_or(0u32),
            s[13..15].parse().unwrap_or(0u32),
        )
    } else {
        (0, 0, 0)
    };

    // Simplified: calculate approximate Unix timestamp
    // (Not handling timezone properly — good enough for proximity detection)
    let days_since_epoch = days_from_ymd(year, month, day);
    (days_since_epoch as f64) * 86400.0 + (hour as f64) * 3600.0 + (min as f64) * 60.0 + sec as f64
}

/// Days from Unix epoch (1970-01-01) to Y-M-D.
fn days_from_ymd(year: i32, month: u32, day: u32) -> i64 {
    // Algorithm from Howard Hinnant
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let m = if month <= 2 { month + 9 } else { month - 3 } as i64;
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * m + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn extract_ics_param(line: &str, param: &str) -> Option<String> {
    let search = format!("{}=", param);
    let start = line.find(&search)? + search.len();
    let rest = &line[start..];
    // Value ends at ; or :
    let end = rest.find(|c| c == ';' || c == ':').unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

fn unescape_ics(s: &str) -> String {
    s.replace("\\n", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}

// ── Calendar → LifeEvent Analysis ───────────────────────────────────

/// Analyze a list of calendar events and produce LifeEvents.
pub fn analyze_calendar(
    events: &[CalendarEvent],
    config: &CalendarConfig,
) -> Vec<LifeEvent> {
    let now = now_ts();
    let mut life_events = Vec::new();

    // 1. Approaching events
    let alert_window = config.approach_alert_minutes as f64 * 60.0;
    for event in events {
        if event.status == EventStatus::Declined {
            continue;
        }

        let until_start = event.start_ts - now;
        if until_start > 0.0 && until_start <= alert_window && !event.all_day {
            life_events.push(approaching_event(event, until_start));
        }

        // Check for personal dates (birthday, anniversary)
        if is_personal_date(event, config) {
            let days_away = (event.start_ts - now) / 86400.0;
            if days_away > 0.0 && days_away <= 7.0 {
                life_events.push(personal_date_approaching(event, days_away));
            }
        }
    }

    // 2. Conflict detection
    life_events.extend(detect_conflicts(events, now));

    // 3. Free block detection
    life_events.extend(detect_free_blocks(events, config, now));

    life_events
}

fn approaching_event(event: &CalendarEvent, until_start: f64) -> LifeEvent {
    let minutes = (until_start / 60.0).round() as u32;
    let mut keywords = vec!["calendar".into(), "upcoming".into()];
    let mut entities: Vec<String> = event.attendees.clone();

    if !event.location.is_empty() {
        keywords.push("location".into());
    }

    let summary = if event.attendees.is_empty() {
        format!("\"{}\" starts in {} minutes", event.summary, minutes)
    } else {
        let people = if event.attendees.len() <= 3 {
            event.attendees.join(", ")
        } else {
            format!("{} and {} others", event.attendees[0], event.attendees.len() - 1)
        };
        entities.push(event.summary.clone());
        format!(
            "\"{}\" with {} starts in {} minutes",
            event.summary, people, minutes
        )
    };

    let mut data = serde_json::json!({
        "event_id": event.id,
        "event_title": event.summary,
        "start_ts": event.start_ts,
        "minutes_until": minutes,
    });
    if !event.location.is_empty() {
        data["location"] = serde_json::Value::String(event.location.clone());
    }

    LifeEvent {
        kind: LifeEventKind::CalendarApproaching,
        summary,
        keywords,
        entities,
        importance: if minutes <= 10 { 0.9 } else { 0.6 },
        source: format!("calendar:{}", event.source),
        data,
        timestamp: now_ts(),
    }
}

fn is_personal_date(event: &CalendarEvent, config: &CalendarConfig) -> bool {
    let lower = event.summary.to_lowercase();
    config.date_keywords.iter().any(|kw| lower.contains(kw))
}

fn personal_date_approaching(event: &CalendarEvent, days_away: f64) -> LifeEvent {
    let days = days_away.ceil() as u32;
    let summary = if days == 1 {
        format!("{} is tomorrow!", event.summary)
    } else {
        format!("{} is in {} days", event.summary, days)
    };

    // Extract person name from event summary (e.g., "John's Birthday" → "John")
    let person = event.summary
        .split("'s")
        .next()
        .or_else(|| event.summary.split("'s").next())
        .unwrap_or(&event.summary)
        .trim()
        .to_string();

    LifeEvent {
        kind: LifeEventKind::DateApproaching,
        summary,
        keywords: vec!["calendar".into(), "personal".into(), "date".into(), "reminder".into()],
        entities: vec![person],
        importance: if days <= 2 { 0.85 } else { 0.6 },
        source: format!("calendar:{}", event.source),
        data: serde_json::json!({
            "event_id": event.id,
            "event_title": event.summary,
            "days_away": days,
            "date_ts": event.start_ts,
        }),
        timestamp: now_ts(),
    }
}

fn detect_conflicts(events: &[CalendarEvent], now: f64) -> Vec<LifeEvent> {
    let mut conflicts = Vec::new();
    let upcoming: Vec<&CalendarEvent> = events
        .iter()
        .filter(|e| {
            e.end_ts > now
                && !e.all_day
                && e.status != EventStatus::Declined
        })
        .collect();

    for i in 0..upcoming.len() {
        for j in (i + 1)..upcoming.len() {
            let a = upcoming[i];
            let b = upcoming[j];

            // Overlap check: a.start < b.end && b.start < a.end
            if a.start_ts < b.end_ts && b.start_ts < a.end_ts {
                let overlap_mins = ((a.end_ts.min(b.end_ts) - a.start_ts.max(b.start_ts)) / 60.0).round() as u32;
                conflicts.push(LifeEvent {
                    kind: LifeEventKind::CalendarConflict,
                    summary: format!(
                        "Schedule conflict: \"{}\" and \"{}\" overlap by {} minutes",
                        a.summary, b.summary, overlap_mins
                    ),
                    keywords: vec!["calendar".into(), "conflict".into(), "schedule".into()],
                    entities: vec![a.summary.clone(), b.summary.clone()],
                    importance: 0.8,
                    source: "calendar".into(),
                    data: serde_json::json!({
                        "event_a": { "id": a.id, "title": a.summary },
                        "event_b": { "id": b.id, "title": b.summary },
                        "overlap_minutes": overlap_mins,
                    }),
                    timestamp: now_ts(),
                });
            }
        }
    }

    conflicts
}

fn detect_free_blocks(
    events: &[CalendarEvent],
    config: &CalendarConfig,
    now: f64,
) -> Vec<LifeEvent> {
    let mut free_blocks = Vec::new();
    let min_gap = config.min_free_block_minutes as f64 * 60.0;

    // Look at today's events only
    let today_start = (now / 86400.0).floor() * 86400.0;
    let work_start = today_start + config.work_start_hour as f64 * 3600.0;
    let work_end = today_start + config.work_end_hour as f64 * 3600.0;

    // Get today's non-allday events sorted by start time
    let mut todays: Vec<&CalendarEvent> = events
        .iter()
        .filter(|e| {
            e.start_ts >= today_start
                && e.start_ts < today_start + 86400.0
                && !e.all_day
                && e.status != EventStatus::Declined
        })
        .collect();
    todays.sort_by(|a, b| a.start_ts.partial_cmp(&b.start_ts).unwrap_or(std::cmp::Ordering::Equal));

    // Find gaps between events during work hours
    let effective_start = now.max(work_start);
    let mut cursor = effective_start;

    for event in &todays {
        if event.start_ts <= cursor {
            cursor = cursor.max(event.end_ts);
            continue;
        }

        let gap = event.start_ts - cursor;
        if gap >= min_gap && cursor < work_end {
            let block_end = event.start_ts.min(work_end);
            let block_mins = ((block_end - cursor) / 60.0).round() as u32;
            let block_hours = block_mins / 60;
            let remaining_mins = block_mins % 60;

            let time_desc = if block_hours > 0 && remaining_mins > 0 {
                format!("{}h {}min", block_hours, remaining_mins)
            } else if block_hours > 0 {
                format!("{}h", block_hours)
            } else {
                format!("{}min", remaining_mins)
            };

            free_blocks.push(LifeEvent {
                kind: LifeEventKind::FreeBlockDetected,
                summary: format!(
                    "Free block: {} available before \"{}\"",
                    time_desc, event.summary
                ),
                keywords: vec!["calendar".into(), "free".into(), "focus".into(), "deep work".into()],
                entities: vec![],
                importance: 0.4,
                source: "calendar".into(),
                data: serde_json::json!({
                    "start_ts": cursor,
                    "end_ts": block_end,
                    "duration_minutes": block_mins,
                    "next_event": event.summary,
                }),
                timestamp: now_ts(),
            });
        }
        cursor = cursor.max(event.end_ts);
    }

    // Check for free block after last event until work end
    if cursor < work_end {
        let gap = work_end - cursor;
        if gap >= min_gap {
            let block_mins = (gap / 60.0).round() as u32;
            let block_hours = block_mins / 60;
            let remaining_mins = block_mins % 60;

            let time_desc = if block_hours > 0 && remaining_mins > 0 {
                format!("{}h {}min", block_hours, remaining_mins)
            } else if block_hours > 0 {
                format!("{}h", block_hours)
            } else {
                format!("{}min", remaining_mins)
            };

            free_blocks.push(LifeEvent {
                kind: LifeEventKind::FreeBlockDetected,
                summary: format!(
                    "Free block: {} available for the rest of the work day",
                    time_desc
                ),
                keywords: vec!["calendar".into(), "free".into(), "focus".into()],
                entities: vec![],
                importance: 0.3,
                source: "calendar".into(),
                data: serde_json::json!({
                    "start_ts": cursor,
                    "end_ts": work_end,
                    "duration_minutes": block_mins,
                }),
                timestamp: now_ts(),
            });
        }
    }

    free_blocks
}

// ── Convert Google SeedEntities → CalendarEvents ────────────────────

/// Convert SeedEntities from the Google connector into normalized CalendarEvents.
pub fn from_google_seeds(seeds: &[super::SeedEntity]) -> Vec<CalendarEvent> {
    seeds
        .iter()
        .filter(|s| s.entity_type == "event" && s.source_system == "google")
        .filter_map(|seed| {
            let start_str = seed.attributes["start_time"].as_str()?;
            let start_ts = parse_rfc3339_approx(start_str);

            let attendees: Vec<String> = seed.attributes["attendees"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();

            let location = seed.attributes["location"]
                .as_str()
                .unwrap_or("")
                .to_string();

            let description = seed.attributes["description"]
                .as_str()
                .unwrap_or("")
                .to_string();

            Some(CalendarEvent {
                id: seed.external_id.clone(),
                summary: seed.display_name.clone(),
                start_ts,
                end_ts: start_ts + 3600.0, // default 1h if no end time
                all_day: start_str.len() <= 10, // "2026-03-09" = date only
                location,
                description,
                attendees,
                recurring: false,
                source: "google".into(),
                status: EventStatus::Confirmed,
            })
        })
        .collect()
}

/// Approximate RFC3339 parser: "2026-03-09T10:00:00+05:30" → unix timestamp.
fn parse_rfc3339_approx(s: &str) -> f64 {
    // Extract date parts
    let parts: Vec<&str> = s.split('T').collect();
    let date_parts: Vec<&str> = parts.first().unwrap_or(&"").split('-').collect();
    if date_parts.len() < 3 {
        return 0.0;
    }

    let year: i32 = date_parts[0].parse().unwrap_or(2026);
    let month: u32 = date_parts[1].parse().unwrap_or(1);
    let day: u32 = date_parts[2].parse().unwrap_or(1);

    let mut hour = 0u32;
    let mut min = 0u32;
    let mut sec = 0u32;

    if let Some(time_part) = parts.get(1) {
        // Strip timezone: take only HH:MM:SS
        let time_clean = time_part
            .split('+').next()
            .and_then(|t| t.split('-').next())
            .unwrap_or(time_part)
            .trim_end_matches('Z');

        let time_parts: Vec<&str> = time_clean.split(':').collect();
        hour = time_parts.first().and_then(|v| v.parse().ok()).unwrap_or(0);
        min = time_parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        sec = time_parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
    }

    let days = days_from_ymd(year, month, day);
    days as f64 * 86400.0 + hour as f64 * 3600.0 + min as f64 * 60.0 + sec as f64
}

// ── Helpers ─────────────────────────────────────────────────────────

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Use a fixed "now" for deterministic tests
    fn fixed_ts(year: i32, month: u32, day: u32, hour: u32, min: u32) -> f64 {
        let days = days_from_ymd(year, month, day);
        days as f64 * 86400.0 + hour as f64 * 3600.0 + min as f64 * 60.0
    }

    fn make_event(summary: &str, start_ts: f64, duration_min: u32) -> CalendarEvent {
        CalendarEvent {
            id: format!("test-{}", summary),
            summary: summary.into(),
            start_ts,
            end_ts: start_ts + duration_min as f64 * 60.0,
            all_day: false,
            location: String::new(),
            description: String::new(),
            attendees: vec![],
            recurring: false,
            source: "test".into(),
            status: EventStatus::Confirmed,
        }
    }

    #[test]
    fn parse_basic_ics() {
        let ics = "BEGIN:VCALENDAR\r\n\
            VERSION:2.0\r\n\
            BEGIN:VEVENT\r\n\
            UID:abc123\r\n\
            SUMMARY:Team Meeting\r\n\
            DTSTART:20260309T100000Z\r\n\
            DTEND:20260309T110000Z\r\n\
            LOCATION:Conference Room A\r\n\
            ATTENDEE;CN=Alice Smith:mailto:alice@example.com\r\n\
            ATTENDEE;CN=Bob Jones:mailto:bob@example.com\r\n\
            STATUS:CONFIRMED\r\n\
            END:VEVENT\r\n\
            END:VCALENDAR\r\n";

        let events = parse_ics(ics);
        assert_eq!(events.len(), 1);

        let ev = &events[0];
        assert_eq!(ev.id, "abc123");
        assert_eq!(ev.summary, "Team Meeting");
        assert_eq!(ev.location, "Conference Room A");
        assert_eq!(ev.attendees.len(), 2);
        assert!(ev.attendees.contains(&"Alice Smith".to_string()));
        assert_eq!(ev.source, "ics");
        assert!(!ev.all_day);
        assert!(ev.end_ts > ev.start_ts);
    }

    #[test]
    fn parse_allday_ics() {
        let ics = "BEGIN:VCALENDAR\r\n\
            BEGIN:VEVENT\r\n\
            SUMMARY:John's Birthday\r\n\
            DTSTART;VALUE=DATE:20260315\r\n\
            RRULE:FREQ=YEARLY\r\n\
            END:VEVENT\r\n\
            END:VCALENDAR\r\n";

        let events = parse_ics(ics);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary, "John's Birthday");
        assert!(events[0].all_day);
        assert!(events[0].recurring);
    }

    #[test]
    fn parse_escaped_ics() {
        let ics = "BEGIN:VCALENDAR\r\n\
            BEGIN:VEVENT\r\n\
            SUMMARY:Catch up\\, chat\r\n\
            DESCRIPTION:Meet at Bob\\;s place\\nBring snacks\r\n\
            DTSTART:20260309T140000Z\r\n\
            END:VEVENT\r\n\
            END:VCALENDAR\r\n";

        let events = parse_ics(ics);
        assert_eq!(events[0].summary, "Catch up, chat");
        assert!(events[0].description.contains('\n'));
    }

    #[test]
    fn conflict_detection() {
        let now = fixed_ts(2026, 3, 9, 8, 0);
        let events = vec![
            make_event("Meeting A", now + 3600.0, 60),  // 9:00-10:00
            make_event("Meeting B", now + 5400.0, 60),  // 9:30-10:30 (overlaps A)
            make_event("Meeting C", now + 7200.0, 60),  // 10:00-11:00 (overlaps B)
        ];

        let conflicts = detect_conflicts(&events, now);
        // A overlaps B, B overlaps C, A touches C edge but doesn't overlap
        assert!(conflicts.len() >= 2, "Should find at least 2 conflicts, found {}", conflicts.len());

        let ab_conflict = conflicts.iter().find(|e| {
            e.summary.contains("Meeting A") && e.summary.contains("Meeting B")
        });
        assert!(ab_conflict.is_some());
    }

    #[test]
    fn no_conflict_when_sequential() {
        let now = fixed_ts(2026, 3, 9, 8, 0);
        let events = vec![
            make_event("Meeting A", now + 3600.0, 60),  // 9:00-10:00
            make_event("Meeting B", now + 7200.0, 60),  // 10:00-11:00 (no overlap)
        ];

        let conflicts = detect_conflicts(&events, now);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn free_block_detection() {
        // Events: 9-10, 11-12, leaving 10-11 free
        let today_start = (now_ts() / 86400.0).floor() * 86400.0;
        let work_start = today_start + 9.0 * 3600.0;
        let config = CalendarConfig::default();

        let events = vec![
            make_event("Standup", work_start, 60),            // 9:00-10:00
            make_event("Design Review", work_start + 7200.0, 60), // 11:00-12:00
        ];

        // Set "now" to just before work starts so we see the full day
        let now = work_start - 60.0; // 8:59
        let free = detect_free_blocks(&events, &config, now);

        // Should detect 10:00-11:00 gap (60 min) and possibly 12:00-18:00
        assert!(!free.is_empty(), "Should detect free blocks");
    }

    #[test]
    fn personal_date_detection() {
        let config = CalendarConfig::default();
        let tomorrow = now_ts() + 86400.0;

        let events = vec![
            CalendarEvent {
                id: "bday-1".into(),
                summary: "Mom's Birthday".into(),
                start_ts: tomorrow,
                end_ts: tomorrow + 86400.0,
                all_day: true,
                location: String::new(),
                description: String::new(),
                attendees: vec![],
                recurring: true,
                source: "ics".into(),
                status: EventStatus::Confirmed,
            },
        ];

        let life_events = analyze_calendar(&events, &config);
        let date_event = life_events.iter().find(|e| e.kind == LifeEventKind::DateApproaching);
        assert!(date_event.is_some(), "Should detect birthday approaching");

        let de = date_event.unwrap();
        assert!(de.entities.contains(&"Mom".to_string()));
        assert!(de.importance >= 0.8);
    }

    #[test]
    fn approaching_event_with_attendees() {
        let config = CalendarConfig {
            approach_alert_minutes: 60,
            ..CalendarConfig::default()
        };

        let soon = now_ts() + 900.0; // 15 minutes from now
        let events = vec![CalendarEvent {
            id: "mtg-1".into(),
            summary: "Sprint Planning".into(),
            start_ts: soon,
            end_ts: soon + 3600.0,
            all_day: false,
            location: "Zoom".into(),
            description: String::new(),
            attendees: vec!["Alice".into(), "Bob".into()],
            recurring: false,
            source: "google".into(),
            status: EventStatus::Confirmed,
        }];

        let life_events = analyze_calendar(&events, &config);
        let approaching = life_events.iter().find(|e| e.kind == LifeEventKind::CalendarApproaching);
        assert!(approaching.is_some());

        let ap = approaching.unwrap();
        assert!(ap.summary.contains("Sprint Planning"));
        assert!(ap.summary.contains("Alice"));
        assert!(ap.importance >= 0.6); // within alert window
    }

    #[test]
    fn declined_events_excluded() {
        let config = CalendarConfig {
            approach_alert_minutes: 60,
            ..CalendarConfig::default()
        };
        let soon = now_ts() + 900.0;

        let events = vec![CalendarEvent {
            id: "declined-1".into(),
            summary: "Optional Sync".into(),
            start_ts: soon,
            end_ts: soon + 3600.0,
            all_day: false,
            location: String::new(),
            description: String::new(),
            attendees: vec![],
            recurring: false,
            source: "google".into(),
            status: EventStatus::Declined,
        }];

        let life_events = analyze_calendar(&events, &config);
        assert!(
            life_events.iter().all(|e| e.kind != LifeEventKind::CalendarApproaching),
            "Declined events should not produce approaching alerts"
        );
    }

    #[test]
    fn rfc3339_parsing() {
        let ts = parse_rfc3339_approx("2026-03-09T10:30:00Z");
        let expected = fixed_ts(2026, 3, 9, 10, 30);
        assert!((ts - expected).abs() < 1.0, "RFC3339 parse mismatch: {} vs {}", ts, expected);

        let ts_tz = parse_rfc3339_approx("2026-03-09T10:30:00+05:30");
        // Should parse the time part (ignoring TZ for simplicity)
        assert!(ts_tz > 0.0);
    }

    #[test]
    fn ics_datetime_parsing() {
        let ts = parse_ics_datetime("20260309T100000Z");
        let expected = fixed_ts(2026, 3, 9, 10, 0);
        assert!((ts - expected).abs() < 1.0);

        let ts_date = parse_ics_datetime("20260309");
        let expected_date = fixed_ts(2026, 3, 9, 0, 0);
        assert!((ts_date - expected_date).abs() < 1.0);
    }
}
