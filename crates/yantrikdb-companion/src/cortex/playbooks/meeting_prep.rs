//! Meeting Prep playbook — detects upcoming meetings where the user
//! hasn't accessed related documents or reviewed relevant context.
//!
//! Evidence signals (need 2+ for high conviction):
//! 1. Calendar event within 30 minutes (from cortex calendar pulses)
//! 2. Related document entity not accessed in last hour
//! 3. Historical pattern: user scrambled last time for similar meetings
//!
//! Action: Notify with meeting details and suggest opening prep materials.

use crate::cortex::playbook::{CortexAction, PlaybookState};
use crate::cortex::schema;

/// Evaluate meeting prep needs. Pure Rust, no LLM.
pub fn evaluate(state: &PlaybookState) -> Vec<CortexAction> {
    let conn = state.conn;
    let now = state.now_ts;

    // Find calendar-related entities that have upcoming meetings
    // Look for MeetingScheduled or MeetingStarting pulses in the next 30 min
    let upcoming_window = 30.0 * 60.0; // 30 minutes

    // Query recent calendar pulses for meetings happening soon
    let meetings = find_upcoming_meetings(conn, now, upcoming_window);
    if meetings.is_empty() {
        return vec![];
    }

    let mut actions = Vec::new();

    for meeting in meetings {
        let mut evidence_count = 0u32;
        let mut evidence_details = Vec::new();

        // Signal 1: Meeting is within 30 minutes (always true if we got here)
        evidence_count += 1;
        evidence_details.push(format!("meeting '{}' in {} min", meeting.title, meeting.minutes_until));

        // Signal 2: Check if related entities (docs, tickets) haven't been accessed
        let related_entities = schema::get_relationships(conn, &meeting.entity_id);
        let mut unaccessed_items = Vec::new();

        for rel in &related_entities {
            // Check if related entity was accessed in the last hour (any event type)
            let recent_access: i64 = conn.query_row(
                "SELECT COUNT(*) FROM cortex_pulses p
                 JOIN cortex_pulse_entities pe ON pe.pulse_id = p.id
                 WHERE pe.entity_id = ?1 AND p.ts >= ?2",
                rusqlite::params![&rel.target_id, now - 3600.0],
                |row| row.get(0),
            ).unwrap_or(0);
            if recent_access == 0 {
                // Get entity display name
                if let Ok(Some(name)) = get_entity_name(conn, &rel.target_id) {
                    unaccessed_items.push(name);
                }
            }
        }

        if !unaccessed_items.is_empty() {
            evidence_count += 1;
            evidence_details.push(format!(
                "{} related item(s) not accessed: {}",
                unaccessed_items.len(),
                unaccessed_items.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
            ));
        }

        // Signal 3: Check if there are attendee entities with recent activity
        let attendee_entities: Vec<String> = related_entities.iter()
            .filter(|r| r.rel_type == "attends" || r.rel_type == "invited_to" || r.rel_type == "organizer")
            .map(|r| r.target_id.clone())
            .collect();

        if !attendee_entities.is_empty() {
            // Check if any attendees have recent emails or tickets
            for attendee_id in attendee_entities.iter().take(5) {
                let recent_emails = schema::count_recent_pulses(
                    conn,
                    attendee_id,
                    "email_received",
                    now - 86400.0, // last 24h
                );
                if recent_emails > 0 {
                    evidence_count += 1;
                    if let Ok(Some(name)) = get_entity_name(conn, attendee_id) {
                        evidence_details.push(format!("{} has recent emails", name));
                    }
                    break; // One attendee signal is enough
                }
            }
        }

        // Require at least 2 evidence signals to fire
        if evidence_count < 2 {
            continue;
        }

        let explanation = format!(
            "Meeting prep needed: {}",
            evidence_details.join("; ")
        );

        actions.push(CortexAction::Notify {
            title: format!("📅 {} in {} min", meeting.title, meeting.minutes_until),
            body: if unaccessed_items.is_empty() {
                "Meeting is approaching. You may want to review related materials.".to_string()
            } else {
                format!(
                    "You haven't looked at {} yet. Might want to review before the meeting.",
                    unaccessed_items.first().unwrap_or(&"the prep materials".to_string())
                )
            },
            explanation,
            playbook_id: "meeting_prep".to_string(),
        });
    }

    actions
}

// ─── Helpers ────────────────────────────────────────────────────────────────

struct UpcomingMeeting {
    entity_id: String,
    title: String,
    minutes_until: u32,
}

/// Find meetings happening within the next `window_secs` seconds.
/// Looks at calendar-related cortex entities and recent MeetingScheduled pulses.
fn find_upcoming_meetings(conn: &rusqlite::Connection, now: f64, window_secs: f64) -> Vec<UpcomingMeeting> {
    let mut meetings = Vec::new();

    // Strategy 1: Check the emails table for calendar-linked events
    // (Calendar events get ingested as cortex entities via calendar tools)

    // Strategy 2: Look for Meeting entities with recent pulses
    // that have a start time within the window
    let query = "
        SELECT e.id, e.display_name, e.attributes
        FROM cortex_entities e
        WHERE e.entity_type = 'meeting'
          AND e.relevance > 0.1
        ORDER BY e.last_seen_ts DESC
        LIMIT 10
    ";

    if let Ok(mut stmt) = conn.prepare(query) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        }) {
            for row in rows.flatten() {
                let (entity_id, display_name, attrs_json) = row;

                // Try to extract start time from attributes JSON
                let start_ts = if let Some(ref attrs) = attrs_json {
                    if let Ok(attrs) = serde_json::from_str::<serde_json::Value>(attrs) {
                        attrs.get("start_ts")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0)
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };

                // If we have a start time, check if it's within the window
                if start_ts > now && start_ts < now + window_secs {
                    let minutes_until = ((start_ts - now) / 60.0) as u32;
                    meetings.push(UpcomingMeeting {
                        entity_id,
                        title: display_name,
                        minutes_until,
                    });
                }
            }
        }
    }

    // Strategy 3: Check the calendar events table directly (if populated)
    if meetings.is_empty() {
        let cal_query = "
            SELECT id, summary, start
            FROM calendar_events
            WHERE start > datetime('now')
              AND start < datetime('now', '+30 minutes')
            ORDER BY start ASC
            LIMIT 5
        ";

        if let Ok(mut stmt) = conn.prepare(cal_query) {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            }) {
                for row in rows.flatten() {
                    // Calendar events are stored via Google Calendar API
                    // Use the entity ID format for consistency
                    let entity_id = format!("meeting:{}", row.0);
                    meetings.push(UpcomingMeeting {
                        entity_id,
                        title: row.1,
                        minutes_until: 30, // approximate
                    });
                }
            }
        }
        // Silently continue if table doesn't exist
    }

    meetings
}

/// Get display name of a cortex entity.
fn get_entity_name(conn: &rusqlite::Connection, entity_id: &str) -> Result<Option<String>, rusqlite::Error> {
    conn.query_row(
        "SELECT display_name FROM cortex_entities WHERE id = ?1",
        rusqlite::params![entity_id],
        |row| row.get(0),
    ).map(Some).or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        _ => Err(e),
    })
}
