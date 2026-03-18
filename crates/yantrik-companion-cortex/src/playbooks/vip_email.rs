//! VIP Email Escalation playbook — surfaces unread emails from important
//! senders that have been sitting too long.
//!
//! Evidence signals (need 2+ for high conviction):
//! 1. Unread email older than 1 hour
//! 2. Sender is high-importance (flagged, frequent contact, boss)
//! 3. Subject contains urgency markers or is a reply chain
//!
//! Action: Notify with sender + subject, no auto-action.

use crate::playbook::{CortexAction, PlaybookState};

/// Evaluate unread VIP emails. Pure Rust, no LLM.
pub fn evaluate(state: &PlaybookState) -> Vec<CortexAction> {
    let conn = state.conn;
    let now = state.now_ts;

    // Find unread emails older than 1 hour from the emails table
    let one_hour_ago = now - 3600.0;
    let twelve_hours_ago = now - 43200.0; // Don't nag about very old emails

    let query = "
        SELECT e.id, e.from_name, e.from_addr, e.subject, e.date_ts,
               e.is_flagged, e.importance
        FROM emails e
        WHERE e.is_read = 0
          AND e.folder = 'INBOX'
          AND e.date_ts < ?1
          AND e.date_ts > ?2
        ORDER BY e.date_ts DESC
        LIMIT 10
    ";

    let emails: Vec<UnreadEmail> = if let Ok(mut stmt) = conn.prepare(query) {
        stmt.query_map(
            rusqlite::params![one_hour_ago, twelve_hours_ago],
            |row| {
                Ok(UnreadEmail {
                    id: row.get(0)?,
                    from_name: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    from_addr: row.get(2)?,
                    subject: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    date_ts: row.get(4)?,
                    is_flagged: row.get::<_, i32>(5)? != 0,
                    importance: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "normal".to_string()),
                })
            },
        )
        .ok()
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    } else {
        return vec![]; // emails table doesn't exist
    };

    if emails.is_empty() {
        return vec![];
    }

    let mut actions = Vec::new();

    for email in emails {
        let mut evidence_count = 0u32;
        let mut evidence_details = Vec::new();

        // Signal 1: Unread for over 1 hour (always true — query filtered)
        let age_hours = (now - email.date_ts) / 3600.0;
        evidence_count += 1;
        evidence_details.push(format!("unread for {:.1}h", age_hours));

        // Signal 2: Sender importance
        let is_vip = email.is_flagged
            || email.importance == "high"
            || is_frequent_sender(conn, &email.from_addr, now)
            || is_known_vip_sender(conn, &email.from_addr);

        if is_vip {
            evidence_count += 1;
            let reason = if email.is_flagged {
                "flagged email"
            } else if email.importance == "high" {
                "high importance"
            } else {
                "frequent sender"
            };
            evidence_details.push(reason.to_string());
        }

        // Signal 3: Urgency markers in subject
        let urgency_markers = ["urgent", "asap", "action required", "deadline",
                               "important", "time sensitive", "please respond",
                               "follow up", "reminder", "overdue"];
        let subject_lower = email.subject.to_lowercase();
        let has_urgency = urgency_markers.iter().any(|m| subject_lower.contains(m));

        if has_urgency {
            evidence_count += 1;
            evidence_details.push("urgency markers in subject".to_string());
        }

        // Signal 3b: Reply chain (Re: prefix suggests ongoing conversation)
        if subject_lower.starts_with("re:") || subject_lower.starts_with("fwd:") {
            evidence_count += 1;
            evidence_details.push("part of active thread".to_string());
        }

        // Require at least 2 evidence signals
        if evidence_count < 2 {
            continue;
        }

        let sender_display = if email.from_name.is_empty() {
            &email.from_addr
        } else {
            &email.from_name
        };

        let explanation = format!(
            "VIP email escalation: {}",
            evidence_details.join("; ")
        );

        actions.push(CortexAction::Notify {
            title: format!("📧 Unread from {}", sender_display),
            body: format!(
                "{}\n({:.0}h ago)",
                truncate(&email.subject, 80),
                age_hours
            ),
            explanation,
            playbook_id: "vip_email".to_string(),
        });

        // Only escalate the most important unread email per cycle
        break;
    }

    actions
}

// ─── Helpers ────────────────────────────────────────────────────────────────

#[allow(dead_code)]
struct UnreadEmail {
    id: i64,
    from_name: String,
    from_addr: String,
    subject: String,
    date_ts: f64,
    is_flagged: bool,
    importance: String,
}

/// Check if sender has sent 5+ emails in the last 30 days.
fn is_frequent_sender(conn: &rusqlite::Connection, from_addr: &str, now: f64) -> bool {
    let thirty_days_ago = now - 30.0 * 86400.0;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM emails WHERE from_addr = ?1 AND date_ts > ?2",
        rusqlite::params![from_addr, thirty_days_ago],
        |row| row.get(0),
    ).unwrap_or(0);
    count >= 5
}

/// Check if sender is a known person entity with high relevance.
fn is_known_vip_sender(conn: &rusqlite::Connection, from_addr: &str) -> bool {
    // Check if sender maps to a cortex person entity with high relevance
    let result: Option<f64> = conn.query_row(
        "SELECT e.relevance FROM cortex_entities e
         WHERE e.entity_type = 'person'
           AND (e.id LIKE '%' || ?1 || '%'
                OR e.display_name LIKE '%' || ?1 || '%')
         ORDER BY e.relevance DESC LIMIT 1",
        rusqlite::params![from_addr.split('@').next().unwrap_or(from_addr)],
        |row| row.get(0),
    ).ok();

    result.map(|r| r > 0.5).unwrap_or(false)
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        &s[..s.char_indices().take(max_len).last().map(|(i, _)| i).unwrap_or(0)]
    }
}
