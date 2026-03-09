//! World Model — typed entities with confidence, provenance, and temporal tracking.
//!
//! Builds on the Cortex entity system to add first-class support for:
//! - **Commitments**: Promises extracted from conversations, emails, calendar events
//! - **Preferences**: User preferences with scope, strength, and contradiction tracking
//! - **Routines**: Repeating behavioral patterns with confidence and schedule
//!
//! Person entities are already tracked by `cortex/entity.rs` — this module
//! adds the richer attributes and cross-references.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

// ── Commitment ──────────────────────────────────────────────────────

/// Lifecycle status of a commitment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitmentStatus {
    /// Extracted but not yet acted on.
    Pending,
    /// User acknowledged and is working on it.
    InProgress,
    /// Fulfilled — action completed.
    Completed,
    /// Deadline passed without completion.
    Overdue,
    /// User explicitly cancelled or deemed irrelevant.
    Cancelled,
    /// Commitment was fulfilled by someone else or became moot.
    Superseded,
}

impl CommitmentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Overdue => "overdue",
            Self::Cancelled => "cancelled",
            Self::Superseded => "superseded",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "overdue" => Self::Overdue,
            "cancelled" => Self::Cancelled,
            "superseded" => Self::Superseded,
            _ => Self::Pending,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Superseded)
    }
}

/// Where a commitment was extracted from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommitmentSource {
    /// Extracted from a chat conversation.
    Conversation { turn_id: Option<String> },
    /// Extracted from an email.
    Email { message_id: String, subject: String },
    /// Extracted from a calendar event.
    Calendar { event_id: String, title: String },
    /// Manually created by the user.
    Manual,
}

impl CommitmentSource {
    pub fn type_tag(&self) -> &'static str {
        match self {
            Self::Conversation { .. } => "conversation",
            Self::Email { .. } => "email",
            Self::Calendar { .. } => "calendar",
            Self::Manual => "manual",
        }
    }
}

/// A tracked promise or action item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commitment {
    pub id: i64,
    /// Who made the promise (entity ID or free text).
    pub promisor: String,
    /// Who the promise was made to (entity ID or free text).
    pub promisee: String,
    /// What was promised — the action description.
    pub action: String,
    /// When the commitment should be fulfilled (Unix timestamp, 0 = no deadline).
    pub deadline: f64,
    /// Current status.
    pub status: CommitmentStatus,
    /// Extraction confidence (0.0 – 1.0).
    pub confidence: f64,
    /// Where this was extracted from.
    pub source: CommitmentSource,
    /// The original text that contained the commitment.
    pub evidence_text: String,
    /// Related cortex entity IDs.
    pub related_entities: Vec<String>,
    /// When the commitment was first detected.
    pub created_at: f64,
    /// When the status was last updated.
    pub updated_at: f64,
    /// Completion evidence (what action fulfilled this).
    pub completion_evidence: Option<String>,
}

// ── Preference ──────────────────────────────────────────────────────

/// How strongly a preference is held.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PreferenceStrength {
    /// Casually mentioned once.
    Weak,
    /// Mentioned multiple times or with emphasis.
    Moderate,
    /// Explicitly stated as important or non-negotiable.
    Strong,
}

impl PreferenceStrength {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Weak => "weak",
            Self::Moderate => "moderate",
            Self::Strong => "strong",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "moderate" => Self::Moderate,
            "strong" => Self::Strong,
            _ => Self::Weak,
        }
    }

    pub fn as_f64(&self) -> f64 {
        match self {
            Self::Weak => 0.3,
            Self::Moderate => 0.6,
            Self::Strong => 0.9,
        }
    }
}

/// Scope where a preference applies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PreferenceScope {
    /// Applies everywhere.
    Global,
    /// Applies to a specific domain (e.g., "food", "music", "work").
    Domain(String),
    /// Applies to interactions with a specific person.
    Person(String),
    /// Applies to a specific tool or feature.
    Tool(String),
}

impl PreferenceScope {
    pub fn as_str(&self) -> String {
        match self {
            Self::Global => "global".into(),
            Self::Domain(d) => format!("domain:{d}"),
            Self::Person(p) => format!("person:{p}"),
            Self::Tool(t) => format!("tool:{t}"),
        }
    }

    pub fn from_str(s: &str) -> Self {
        if let Some(d) = s.strip_prefix("domain:") {
            Self::Domain(d.into())
        } else if let Some(p) = s.strip_prefix("person:") {
            Self::Person(p.into())
        } else if let Some(t) = s.strip_prefix("tool:") {
            Self::Tool(t.into())
        } else {
            Self::Global
        }
    }
}

/// A user preference with provenance and contradiction tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preference {
    pub id: i64,
    /// Category (e.g., "food", "communication", "workflow", "music").
    pub domain: String,
    /// The preference key (e.g., "coffee_type", "meeting_style").
    pub key: String,
    /// The preference value (e.g., "black coffee", "async-first").
    pub value: String,
    /// How strongly held.
    pub strength: PreferenceStrength,
    /// Where this preference applies.
    pub scope: PreferenceScope,
    /// Number of times this preference was observed.
    pub observation_count: u32,
    /// Evidence: when was this last confirmed.
    pub last_observed_at: f64,
    /// When first observed.
    pub created_at: f64,
    /// Contradictions: other preferences that conflict with this one.
    pub contradictions: Vec<String>,
    /// Whether this preference is currently active (not superseded).
    pub active: bool,
}

// ── Routine ─────────────────────────────────────────────────────────

/// A repeating behavioral pattern detected from user activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routine {
    pub id: i64,
    /// Human-readable description (e.g., "Morning email check").
    pub description: String,
    /// What triggers this routine (time of day, event, etc.).
    pub trigger: RoutineTrigger,
    /// What the user typically does (sequence of actions/tools).
    pub action_sequence: Vec<String>,
    /// How many times this pattern has been observed.
    pub observation_count: u32,
    /// Statistical confidence that this is a real routine (0.0 – 1.0).
    pub confidence: f64,
    /// Average duration in minutes.
    pub avg_duration_min: f64,
    /// When this routine was first detected.
    pub created_at: f64,
    /// Last time this routine was observed.
    pub last_observed_at: f64,
    /// Whether the routine is still active (observed recently).
    pub active: bool,
}

/// What triggers a routine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoutineTrigger {
    /// Time-based (hour of day, day of week).
    TimeOfDay { hour: u8, days: Vec<String> },
    /// Event-based (after receiving email, after meeting, etc.).
    Event { event_type: String },
    /// Location-based (at home, at work).
    Location { place: String },
    /// Sequence-based (after completing another routine).
    After { routine_id: i64 },
}

impl RoutineTrigger {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> Self {
        serde_json::from_str(s).unwrap_or(Self::TimeOfDay {
            hour: 9,
            days: vec!["weekday".into()],
        })
    }
}

// ── Life Thread ─────────────────────────────────────────────────────

/// The category of a life thread.
///
/// Covers any communication channel or tracked item type.
/// The `life_threads` table is the unified attention table —
/// any source (email, WhatsApp, call, text, file, etc.) can
/// create threads here for the companion to track.
#[derive(Debug, Clone, PartialEq)]
pub enum ThreadType {
    Email,
    Commitment,
    File,
    Person,
    Task,
    /// WhatsApp message needing reply.
    WhatsApp,
    /// Phone call — missed or needing callback.
    Call,
    /// SMS / text message needing reply.
    Text,
    /// Telegram message needing reply.
    Telegram,
    /// Calendar event needing action (RSVP, prepare, etc.).
    Calendar,
    /// Generic / extensible type via string tag.
    Custom(String),
}

impl ThreadType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Email => "email",
            Self::Commitment => "commitment",
            Self::File => "file",
            Self::Person => "person",
            Self::Task => "task",
            Self::WhatsApp => "whatsapp",
            Self::Call => "call",
            Self::Text => "text",
            Self::Telegram => "telegram",
            Self::Calendar => "calendar",
            Self::Custom(s) => s.as_str(),
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "email" => Self::Email,
            "commitment" => Self::Commitment,
            "file" => Self::File,
            "person" => Self::Person,
            "task" => Self::Task,
            "whatsapp" => Self::WhatsApp,
            "call" => Self::Call,
            "text" => Self::Text,
            "telegram" => Self::Telegram,
            "calendar" => Self::Calendar,
            other => Self::Custom(other.to_string()),
        }
    }

    /// Human-readable label for display.
    pub fn label(&self) -> &str {
        match self {
            Self::Email => "email",
            Self::Commitment => "commitment",
            Self::File => "file",
            Self::Person => "person",
            Self::Task => "task",
            Self::WhatsApp => "whatsapp",
            Self::Call => "call",
            Self::Text => "text",
            Self::Telegram => "telegram",
            Self::Calendar => "calendar",
            Self::Custom(s) => s.as_str(),
        }
    }
}

impl std::fmt::Display for ThreadType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Lifecycle status of a life thread.
#[derive(Debug, Clone, PartialEq)]
pub enum ThreadStatus {
    Open,
    Stalled,
    Overdue,
    Resolved,
    Snoozed,
    Archived,
}

impl ThreadStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Stalled => "stalled",
            Self::Overdue => "overdue",
            Self::Resolved => "resolved",
            Self::Snoozed => "snoozed",
            Self::Archived => "archived",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "open" => Self::Open,
            "stalled" => Self::Stalled,
            "overdue" => Self::Overdue,
            "resolved" => Self::Resolved,
            "snoozed" => Self::Snoozed,
            "archived" => Self::Archived,
            _ => Self::Open,
        }
    }

    /// Whether this status is terminal (no further transitions expected).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Resolved | Self::Archived)
    }
}

impl std::fmt::Display for ThreadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A tracked thread of activity spanning emails, commitments, people, files, or tasks.
#[derive(Debug, Clone)]
pub struct LifeThread {
    pub id: i64,
    pub thread_type: ThreadType,
    pub entity_id: String,
    pub label: String,
    pub deadline_ts: f64,
    pub status: ThreadStatus,
    pub importance: f64,
    pub days_open: i64,
    pub last_action_ts: f64,
    pub snooze_until: f64,
    pub source: String,
    pub context_json: String,
    pub created_at: f64,
    pub updated_at: f64,
}

// ── Life Thread CRUD (standalone functions) ─────────────────────────

/// Upsert a life thread — INSERT or REPLACE keyed on (thread_type, entity_id).
pub fn upsert_life_thread(
    conn: &Connection,
    thread_type: &ThreadType,
    entity_id: &str,
    label: &str,
    deadline_ts: f64,
    importance: f64,
    source: &str,
    context_json: &str,
) -> i64 {
    let now = now_ts();
    // Try to find existing to preserve created_at
    let existing_created: Option<f64> = conn
        .query_row(
            "SELECT created_at FROM life_threads WHERE thread_type = ?1 AND entity_id = ?2",
            params![thread_type.as_str(), entity_id],
            |row| row.get(0),
        )
        .ok();
    let created = existing_created.unwrap_or(now);

    conn.execute(
        "INSERT INTO life_threads
         (thread_type, entity_id, label, deadline_ts, status, importance,
          days_open, last_action_ts, snooze_until, source, context_json,
          created_at, updated_at)
         VALUES (?1,?2,?3,?4,'open',?5,0,?6,0,?7,?8,?9,?10)
         ON CONFLICT(thread_type, entity_id) DO UPDATE SET
           label = excluded.label,
           deadline_ts = excluded.deadline_ts,
           importance = excluded.importance,
           last_action_ts = excluded.last_action_ts,
           source = excluded.source,
           context_json = excluded.context_json,
           updated_at = excluded.updated_at",
        params![
            thread_type.as_str(),
            entity_id,
            label,
            deadline_ts,
            importance,
            now,
            source,
            context_json,
            created,
            now,
        ],
    )
    .expect("failed to upsert life thread");
    conn.last_insert_rowid()
}

/// Update the status of a life thread.
pub fn update_thread_status(
    conn: &Connection,
    thread_type: &ThreadType,
    entity_id: &str,
    new_status: &ThreadStatus,
) {
    let now = now_ts();
    let _ = conn.execute(
        "UPDATE life_threads SET status = ?1, updated_at = ?2
         WHERE thread_type = ?3 AND entity_id = ?4",
        params![new_status.as_str(), now, thread_type.as_str(), entity_id],
    );
}

/// Resolve a life thread, storing evidence in context_json.
pub fn resolve_thread(
    conn: &Connection,
    thread_type: &ThreadType,
    entity_id: &str,
    evidence: &str,
) {
    let now = now_ts();
    // Merge evidence into existing context_json
    let existing_ctx: String = conn
        .query_row(
            "SELECT context_json FROM life_threads WHERE thread_type = ?1 AND entity_id = ?2",
            params![thread_type.as_str(), entity_id],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "{}".into());

    let new_ctx = if let Ok(mut map) =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&existing_ctx)
    {
        map.insert(
            "resolution_evidence".into(),
            serde_json::Value::String(evidence.into()),
        );
        serde_json::to_string(&map).unwrap_or_else(|_| existing_ctx.clone())
    } else {
        format!("{{\"resolution_evidence\":{}}}", serde_json::json!(evidence))
    };

    let _ = conn.execute(
        "UPDATE life_threads SET status = 'resolved', context_json = ?1, updated_at = ?2
         WHERE thread_type = ?3 AND entity_id = ?4",
        params![new_ctx, now, thread_type.as_str(), entity_id],
    );
}

/// Snooze a life thread until a given timestamp.
pub fn snooze_thread(
    conn: &Connection,
    thread_type: &ThreadType,
    entity_id: &str,
    until_ts: f64,
) {
    let now = now_ts();
    let _ = conn.execute(
        "UPDATE life_threads SET status = 'snoozed', snooze_until = ?1, updated_at = ?2
         WHERE thread_type = ?3 AND entity_id = ?4",
        params![until_ts, now, thread_type.as_str(), entity_id],
    );
}

/// Archive a life thread.
pub fn archive_thread(conn: &Connection, thread_type: &ThreadType, entity_id: &str) {
    let now = now_ts();
    let _ = conn.execute(
        "UPDATE life_threads SET status = 'archived', updated_at = ?1
         WHERE thread_type = ?2 AND entity_id = ?3",
        params![now, thread_type.as_str(), entity_id],
    );
}

/// Query open threads (open, stalled, overdue) that are not snoozed.
/// Ordered by importance DESC, then deadline ASC.
pub fn query_open_threads(conn: &Connection, limit: usize) -> Vec<LifeThread> {
    let now = now_ts();
    conn.prepare(
        "SELECT id, thread_type, entity_id, label, deadline_ts, status,
                importance, days_open, last_action_ts, snooze_until,
                source, context_json, created_at, updated_at
         FROM life_threads
         WHERE status IN ('open','stalled','overdue')
           AND (snooze_until = 0 OR snooze_until < ?1)
         ORDER BY importance DESC, deadline_ts ASC
         LIMIT ?2",
    )
    .and_then(|mut stmt| {
        stmt.query_map(params![now, limit as i64], |row| row_to_life_thread(row))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
    })
    .unwrap_or_default()
}

/// Query threads filtered by type.
pub fn query_threads_by_type(
    conn: &Connection,
    thread_type: &ThreadType,
    limit: usize,
) -> Vec<LifeThread> {
    conn.prepare(
        "SELECT id, thread_type, entity_id, label, deadline_ts, status,
                importance, days_open, last_action_ts, snooze_until,
                source, context_json, created_at, updated_at
         FROM life_threads
         WHERE thread_type = ?1
         ORDER BY importance DESC, updated_at DESC
         LIMIT ?2",
    )
    .and_then(|mut stmt| {
        stmt.query_map(params![thread_type.as_str(), limit as i64], |row| {
            row_to_life_thread(row)
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
    })
    .unwrap_or_default()
}

/// Quick count of open threads (open, stalled, overdue) for dashboard display.
pub fn count_open_threads(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM life_threads WHERE status IN ('open','stalled','overdue')",
        [],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

/// Refresh `days_open` for all non-terminal threads.
pub fn refresh_days_open(conn: &Connection) {
    let now = now_ts();
    let _ = conn.execute(
        "UPDATE life_threads
         SET days_open = CAST((?1 - created_at) / 86400.0 AS INTEGER)
         WHERE status NOT IN ('resolved', 'archived')",
        params![now],
    );
}

/// Transition threads with expired deadlines to 'overdue'. Returns count transitioned.
pub fn transition_overdue(conn: &Connection) -> usize {
    let now = now_ts();
    conn.execute(
        "UPDATE life_threads SET status = 'overdue', updated_at = ?1
         WHERE deadline_ts > 0 AND deadline_ts < ?2
         AND status IN ('open', 'stalled')",
        params![now, now],
    )
    .unwrap_or(0)
}

/// Map a row to a LifeThread struct.
fn row_to_life_thread(row: &rusqlite::Row) -> rusqlite::Result<LifeThread> {
    Ok(LifeThread {
        id: row.get(0)?,
        thread_type: ThreadType::from_str(&row.get::<_, String>(1).unwrap_or_default()),
        entity_id: row.get(2)?,
        label: row.get(3)?,
        deadline_ts: row.get(4)?,
        status: ThreadStatus::from_str(&row.get::<_, String>(5).unwrap_or_default()),
        importance: row.get(6)?,
        days_open: row.get(7)?,
        last_action_ts: row.get(8)?,
        snooze_until: row.get(9)?,
        source: row.get(10)?,
        context_json: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

// ── Attention Items (unified inbound communication tracking) ────────

/// A communication item needing the user's attention.
///
/// Any channel (email, WhatsApp, SMS, Telegram, missed calls, etc.)
/// writes items here. The open loops monitor scans for unhandled items
/// and promotes them to life threads when they've been waiting too long.
#[derive(Debug, Clone)]
pub struct AttentionItem {
    pub id: i64,
    /// Channel identifier: "email", "whatsapp", "telegram", "call", "text", etc.
    pub channel: String,
    /// Channel-specific ID (email message_id, WhatsApp message ID, etc.).
    pub external_id: String,
    pub sender: String,
    pub sender_name: String,
    pub subject: String,
    pub preview: String,
    pub received_ts: f64,
    /// "low", "normal", "high", "urgent"
    pub importance: String,
    /// Whether this item expects a user reply.
    pub needs_reply: bool,
    /// Whether the user has replied.
    pub replied: bool,
    /// Whether the monitor has processed this into a life thread.
    pub handled: bool,
    pub context_json: String,
    pub created_at: f64,
    pub updated_at: f64,
}

/// Upsert an attention item (idempotent per channel + external_id).
pub fn upsert_attention_item(
    conn: &Connection,
    channel: &str,
    external_id: &str,
    sender: &str,
    sender_name: &str,
    subject: &str,
    preview: &str,
    received_ts: f64,
    importance: &str,
    needs_reply: bool,
    context_json: &str,
) -> i64 {
    let now = now_ts();
    conn.execute(
        "INSERT INTO attention_items
         (channel, external_id, sender, sender_name, subject, preview,
          received_ts, importance, needs_reply, replied, handled,
          context_json, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,0,0,?10,?11,?12)
         ON CONFLICT(channel, external_id) DO UPDATE SET
           sender_name = excluded.sender_name,
           subject = excluded.subject,
           importance = excluded.importance,
           context_json = excluded.context_json,
           updated_at = excluded.updated_at",
        params![
            channel, external_id, sender, sender_name, subject, preview,
            received_ts, importance, needs_reply as i32,
            context_json, now, now,
        ],
    )
    .expect("failed to upsert attention item");
    conn.last_insert_rowid()
}

/// Mark an attention item as replied.
pub fn mark_attention_replied(conn: &Connection, channel: &str, external_id: &str) {
    let now = now_ts();
    let _ = conn.execute(
        "UPDATE attention_items SET replied = 1, updated_at = ?1
         WHERE channel = ?2 AND external_id = ?3",
        params![now, channel, external_id],
    );
}

/// Mark an attention item as handled (promoted to life thread).
pub fn mark_attention_handled(conn: &Connection, id: i64) {
    let now = now_ts();
    let _ = conn.execute(
        "UPDATE attention_items SET handled = 1, updated_at = ?1 WHERE id = ?2",
        params![now, id],
    );
}

/// Query unhandled attention items that need reply and are older than `min_age_secs`.
pub fn query_pending_attention(
    conn: &Connection,
    min_age_secs: f64,
    limit: usize,
) -> Vec<AttentionItem> {
    let cutoff = now_ts() - min_age_secs;
    let max_lookback = now_ts() - 14.0 * 86400.0; // don't go back more than 14 days
    conn.prepare(
        "SELECT id, channel, external_id, sender, sender_name, subject, preview,
                received_ts, importance, needs_reply, replied, handled,
                context_json, created_at, updated_at
         FROM attention_items
         WHERE needs_reply = 1 AND replied = 0 AND handled = 0
           AND received_ts < ?1 AND received_ts > ?2
         ORDER BY
           CASE importance
             WHEN 'urgent' THEN 0
             WHEN 'high' THEN 1
             WHEN 'normal' THEN 2
             ELSE 3
           END,
           received_ts ASC
         LIMIT ?3",
    )
    .and_then(|mut stmt| {
        stmt.query_map(params![cutoff, max_lookback, limit as i64], |row| {
            Ok(AttentionItem {
                id: row.get(0)?,
                channel: row.get(1)?,
                external_id: row.get(2)?,
                sender: row.get(3)?,
                sender_name: row.get(4)?,
                subject: row.get(5)?,
                preview: row.get(6)?,
                received_ts: row.get(7)?,
                importance: row.get(8)?,
                needs_reply: row.get::<_, i32>(9)? != 0,
                replied: row.get::<_, i32>(10)? != 0,
                handled: row.get::<_, i32>(11)? != 0,
                context_json: row.get(12)?,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
    })
    .unwrap_or_default()
}

/// Count pending attention items per channel.
pub fn attention_summary(conn: &Connection) -> Vec<(String, i64)> {
    conn.prepare(
        "SELECT channel, COUNT(*) FROM attention_items
         WHERE needs_reply = 1 AND replied = 0 AND handled = 0
         GROUP BY channel ORDER BY COUNT(*) DESC",
    )
    .and_then(|mut stmt| {
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
    })
    .unwrap_or_default()
}

// ── WorldModel (persistence + queries) ──────────────────────────────

/// Manages world model persistence and queries.
pub struct WorldModel;

impl WorldModel {
    /// Create all world model tables.
    pub fn ensure_tables(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS wm_commitments (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                promisor          TEXT NOT NULL,
                promisee          TEXT NOT NULL,
                action            TEXT NOT NULL,
                deadline          REAL NOT NULL DEFAULT 0,
                status            TEXT NOT NULL DEFAULT 'pending',
                confidence        REAL NOT NULL DEFAULT 0.5,
                source_type       TEXT NOT NULL DEFAULT 'conversation',
                source_data       TEXT NOT NULL DEFAULT '{}',
                evidence_text     TEXT NOT NULL DEFAULT '',
                related_entities  TEXT NOT NULL DEFAULT '[]',
                completion_evidence TEXT,
                created_at        REAL NOT NULL,
                updated_at        REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_wm_commit_status ON wm_commitments(status);
            CREATE INDEX IF NOT EXISTS idx_wm_commit_deadline ON wm_commitments(deadline);
            CREATE INDEX IF NOT EXISTS idx_wm_commit_promisor ON wm_commitments(promisor);

            CREATE TABLE IF NOT EXISTS wm_preferences (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                domain            TEXT NOT NULL,
                key               TEXT NOT NULL,
                value             TEXT NOT NULL,
                strength          TEXT NOT NULL DEFAULT 'weak',
                scope             TEXT NOT NULL DEFAULT 'global',
                observation_count INTEGER NOT NULL DEFAULT 1,
                last_observed_at  REAL NOT NULL,
                created_at        REAL NOT NULL,
                contradictions    TEXT NOT NULL DEFAULT '[]',
                active            INTEGER NOT NULL DEFAULT 1
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_wm_pref_unique ON wm_preferences(domain, key, scope);
            CREATE INDEX IF NOT EXISTS idx_wm_pref_domain ON wm_preferences(domain);

            CREATE TABLE IF NOT EXISTS wm_routines (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                description       TEXT NOT NULL,
                trigger_data      TEXT NOT NULL DEFAULT '{}',
                action_sequence   TEXT NOT NULL DEFAULT '[]',
                observation_count INTEGER NOT NULL DEFAULT 1,
                confidence        REAL NOT NULL DEFAULT 0.3,
                avg_duration_min  REAL NOT NULL DEFAULT 0.0,
                created_at        REAL NOT NULL,
                last_observed_at  REAL NOT NULL,
                active            INTEGER NOT NULL DEFAULT 1
            );
            CREATE INDEX IF NOT EXISTS idx_wm_routine_active ON wm_routines(active);

            CREATE TABLE IF NOT EXISTS life_threads (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                thread_type     TEXT NOT NULL,
                entity_id       TEXT NOT NULL,
                label           TEXT NOT NULL,
                deadline_ts     REAL DEFAULT 0,
                status          TEXT NOT NULL DEFAULT 'open',
                importance      REAL DEFAULT 0.5,
                days_open       INTEGER DEFAULT 0,
                last_action_ts  REAL DEFAULT 0,
                snooze_until    REAL DEFAULT 0,
                source          TEXT DEFAULT '',
                context_json    TEXT DEFAULT '{}',
                created_at      REAL NOT NULL,
                updated_at      REAL NOT NULL,
                UNIQUE(thread_type, entity_id)
            );
            CREATE INDEX IF NOT EXISTS idx_life_threads_status ON life_threads(status);
            CREATE INDEX IF NOT EXISTS idx_life_threads_type ON life_threads(thread_type, status);

            CREATE TABLE IF NOT EXISTS attention_items (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                channel         TEXT NOT NULL,
                external_id     TEXT NOT NULL,
                sender          TEXT NOT NULL DEFAULT '',
                sender_name     TEXT NOT NULL DEFAULT '',
                subject         TEXT NOT NULL DEFAULT '',
                preview         TEXT NOT NULL DEFAULT '',
                received_ts     REAL NOT NULL,
                importance      TEXT NOT NULL DEFAULT 'normal',
                needs_reply     INTEGER NOT NULL DEFAULT 1,
                replied         INTEGER NOT NULL DEFAULT 0,
                handled         INTEGER NOT NULL DEFAULT 0,
                context_json    TEXT NOT NULL DEFAULT '{}',
                created_at      REAL NOT NULL,
                updated_at      REAL NOT NULL,
                UNIQUE(channel, external_id)
            );
            CREATE INDEX IF NOT EXISTS idx_attention_channel ON attention_items(channel, handled);
            CREATE INDEX IF NOT EXISTS idx_attention_needs_reply ON attention_items(needs_reply, replied, received_ts);",
        )
        .expect("failed to create world model tables");
    }

    // ── Commitment CRUD ──

    /// Insert a new commitment, returning its ID.
    pub fn insert_commitment(conn: &Connection, c: &Commitment) -> i64 {
        let source_data = serde_json::to_string(&c.source).unwrap_or_default();
        let related = serde_json::to_string(&c.related_entities).unwrap_or_default();
        conn.execute(
            "INSERT INTO wm_commitments
             (promisor, promisee, action, deadline, status, confidence,
              source_type, source_data, evidence_text, related_entities,
              completion_evidence, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                c.promisor,
                c.promisee,
                c.action,
                c.deadline,
                c.status.as_str(),
                c.confidence,
                c.source.type_tag(),
                source_data,
                c.evidence_text,
                related,
                c.completion_evidence,
                c.created_at,
                c.updated_at,
            ],
        )
        .expect("failed to insert commitment");
        conn.last_insert_rowid()
    }

    /// Update commitment status.
    pub fn update_commitment_status(
        conn: &Connection,
        id: i64,
        status: CommitmentStatus,
        evidence: Option<&str>,
    ) {
        let now = now_ts();
        let _ = conn.execute(
            "UPDATE wm_commitments SET status = ?1, updated_at = ?2, completion_evidence = ?3
             WHERE id = ?4",
            params![status.as_str(), now, evidence, id],
        );
    }

    /// Get all active (non-terminal) commitments.
    pub fn active_commitments(conn: &Connection) -> Vec<Commitment> {
        Self::query_commitments(
            conn,
            "WHERE status NOT IN ('completed', 'cancelled', 'superseded')
             ORDER BY deadline ASC, created_at ASC",
        )
    }

    /// Get commitments approaching deadline (within `hours` hours from now).
    pub fn approaching_deadlines(conn: &Connection, hours: f64) -> Vec<Commitment> {
        let now = now_ts();
        let cutoff = now + hours * 3600.0;
        Self::query_commitments_params(
            conn,
            "WHERE status IN ('pending', 'in_progress')
             AND deadline > 0 AND deadline <= ?1 AND deadline > ?2
             ORDER BY deadline ASC",
            params![cutoff, now],
        )
    }

    /// Get overdue commitments.
    pub fn overdue_commitments(conn: &Connection) -> Vec<Commitment> {
        Self::query_commitments(
            conn,
            "WHERE status = 'overdue' ORDER BY deadline ASC",
        )
    }

    /// Get commitments by promisor (what someone owes).
    pub fn commitments_by(conn: &Connection, promisor: &str) -> Vec<Commitment> {
        Self::query_commitments_params(
            conn,
            "WHERE promisor = ?1 AND status NOT IN ('completed', 'cancelled', 'superseded')
             ORDER BY deadline ASC",
            params![promisor],
        )
    }

    /// Get commitments to promisee (what's owed to someone).
    pub fn commitments_to(conn: &Connection, promisee: &str) -> Vec<Commitment> {
        Self::query_commitments_params(
            conn,
            "WHERE promisee = ?1 AND status NOT IN ('completed', 'cancelled', 'superseded')
             ORDER BY deadline ASC",
            params![promisee],
        )
    }

    /// Check for overdue commitments and auto-transition them.
    pub fn check_overdue(conn: &Connection) -> Vec<Commitment> {
        let now = now_ts();
        let _ = conn.execute(
            "UPDATE wm_commitments SET status = 'overdue', updated_at = ?1
             WHERE status IN ('pending', 'in_progress')
             AND deadline > 0 AND deadline < ?2",
            params![now, now],
        );
        Self::overdue_commitments(conn)
    }

    // ── Preference CRUD ──

    /// Upsert a preference — if same domain+key+scope exists, update it.
    pub fn upsert_preference(conn: &Connection, p: &Preference) {
        let scope_str = p.scope.as_str();
        let contradictions = serde_json::to_string(&p.contradictions).unwrap_or_default();

        // Check for existing preference with same domain+key+scope
        let existing: Option<(i64, u32, String)> = conn
            .query_row(
                "SELECT id, observation_count, value FROM wm_preferences
                 WHERE domain = ?1 AND key = ?2 AND scope = ?3",
                params![p.domain, p.key, scope_str],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();

        if let Some((id, count, old_value)) = existing {
            if old_value == p.value {
                // Same value — reinforce
                let new_strength = if count + 1 >= 5 {
                    "strong"
                } else if count + 1 >= 2 {
                    "moderate"
                } else {
                    p.strength.as_str()
                };
                let _ = conn.execute(
                    "UPDATE wm_preferences
                     SET observation_count = ?1, strength = ?2, last_observed_at = ?3, active = 1
                     WHERE id = ?4",
                    params![count + 1, new_strength, p.last_observed_at, id],
                );
            } else {
                // Different value — record contradiction and replace
                let mut contras = p.contradictions.clone();
                contras.push(format!("was '{}' (observed {}x)", old_value, count));
                let contras_json = serde_json::to_string(&contras).unwrap_or_default();
                let _ = conn.execute(
                    "UPDATE wm_preferences
                     SET value = ?1, strength = ?2, observation_count = 1,
                         last_observed_at = ?3, contradictions = ?4, active = 1
                     WHERE id = ?5",
                    params![p.value, p.strength.as_str(), p.last_observed_at, contras_json, id],
                );
            }
        } else {
            // New preference
            let _ = conn.execute(
                "INSERT INTO wm_preferences
                 (domain, key, value, strength, scope, observation_count,
                  last_observed_at, created_at, contradictions, active)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,1)",
                params![
                    p.domain,
                    p.key,
                    p.value,
                    p.strength.as_str(),
                    scope_str,
                    p.observation_count,
                    p.last_observed_at,
                    p.created_at,
                    contradictions,
                ],
            );
        }
    }

    /// Get all active preferences, optionally filtered by domain.
    pub fn preferences(conn: &Connection, domain: Option<&str>) -> Vec<Preference> {
        let sql = if let Some(d) = domain {
            format!(
                "SELECT id, domain, key, value, strength, scope, observation_count,
                        last_observed_at, created_at, contradictions, active
                 FROM wm_preferences WHERE active = 1 AND domain = '{}'
                 ORDER BY observation_count DESC",
                d.replace('\'', "''")
            )
        } else {
            "SELECT id, domain, key, value, strength, scope, observation_count,
                    last_observed_at, created_at, contradictions, active
             FROM wm_preferences WHERE active = 1
             ORDER BY domain, observation_count DESC"
                .to_string()
        };

        conn.prepare(&sql)
            .and_then(|mut stmt| {
                stmt.query_map([], |row| {
                    let contras_json: String = row.get(9)?;
                    let contradictions: Vec<String> =
                        serde_json::from_str(&contras_json).unwrap_or_default();
                    Ok(Preference {
                        id: row.get(0)?,
                        domain: row.get(1)?,
                        key: row.get(2)?,
                        value: row.get(3)?,
                        strength: PreferenceStrength::from_str(
                            &row.get::<_, String>(4).unwrap_or_default(),
                        ),
                        scope: PreferenceScope::from_str(
                            &row.get::<_, String>(5).unwrap_or_default(),
                        ),
                        observation_count: row.get::<_, u32>(6)?,
                        last_observed_at: row.get(7)?,
                        created_at: row.get(8)?,
                        contradictions,
                        active: row.get::<_, i32>(10)? != 0,
                    })
                })
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default()
    }

    /// Get a specific preference by domain+key.
    pub fn get_preference(conn: &Connection, domain: &str, key: &str) -> Option<Preference> {
        Self::preferences(conn, Some(domain))
            .into_iter()
            .find(|p| p.key == key)
    }

    // ── Routine CRUD ──

    /// Insert a new routine.
    pub fn insert_routine(conn: &Connection, r: &Routine) -> i64 {
        let trigger = r.trigger.to_json();
        let actions = serde_json::to_string(&r.action_sequence).unwrap_or_default();
        conn.execute(
            "INSERT INTO wm_routines
             (description, trigger_data, action_sequence, observation_count,
              confidence, avg_duration_min, created_at, last_observed_at, active)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,1)",
            params![
                r.description,
                trigger,
                actions,
                r.observation_count,
                r.confidence,
                r.avg_duration_min,
                r.created_at,
                r.last_observed_at,
            ],
        )
        .expect("failed to insert routine");
        conn.last_insert_rowid()
    }

    /// Reinforce an observed routine (bump count + confidence).
    pub fn reinforce_routine(conn: &Connection, id: i64, duration_min: f64) {
        let now = now_ts();
        let _ = conn.execute(
            "UPDATE wm_routines
             SET observation_count = observation_count + 1,
                 confidence = MIN(0.95, confidence + 0.05),
                 avg_duration_min = (avg_duration_min * observation_count + ?1) / (observation_count + 1),
                 last_observed_at = ?2,
                 active = 1
             WHERE id = ?3",
            params![duration_min, now, id],
        );
    }

    /// Get active routines sorted by confidence.
    pub fn active_routines(conn: &Connection) -> Vec<Routine> {
        conn.prepare(
            "SELECT id, description, trigger_data, action_sequence, observation_count,
                    confidence, avg_duration_min, created_at, last_observed_at, active
             FROM wm_routines WHERE active = 1
             ORDER BY confidence DESC",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                let trigger_json: String = row.get(2)?;
                let actions_json: String = row.get(3)?;
                Ok(Routine {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    trigger: RoutineTrigger::from_json(&trigger_json),
                    action_sequence: serde_json::from_str(&actions_json).unwrap_or_default(),
                    observation_count: row.get::<_, u32>(4)?,
                    confidence: row.get(5)?,
                    avg_duration_min: row.get(6)?,
                    created_at: row.get(7)?,
                    last_observed_at: row.get(8)?,
                    active: row.get::<_, i32>(9)? != 0,
                })
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
    }

    // ── Summary / Dashboard ──

    /// Generate a world model summary for system prompt injection.
    pub fn summary(conn: &Connection, user_name: &str) -> String {
        let commitments = Self::active_commitments(conn);
        let overdue = Self::overdue_commitments(conn);
        let prefs = Self::preferences(conn, None);
        let routines = Self::active_routines(conn);

        if commitments.is_empty() && prefs.is_empty() && routines.is_empty() {
            return String::new();
        }

        let mut out = String::from("## World Model\n");

        // Overdue commitments (urgent)
        if !overdue.is_empty() {
            out.push_str("\n### OVERDUE Commitments\n");
            for c in &overdue {
                out.push_str(&format!(
                    "- {} → {}: \"{}\" (was due {})\n",
                    c.promisor, c.promisee, c.action,
                    format_relative_time(c.deadline),
                ));
            }
        }

        // Active commitments
        let pending: Vec<&Commitment> = commitments
            .iter()
            .filter(|c| !c.status.is_terminal() && c.status != CommitmentStatus::Overdue)
            .collect();
        if !pending.is_empty() {
            out.push_str(&format!("\n### Active Commitments ({})\n", pending.len()));
            for c in pending.iter().take(5) {
                let deadline_str = if c.deadline > 0.0 {
                    format!(" (due {})", format_relative_time(c.deadline))
                } else {
                    String::new()
                };
                out.push_str(&format!(
                    "- {} → {}: \"{}\"{}\n",
                    c.promisor, c.promisee, c.action, deadline_str,
                ));
            }
            if pending.len() > 5 {
                out.push_str(&format!("  ...and {} more\n", pending.len() - 5));
            }
        }

        // Key preferences
        let strong_prefs: Vec<&Preference> = prefs
            .iter()
            .filter(|p| matches!(p.strength, PreferenceStrength::Strong | PreferenceStrength::Moderate))
            .collect();
        if !strong_prefs.is_empty() {
            out.push_str(&format!(
                "\n### Known Preferences ({} strong/moderate)\n",
                strong_prefs.len()
            ));
            for p in strong_prefs.iter().take(8) {
                out.push_str(&format!(
                    "- {}/{}: {} [{}]\n",
                    p.domain, p.key, p.value,
                    p.strength.as_str(),
                ));
            }
        }

        // Active routines
        let confident_routines: Vec<&Routine> = routines
            .iter()
            .filter(|r| r.confidence >= 0.5)
            .collect();
        if !confident_routines.is_empty() {
            out.push_str(&format!(
                "\n### Detected Routines ({} confident)\n",
                confident_routines.len()
            ));
            for r in confident_routines.iter().take(5) {
                out.push_str(&format!(
                    "- {} (observed {}x, ~{:.0}min)\n",
                    r.description, r.observation_count, r.avg_duration_min,
                ));
            }
        }

        out
    }

    // ── Internal helpers ──

    fn query_commitments(conn: &Connection, where_clause: &str) -> Vec<Commitment> {
        let sql = format!(
            "SELECT id, promisor, promisee, action, deadline, status, confidence,
                    source_type, source_data, evidence_text, related_entities,
                    completion_evidence, created_at, updated_at
             FROM wm_commitments {}",
            where_clause
        );
        conn.prepare(&sql)
            .and_then(|mut stmt| {
                stmt.query_map([], |row| Self::row_to_commitment(row))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default()
    }

    fn query_commitments_params(
        conn: &Connection,
        where_clause: &str,
        params: &[&dyn rusqlite::types::ToSql],
    ) -> Vec<Commitment> {
        let sql = format!(
            "SELECT id, promisor, promisee, action, deadline, status, confidence,
                    source_type, source_data, evidence_text, related_entities,
                    completion_evidence, created_at, updated_at
             FROM wm_commitments {}",
            where_clause
        );
        conn.prepare(&sql)
            .and_then(|mut stmt| {
                stmt.query_map(params, |row| Self::row_to_commitment(row))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default()
    }

    fn row_to_commitment(row: &rusqlite::Row) -> rusqlite::Result<Commitment> {
        let source_type: String = row.get(7)?;
        let source_data: String = row.get(8)?;
        let source = match source_type.as_str() {
            "email" => serde_json::from_str(&source_data).unwrap_or(CommitmentSource::Manual),
            "calendar" => serde_json::from_str(&source_data).unwrap_or(CommitmentSource::Manual),
            "conversation" => {
                serde_json::from_str(&source_data).unwrap_or(CommitmentSource::Conversation {
                    turn_id: None,
                })
            }
            _ => CommitmentSource::Manual,
        };
        let related_json: String = row.get(10)?;
        let related: Vec<String> = serde_json::from_str(&related_json).unwrap_or_default();

        Ok(Commitment {
            id: row.get(0)?,
            promisor: row.get(1)?,
            promisee: row.get(2)?,
            action: row.get(3)?,
            deadline: row.get(4)?,
            status: CommitmentStatus::from_str(&row.get::<_, String>(5).unwrap_or_default()),
            confidence: row.get(6)?,
            source,
            evidence_text: row.get(9)?,
            related_entities: related,
            completion_evidence: row.get(11)?,
            created_at: row.get(12)?,
            updated_at: row.get(13)?,
        })
    }
}

/// Format a Unix timestamp as relative time (e.g., "2 hours ago", "in 3 days").
fn format_relative_time(ts: f64) -> String {
    let now = now_ts();
    let diff = ts - now;
    let abs_diff = diff.abs();

    let (value, unit) = if abs_diff < 3600.0 {
        ((abs_diff / 60.0).round() as i64, "min")
    } else if abs_diff < 86400.0 {
        ((abs_diff / 3600.0).round() as i64, "hour")
    } else {
        ((abs_diff / 86400.0).round() as i64, "day")
    };

    let plural = if value != 1 { "s" } else { "" };
    if diff < 0.0 {
        format!("{value} {unit}{plural} ago")
    } else {
        format!("in {value} {unit}{plural}")
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        WorldModel::ensure_tables(&conn);
        conn
    }

    #[test]
    fn commitment_lifecycle() {
        let conn = setup();
        let now = now_ts();

        let c = Commitment {
            id: 0,
            promisor: "user".into(),
            promisee: "Sarah".into(),
            action: "Send the quarterly report".into(),
            deadline: now + 86400.0, // 1 day from now
            status: CommitmentStatus::Pending,
            confidence: 0.9,
            source: CommitmentSource::Email {
                message_id: "msg-123".into(),
                subject: "Quarterly Report".into(),
            },
            evidence_text: "I'll send the report by tomorrow".into(),
            related_entities: vec!["person:sarah".into()],
            created_at: now,
            updated_at: now,
            completion_evidence: None,
        };

        let id = WorldModel::insert_commitment(&conn, &c);
        assert!(id > 0);

        // Should appear in active commitments
        let active = WorldModel::active_commitments(&conn);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].action, "Send the quarterly report");

        // Mark completed
        WorldModel::update_commitment_status(
            &conn,
            id,
            CommitmentStatus::Completed,
            Some("Sent email with attachment"),
        );

        // Should no longer appear in active
        let active2 = WorldModel::active_commitments(&conn);
        assert!(active2.is_empty());
    }

    #[test]
    fn overdue_detection() {
        let conn = setup();
        let now = now_ts();

        // Commitment that's already past deadline
        let c = Commitment {
            id: 0,
            promisor: "user".into(),
            promisee: "Boss".into(),
            action: "Submit timesheet".into(),
            deadline: now - 3600.0, // 1 hour ago
            status: CommitmentStatus::Pending,
            confidence: 0.8,
            source: CommitmentSource::Conversation { turn_id: None },
            evidence_text: "I need to submit my timesheet".into(),
            related_entities: vec![],
            created_at: now - 7200.0,
            updated_at: now - 7200.0,
            completion_evidence: None,
        };

        WorldModel::insert_commitment(&conn, &c);

        let overdue = WorldModel::check_overdue(&conn);
        assert_eq!(overdue.len(), 1);
        assert_eq!(overdue[0].status, CommitmentStatus::Overdue);
    }

    #[test]
    fn preference_upsert_and_reinforce() {
        let conn = setup();
        let now = now_ts();

        let p = Preference {
            id: 0,
            domain: "food".into(),
            key: "coffee".into(),
            value: "black, no sugar".into(),
            strength: PreferenceStrength::Weak,
            scope: PreferenceScope::Global,
            observation_count: 1,
            last_observed_at: now,
            created_at: now,
            contradictions: vec![],
            active: true,
        };

        // First insert
        WorldModel::upsert_preference(&conn, &p);
        let prefs = WorldModel::preferences(&conn, Some("food"));
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].observation_count, 1);

        // Reinforce same value
        WorldModel::upsert_preference(&conn, &p);
        let prefs2 = WorldModel::preferences(&conn, Some("food"));
        assert_eq!(prefs2.len(), 1);
        assert_eq!(prefs2[0].observation_count, 2);
        assert_eq!(prefs2[0].strength, PreferenceStrength::Moderate);
    }

    #[test]
    fn preference_contradiction() {
        let conn = setup();
        let now = now_ts();

        let p1 = Preference {
            id: 0,
            domain: "food".into(),
            key: "coffee".into(),
            value: "black, no sugar".into(),
            strength: PreferenceStrength::Moderate,
            scope: PreferenceScope::Global,
            observation_count: 3,
            last_observed_at: now,
            created_at: now,
            contradictions: vec![],
            active: true,
        };
        WorldModel::upsert_preference(&conn, &p1);
        // Reinforce twice more to make it 3 observations
        WorldModel::upsert_preference(&conn, &p1);
        WorldModel::upsert_preference(&conn, &p1);

        // Contradicting preference
        let p2 = Preference {
            id: 0,
            domain: "food".into(),
            key: "coffee".into(),
            value: "latte with oat milk".into(),
            strength: PreferenceStrength::Weak,
            scope: PreferenceScope::Global,
            observation_count: 1,
            last_observed_at: now + 1.0,
            created_at: now + 1.0,
            contradictions: vec![],
            active: true,
        };
        WorldModel::upsert_preference(&conn, &p2);

        let prefs = WorldModel::preferences(&conn, Some("food"));
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].value, "latte with oat milk");
        assert!(!prefs[0].contradictions.is_empty());
        assert!(prefs[0].contradictions[0].contains("black, no sugar"));
    }

    #[test]
    fn routine_tracking() {
        let conn = setup();
        let now = now_ts();

        let r = Routine {
            id: 0,
            description: "Morning email check".into(),
            trigger: RoutineTrigger::TimeOfDay {
                hour: 9,
                days: vec!["weekday".into()],
            },
            action_sequence: vec!["email_check".into(), "email_list".into()],
            observation_count: 3,
            confidence: 0.4,
            avg_duration_min: 15.0,
            created_at: now,
            last_observed_at: now,
            active: true,
        };

        let id = WorldModel::insert_routine(&conn, &r);
        WorldModel::reinforce_routine(&conn, id, 12.0);

        let routines = WorldModel::active_routines(&conn);
        assert_eq!(routines.len(), 1);
        assert_eq!(routines[0].observation_count, 4);
        assert!(routines[0].confidence > 0.4);
    }

    #[test]
    fn summary_generation() {
        let conn = setup();
        let now = now_ts();

        // Add a commitment
        let c = Commitment {
            id: 0,
            promisor: "user".into(),
            promisee: "Sarah".into(),
            action: "Review PR #42".into(),
            deadline: now + 3600.0,
            status: CommitmentStatus::Pending,
            confidence: 0.9,
            source: CommitmentSource::Conversation { turn_id: None },
            evidence_text: "I'll review your PR".into(),
            related_entities: vec![],
            created_at: now,
            updated_at: now,
            completion_evidence: None,
        };
        WorldModel::insert_commitment(&conn, &c);

        // Add a preference
        let p = Preference {
            id: 0,
            domain: "communication".into(),
            key: "style".into(),
            value: "async-first".into(),
            strength: PreferenceStrength::Strong,
            scope: PreferenceScope::Global,
            observation_count: 5,
            last_observed_at: now,
            created_at: now,
            contradictions: vec![],
            active: true,
        };
        WorldModel::upsert_preference(&conn, &p);

        let summary = WorldModel::summary(&conn, "Pranab");
        assert!(summary.contains("World Model"));
        assert!(summary.contains("Review PR #42"));
        assert!(summary.contains("async-first"));
    }

    // ── Life Thread tests ──

    #[test]
    fn life_thread_upsert_creates_new() {
        let conn = setup();
        let id = upsert_life_thread(
            &conn,
            &ThreadType::Email,
            "msg-abc",
            "Re: Quarterly report",
            0.0,
            0.7,
            "email_scan",
            "{}",
        );
        assert!(id > 0);

        let threads = query_open_threads(&conn, 10);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].label, "Re: Quarterly report");
        assert_eq!(threads[0].thread_type, ThreadType::Email);
        assert_eq!(threads[0].entity_id, "msg-abc");
        assert_eq!(threads[0].importance, 0.7);
    }

    #[test]
    fn life_thread_upsert_updates_existing() {
        let conn = setup();

        // First insert
        upsert_life_thread(
            &conn,
            &ThreadType::Email,
            "msg-abc",
            "Old label",
            0.0,
            0.5,
            "email_scan",
            "{}",
        );

        // Upsert same (type, entity_id) with new data
        upsert_life_thread(
            &conn,
            &ThreadType::Email,
            "msg-abc",
            "Updated label",
            0.0,
            0.9,
            "email_scan",
            "{\"sender\":\"boss@co.com\"}",
        );

        let threads = query_open_threads(&conn, 10);
        assert_eq!(threads.len(), 1, "should still be one thread (dedup)");
        assert_eq!(threads[0].label, "Updated label");
        assert_eq!(threads[0].importance, 0.9);
    }

    #[test]
    fn life_thread_status_transitions() {
        let conn = setup();
        upsert_life_thread(
            &conn,
            &ThreadType::Task,
            "task-1",
            "Write docs",
            0.0,
            0.5,
            "cortex",
            "{}",
        );

        // Transition to stalled
        update_thread_status(&conn, &ThreadType::Task, "task-1", &ThreadStatus::Stalled);
        let threads = query_open_threads(&conn, 10);
        assert_eq!(threads[0].status, ThreadStatus::Stalled);

        // Transition to archived (terminal)
        archive_thread(&conn, &ThreadType::Task, "task-1");
        let threads2 = query_open_threads(&conn, 10);
        assert!(threads2.is_empty(), "archived thread should not appear in open query");
    }

    #[test]
    fn life_thread_resolve_stores_evidence() {
        let conn = setup();
        upsert_life_thread(
            &conn,
            &ThreadType::Commitment,
            "commit-7",
            "Send invoice",
            0.0,
            0.8,
            "commitment_extract",
            "{\"to\":\"client\"}",
        );

        resolve_thread(
            &conn,
            &ThreadType::Commitment,
            "commit-7",
            "Invoice #42 sent via email",
        );

        // Should not appear in open threads
        let open = query_open_threads(&conn, 10);
        assert!(open.is_empty());

        // Verify evidence stored in context_json
        let ctx: String = conn
            .query_row(
                "SELECT context_json FROM life_threads WHERE entity_id = 'commit-7'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(ctx.contains("Invoice #42 sent via email"));
        assert!(ctx.contains("resolution_evidence"));
        // Original context should be preserved
        assert!(ctx.contains("client"));
    }

    #[test]
    fn life_thread_snooze_hides_until_time() {
        let conn = setup();
        let now = now_ts();

        upsert_life_thread(
            &conn,
            &ThreadType::Email,
            "msg-snooze",
            "Non-urgent email",
            0.0,
            0.3,
            "email_scan",
            "{}",
        );

        // Snooze into the far future
        snooze_thread(&conn, &ThreadType::Email, "msg-snooze", now + 999999.0);

        let open = query_open_threads(&conn, 10);
        assert!(open.is_empty(), "snoozed thread should be hidden");

        // Snooze into the past (simulates time passing)
        snooze_thread(&conn, &ThreadType::Email, "msg-snooze", now - 1.0);
        // Need to set status back to open for it to reappear
        update_thread_status(&conn, &ThreadType::Email, "msg-snooze", &ThreadStatus::Open);
        // Also reset snooze_until for the query
        let _ = conn.execute(
            "UPDATE life_threads SET snooze_until = ?1 WHERE entity_id = 'msg-snooze'",
            params![now - 1.0],
        );

        let open2 = query_open_threads(&conn, 10);
        assert_eq!(open2.len(), 1, "un-snoozed thread should reappear");
    }

    #[test]
    fn life_thread_query_open_respects_limit_and_order() {
        let conn = setup();

        // Insert threads with varying importance
        for (i, imp) in [(1, 0.3), (2, 0.9), (3, 0.6)] {
            upsert_life_thread(
                &conn,
                &ThreadType::Task,
                &format!("task-{i}"),
                &format!("Task {i}"),
                0.0,
                imp,
                "cortex",
                "{}",
            );
        }

        // Limit 2
        let threads = query_open_threads(&conn, 2);
        assert_eq!(threads.len(), 2);
        // Should be ordered by importance DESC
        assert_eq!(threads[0].importance, 0.9);
        assert_eq!(threads[1].importance, 0.6);

        // Limit 100 gets all 3
        let all = query_open_threads(&conn, 100);
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn life_thread_transition_overdue() {
        let conn = setup();
        let now = now_ts();

        // Thread with expired deadline
        upsert_life_thread(
            &conn,
            &ThreadType::Task,
            "task-overdue",
            "Past-due task",
            now - 3600.0, // 1 hour ago
            0.8,
            "cortex",
            "{}",
        );

        // Thread with future deadline
        upsert_life_thread(
            &conn,
            &ThreadType::Task,
            "task-future",
            "Future task",
            now + 86400.0, // 1 day from now
            0.5,
            "cortex",
            "{}",
        );

        // Thread with no deadline
        upsert_life_thread(
            &conn,
            &ThreadType::Email,
            "msg-nodeadline",
            "No deadline email",
            0.0,
            0.4,
            "email_scan",
            "{}",
        );

        let count = transition_overdue(&conn);
        assert_eq!(count, 1, "only the expired-deadline thread should transition");

        let open = query_open_threads(&conn, 10);
        // The overdue thread should still show (status=overdue is in the query)
        let overdue_thread = open.iter().find(|t| t.entity_id == "task-overdue");
        assert!(overdue_thread.is_some());
        assert_eq!(overdue_thread.unwrap().status, ThreadStatus::Overdue);

        // Future task should still be open
        let future = open.iter().find(|t| t.entity_id == "task-future");
        assert_eq!(future.unwrap().status, ThreadStatus::Open);
    }

    #[test]
    fn life_thread_count_open_matches() {
        let conn = setup();

        // 3 open threads
        for i in 0..3 {
            upsert_life_thread(
                &conn,
                &ThreadType::Task,
                &format!("t-{i}"),
                &format!("Task {i}"),
                0.0,
                0.5,
                "test",
                "{}",
            );
        }
        assert_eq!(count_open_threads(&conn), 3);

        // Resolve one
        resolve_thread(&conn, &ThreadType::Task, "t-0", "done");
        assert_eq!(count_open_threads(&conn), 2);

        // Archive one
        archive_thread(&conn, &ThreadType::Task, "t-1");
        assert_eq!(count_open_threads(&conn), 1);

        // Mark one overdue (still counts as open)
        update_thread_status(&conn, &ThreadType::Task, "t-2", &ThreadStatus::Overdue);
        assert_eq!(count_open_threads(&conn), 1);
    }
}
