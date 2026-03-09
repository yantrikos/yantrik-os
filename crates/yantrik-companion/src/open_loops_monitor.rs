//! Open Loops Monitor — scans commitments and attention items for unresolved items.
//!
//! Periodically (called from the maintenance cycle):
//! 1. Transitions overdue commitments → creates/updates life threads
//! 2. Detects approaching deadlines → creates life threads with urgency
//! 3. Scans the unified `attention_items` table for unhandled items across
//!    all channels (email, WhatsApp, Telegram, SMS, calls, etc.)
//! 4. Promotes pending attention items → life threads
//! 5. Emits `SignificantEvent::CommitmentAlert` for event-driven activation
//!
//! This module is pure logic — no timers, no threads.
//! The caller (stewardship loop or maintenance tick) invokes `scan()`.

use rusqlite::Connection;

use crate::event_driven::SignificantEvent;
use crate::world_model::{
    self, ThreadStatus, ThreadType, WorldModel,
};

// ── Configuration ───────────────────────────────────────────────────

/// Configuration for the open loops monitor.
#[derive(Debug, Clone)]
pub struct MonitorConfig {
    /// Hours before deadline to flag as "approaching" (default: 24).
    pub approaching_hours: f64,
    /// Hours before deadline to flag as "urgent" (default: 4).
    pub urgent_hours: f64,
    /// Seconds an attention item must wait before promotion to life thread.
    /// Default: 2 days. This applies across all channels.
    pub attention_min_age_secs: f64,
    /// Maximum life threads to create per scan (prevent flood).
    pub max_threads_per_scan: usize,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            approaching_hours: 24.0,
            urgent_hours: 4.0,
            attention_min_age_secs: 2.0 * 86400.0, // 2 days
            max_threads_per_scan: 10,
        }
    }
}

// ── Scan result ─────────────────────────────────────────────────────

/// Result of a single monitor scan cycle.
#[derive(Debug, Default)]
pub struct ScanResult {
    /// Number of commitments transitioned to overdue.
    pub overdue_transitioned: usize,
    /// Life threads created or updated for approaching deadlines.
    pub approaching_threads: usize,
    /// Life threads created or updated for overdue commitments.
    pub overdue_threads: usize,
    /// Life threads created from attention items (any channel).
    pub attention_threads: usize,
    /// Significant events to emit (commitment alerts).
    pub events: Vec<SignificantEvent>,
}

// ── Core scan ───────────────────────────────────────────────────────

/// Run a full open-loops scan. Returns events to feed into EventDrivenState.
pub fn scan(conn: &Connection, config: &MonitorConfig) -> ScanResult {
    let mut result = ScanResult::default();

    // 1. Transition overdue commitments in wm_commitments table
    let overdue_list = WorldModel::check_overdue(conn);
    result.overdue_transitioned = overdue_list.len();

    // 2. Transition overdue life threads
    let _thread_overdue = world_model::transition_overdue(conn);
    world_model::refresh_days_open(conn);

    // 3. Create life threads for overdue commitments
    for c in &overdue_list {
        if result.overdue_threads >= config.max_threads_per_scan {
            break;
        }
        let entity_id = format!("commitment:{}", c.id);
        let ctx = serde_json::json!({
            "action": c.action,
            "promisor": c.promisor,
            "promisee": c.promisee,
            "deadline": c.deadline,
            "status": "overdue",
        });
        world_model::upsert_life_thread(
            conn,
            &ThreadType::Commitment,
            &entity_id,
            &format!("OVERDUE: {}", truncate(&c.action, 60)),
            c.deadline,
            0.9,
            &format!("commitment_monitor:{}", c.source.type_tag()),
            &ctx.to_string(),
        );
        result.overdue_threads += 1;
        result.events.push(SignificantEvent::CommitmentAlert {
            description: format!("Overdue: {}", truncate(&c.action, 80)),
        });
    }

    // 4. Create life threads for approaching deadlines
    let approaching = WorldModel::approaching_deadlines(conn, config.approaching_hours);
    for c in &approaching {
        if result.approaching_threads >= config.max_threads_per_scan {
            break;
        }
        let entity_id = format!("commitment:{}", c.id);
        let hours_left = (c.deadline - now_ts()) / 3600.0;
        let importance = if hours_left <= config.urgent_hours {
            0.85
        } else {
            0.6
        };
        let urgency = if hours_left <= config.urgent_hours {
            "URGENT"
        } else {
            "Approaching"
        };
        let ctx = serde_json::json!({
            "action": c.action,
            "promisor": c.promisor,
            "promisee": c.promisee,
            "deadline": c.deadline,
            "hours_left": format!("{:.1}", hours_left),
            "urgency": urgency,
        });
        world_model::upsert_life_thread(
            conn,
            &ThreadType::Commitment,
            &entity_id,
            &format!("{}: {} ({:.0}h left)", urgency, truncate(&c.action, 50), hours_left),
            c.deadline,
            importance,
            "commitment_monitor",
            &ctx.to_string(),
        );
        result.approaching_threads += 1;

        if hours_left <= config.urgent_hours {
            result.events.push(SignificantEvent::CommitmentAlert {
                description: format!(
                    "{}: {} ({:.0}h left)",
                    urgency,
                    truncate(&c.action, 60),
                    hours_left
                ),
            });
        }
    }

    // 5. Scan attention items (all channels) → promote to life threads
    result.attention_threads = scan_attention_items(conn, config);

    result
}

// ── Attention item scanner (channel-agnostic) ───────────────────────

/// Scan the unified `attention_items` table for unhandled items across
/// all channels. Promotes old enough items to life threads.
fn scan_attention_items(conn: &Connection, config: &MonitorConfig) -> usize {
    let items = world_model::query_pending_attention(
        conn,
        config.attention_min_age_secs,
        config.max_threads_per_scan,
    );

    let mut count = 0;
    for item in &items {
        let thread_type = ThreadType::from_str(&item.channel);
        let entity_id = format!("{}:{}", item.channel, item.external_id);

        // Skip if already resolved/archived as a life thread
        let existing = world_model::query_threads_by_type(conn, &thread_type, 200);
        let already_terminal = existing.iter().any(|t| {
            t.entity_id == entity_id && t.status.is_terminal()
        });
        if already_terminal {
            world_model::mark_attention_handled(conn, item.id);
            continue;
        }

        let days_ago = (now_ts() - item.received_ts) / 86400.0;
        let display_name = if item.sender_name.is_empty() {
            &item.sender
        } else {
            &item.sender_name
        };
        let importance = match item.importance.as_str() {
            "urgent" => 0.95,
            "high" => 0.8,
            "normal" => 0.5,
            _ => 0.3,
        };

        let label = format!(
            "Unanswered {} from {}: {}",
            item.channel,
            truncate(display_name, 20),
            truncate(&item.subject, 40),
        );

        let ctx = serde_json::json!({
            "channel": item.channel,
            "from": item.sender,
            "sender_name": display_name,
            "subject": item.subject,
            "preview": truncate(&item.preview, 200),
            "received_ts": item.received_ts,
            "days_unanswered": format!("{:.1}", days_ago),
            "importance": item.importance,
        });

        world_model::upsert_life_thread(
            conn,
            &thread_type,
            &entity_id,
            &label,
            0.0, // no deadline for messages
            importance,
            &format!("{}_monitor", item.channel),
            &ctx.to_string(),
        );

        world_model::mark_attention_handled(conn, item.id);
        count += 1;
    }

    count
}

// ── Summary for LLM context ────────────────────────────────────────

/// Generate a concise open-loops summary suitable for LLM context injection.
pub fn open_loops_summary(conn: &Connection, max_items: usize) -> String {
    let threads = world_model::query_open_threads(conn, max_items);
    if threads.is_empty() {
        return String::new();
    }

    let mut lines = Vec::new();
    lines.push(format!("Open loops ({}):", threads.len()));

    for t in &threads {
        let status_icon = match t.status {
            ThreadStatus::Overdue => "🔴",
            ThreadStatus::Stalled => "🟡",
            _ => "🔵",
        };
        lines.push(format!(
            "  {} [{}] {}",
            status_icon,
            t.thread_type.label(),
            t.label
        ));
    }

    lines.join("\n")
}

/// Attention summary per channel (for dashboards).
pub fn channel_summary(conn: &Connection) -> Vec<(String, i64)> {
    world_model::attention_summary(conn)
}

// ── Helpers ─────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        &s[..end]
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_model::{Commitment, CommitmentSource, WorldModel};

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        WorldModel::init_tables(&conn);
        conn
    }

    fn now_ts_test() -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
    }

    fn insert_commitment(conn: &Connection, action: &str, deadline: f64, status: crate::world_model::CommitmentStatus) -> i64 {
        let now = now_ts_test();
        let c = Commitment {
            id: 0,
            promisor: "Pranab".into(),
            promisee: "Team".into(),
            action: action.into(),
            deadline,
            status,
            confidence: 0.8,
            source: CommitmentSource::Conversation { turn_id: None },
            evidence_text: "test evidence".into(),
            related_entities: vec![],
            created_at: now,
            updated_at: now,
            completion_evidence: None,
        };
        WorldModel::insert_commitment(conn, &c)
    }

    #[test]
    fn scan_detects_overdue_commitments() {
        let conn = setup_db();
        let past = now_ts_test() - 3600.0;
        insert_commitment(&conn, "Submit quarterly report", past, crate::world_model::CommitmentStatus::Pending);

        let config = MonitorConfig::default();
        let result = scan(&conn, &config);

        assert!(result.overdue_transitioned > 0 || result.overdue_threads > 0);
        let threads = world_model::query_open_threads(&conn, 10);
        assert!(!threads.is_empty(), "Should create life thread for overdue commitment");
    }

    #[test]
    fn scan_detects_approaching_deadlines() {
        let conn = setup_db();
        let future_3h = now_ts_test() + 3.0 * 3600.0;
        insert_commitment(&conn, "Review PR before merge", future_3h, crate::world_model::CommitmentStatus::Pending);

        let config = MonitorConfig {
            approaching_hours: 24.0,
            urgent_hours: 4.0,
            ..Default::default()
        };
        let result = scan(&conn, &config);

        assert!(result.approaching_threads > 0, "Should detect approaching deadline");
        assert!(!result.events.is_empty(), "Should emit urgent commitment alert");
    }

    #[test]
    fn scan_approaching_not_urgent_no_event() {
        let conn = setup_db();
        let future_12h = now_ts_test() + 12.0 * 3600.0;
        insert_commitment(&conn, "Prepare presentation slides", future_12h, crate::world_model::CommitmentStatus::Pending);

        let config = MonitorConfig {
            approaching_hours: 24.0,
            urgent_hours: 4.0,
            ..Default::default()
        };
        let result = scan(&conn, &config);

        assert!(result.approaching_threads > 0);
        assert!(result.events.is_empty(), "Should NOT emit event for non-urgent approaching");
    }

    #[test]
    fn scan_ignores_completed_commitments() {
        let conn = setup_db();
        let past = now_ts_test() - 3600.0;
        let id = insert_commitment(&conn, "Already done task", past, crate::world_model::CommitmentStatus::Pending);
        WorldModel::update_commitment_status(&conn, id, crate::world_model::CommitmentStatus::Completed, Some("done"));

        let config = MonitorConfig::default();
        let result = scan(&conn, &config);

        assert_eq!(result.overdue_threads, 0, "Completed commitments should not generate threads");
    }

    // ── Attention items tests (channel-agnostic) ──

    #[test]
    fn attention_email_creates_thread() {
        let conn = setup_db();
        let old_ts = now_ts_test() - 5.0 * 86400.0; // 5 days ago
        world_model::upsert_attention_item(
            &conn, "email", "msg-001",
            "alice@example.com", "Alice",
            "Project update needed", "Hi, can you send the update?",
            old_ts, "normal", true, "{}",
        );

        let config = MonitorConfig {
            attention_min_age_secs: 2.0 * 86400.0,
            ..Default::default()
        };
        let result = scan(&conn, &config);

        assert!(result.attention_threads > 0, "Should detect unanswered email via attention_items");
        let threads = world_model::query_threads_by_type(&conn, &ThreadType::Email, 10);
        assert!(!threads.is_empty());
        assert!(threads[0].label.contains("Alice"));
    }

    #[test]
    fn attention_whatsapp_creates_thread() {
        let conn = setup_db();
        let old_ts = now_ts_test() - 3.0 * 86400.0; // 3 days ago
        world_model::upsert_attention_item(
            &conn, "whatsapp", "wa-msg-42",
            "+919876543210", "Rahul",
            "", "Hey, are we still meeting tomorrow?",
            old_ts, "high", true, "{}",
        );

        let config = MonitorConfig {
            attention_min_age_secs: 2.0 * 86400.0,
            ..Default::default()
        };
        let result = scan(&conn, &config);

        assert!(result.attention_threads > 0, "Should detect unanswered WhatsApp");
        let threads = world_model::query_threads_by_type(&conn, &ThreadType::WhatsApp, 10);
        assert!(!threads.is_empty());
        assert!(threads[0].label.contains("Rahul"));
    }

    #[test]
    fn attention_telegram_creates_thread() {
        let conn = setup_db();
        let old_ts = now_ts_test() - 4.0 * 86400.0;
        world_model::upsert_attention_item(
            &conn, "telegram", "tg-msg-99",
            "@devops_bot", "DevOps Bot",
            "Deploy failed", "Build #42 failed on staging",
            old_ts, "urgent", true, "{}",
        );

        let config = MonitorConfig::default();
        let result = scan(&conn, &config);

        assert!(result.attention_threads > 0);
        let threads = world_model::query_threads_by_type(&conn, &ThreadType::Telegram, 10);
        assert!(!threads.is_empty());
    }

    #[test]
    fn attention_missed_call_creates_thread() {
        let conn = setup_db();
        let old_ts = now_ts_test() - 3.0 * 86400.0;
        world_model::upsert_attention_item(
            &conn, "call", "call-2026-03-06-1430",
            "+14155551234", "Mom",
            "Missed call", "",
            old_ts, "high", true,
            r#"{"type":"missed","duration":0}"#,
        );

        let config = MonitorConfig::default();
        let result = scan(&conn, &config);

        assert!(result.attention_threads > 0);
        let threads = world_model::query_threads_by_type(&conn, &ThreadType::Call, 10);
        assert!(!threads.is_empty());
        assert!(threads[0].label.contains("Mom"));
    }

    #[test]
    fn attention_skips_recent_items() {
        let conn = setup_db();
        let recent = now_ts_test() - 3600.0; // 1 hour ago
        world_model::upsert_attention_item(
            &conn, "email", "msg-new",
            "bob@example.com", "Bob",
            "Quick question", "Hey",
            recent, "normal", true, "{}",
        );

        let config = MonitorConfig {
            attention_min_age_secs: 2.0 * 86400.0,
            ..Default::default()
        };
        let result = scan(&conn, &config);

        assert_eq!(result.attention_threads, 0, "Recent items should not be promoted");
    }

    #[test]
    fn attention_skips_replied_items() {
        let conn = setup_db();
        let old_ts = now_ts_test() - 5.0 * 86400.0;
        world_model::upsert_attention_item(
            &conn, "email", "msg-replied",
            "alice@example.com", "Alice",
            "Follow up", "Did you check?",
            old_ts, "normal", true, "{}",
        );
        world_model::mark_attention_replied(&conn, "email", "msg-replied");

        let config = MonitorConfig::default();
        let result = scan(&conn, &config);

        assert_eq!(result.attention_threads, 0, "Replied items should not be promoted");
    }

    #[test]
    fn attention_marks_handled_after_promotion() {
        let conn = setup_db();
        let old_ts = now_ts_test() - 5.0 * 86400.0;
        world_model::upsert_attention_item(
            &conn, "text", "sms-001",
            "+919876543210", "Dad",
            "", "Call me when free",
            old_ts, "high", true, "{}",
        );

        let config = MonitorConfig::default();
        scan(&conn, &config);

        // Second scan should not re-promote
        let result2 = scan(&conn, &config);
        assert_eq!(result2.attention_threads, 0, "Handled items should not be re-promoted");
    }

    #[test]
    fn open_loops_summary_empty() {
        let conn = setup_db();
        let summary = open_loops_summary(&conn, 10);
        assert!(summary.is_empty());
    }

    #[test]
    fn open_loops_summary_with_threads() {
        let conn = setup_db();
        world_model::upsert_life_thread(
            &conn,
            &ThreadType::Commitment,
            "commitment:1",
            "OVERDUE: Submit report",
            now_ts_test() - 3600.0,
            0.9, "test", "{}",
        );
        world_model::upsert_life_thread(
            &conn,
            &ThreadType::WhatsApp,
            "whatsapp:wa-42",
            "Unanswered whatsapp from Rahul",
            0.0, 0.5, "test", "{}",
        );

        let summary = open_loops_summary(&conn, 10);
        assert!(summary.contains("Open loops"));
        assert!(summary.contains("commitment"));
        assert!(summary.contains("whatsapp"));
    }

    #[test]
    fn channel_summary_counts() {
        let conn = setup_db();
        let old_ts = now_ts_test() - 5.0 * 86400.0;
        world_model::upsert_attention_item(&conn, "email", "e1", "a@b.com", "A", "s1", "p1", old_ts, "normal", true, "{}");
        world_model::upsert_attention_item(&conn, "email", "e2", "c@d.com", "C", "s2", "p2", old_ts, "normal", true, "{}");
        world_model::upsert_attention_item(&conn, "whatsapp", "w1", "+91", "X", "", "hi", old_ts, "normal", true, "{}");

        let summary = channel_summary(&conn);
        assert_eq!(summary.len(), 2);
        // email should be first with count 2
        assert_eq!(summary[0].0, "email");
        assert_eq!(summary[0].1, 2);
        assert_eq!(summary[1].0, "whatsapp");
        assert_eq!(summary[1].1, 1);
    }

    #[test]
    fn max_threads_per_scan_respected() {
        let conn = setup_db();
        for i in 0..5 {
            let past = now_ts_test() - (i as f64 + 1.0) * 3600.0;
            insert_commitment(&conn, &format!("Task {}", i), past, crate::world_model::CommitmentStatus::Pending);
        }

        let config = MonitorConfig {
            max_threads_per_scan: 2,
            ..Default::default()
        };
        let result = scan(&conn, &config);

        assert!(result.overdue_threads <= 2, "Should respect max_threads_per_scan");
    }
}
