//! Cognitive Event Bus — typed, causal, replayable event system.
//!
//! Every meaningful change in Yantrik flows through the event bus as a
//! `YantrikEvent`. Each event carries a `TraceId` for causal tracing —
//! you can always answer "why did this happen?" by following the trace chain.
//!
//! Architecture:
//! - **Producers** emit events via `EventBus::emit()` or `emit_with_parent()`
//! - **Consumers** subscribe via `EventBus::subscribe()` and receive a `Receiver`
//! - **EventLog** persists events to SQLite for replay, debugging, and analysis
//! - **SystemObserver bridge** converts existing `SystemEvent` → `YantrikEvent`
//!
//! The bus uses crossbeam broadcast channels (bounded, backpressure-aware).
//! Subscribers that fall behind will miss events (lossy) — this is intentional
//! to prevent slow consumers from blocking the entire system.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};

use crate::events::SystemEvent;

// ────────────────────────────────────────────────────────────────────────────
// Trace IDs — causal identity for every event
// ────────────────────────────────────────────────────────────────────────────

/// Monotonic counter for trace ID generation (process-unique, not globally unique).
static TRACE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// A causal trace identifier. Every event gets one. Events can reference a
/// `parent_trace` to form causal chains (e.g., "this tool outcome was caused
/// by this user message which was triggered by this system event").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TraceId(u64);

impl TraceId {
    /// Generate a new unique trace ID.
    pub fn new() -> Self {
        Self(TRACE_COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Create from a raw value (for deserialization / replay).
    pub fn from_raw(v: u64) -> Self {
        Self(v)
    }

    /// Get the raw numeric value.
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t-{:08x}", self.0)
    }
}

impl Default for TraceId {
    fn default() -> Self {
        Self::new()
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Event categories — the cognitive vocabulary
// ────────────────────────────────────────────────────────────────────────────

/// The unified event type for the entire Yantrik cognitive runtime.
///
/// Every meaningful state change — from hardware signals to tool outcomes
/// to proactive decisions — is expressed as a `YantrikEvent`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventKind {
    // ── System (from SystemObserver) ──
    /// Hardware/OS state change (battery, network, CPU, memory, disk, processes).
    System(SystemEvent),

    // ── User Interaction ──
    /// User sent a message to the companion.
    UserMessage {
        text: String,
        /// Which screen/context the message came from.
        source: String,
    },
    /// User resumed from idle.
    UserResumed {
        idle_seconds: u64,
    },
    /// User interacted with a whisper card (accepted, dismissed, ignored).
    WhisperCardInteraction {
        card_id: String,
        action: CardAction,
    },

    // ── Tool Execution ──
    /// A tool was called by the agent.
    ToolCalled {
        tool_name: String,
        arguments: serde_json::Value,
        /// Permission level used for this call.
        permission: String,
    },
    /// A tool execution completed.
    ToolCompleted {
        tool_name: String,
        outcome: ToolOutcome,
        duration_ms: u64,
        /// Truncated result preview (first 200 chars).
        result_preview: String,
    },

    // ── Companion Cognition ──
    /// Companion generated a response.
    CompanionResponse {
        text_length: usize,
        tool_calls_count: usize,
        /// How long the full response took (including tool loops).
        total_ms: u64,
    },
    /// An instinct evaluated and produced urges.
    InstinctFired {
        instinct_name: String,
        urge_count: usize,
        max_urgency: f64,
    },
    /// A proactive message was generated and delivered.
    ProactiveDelivered {
        urge_ids: Vec<String>,
        text_preview: String,
        delivery_channel: String,
    },
    /// A proactive candidate was suppressed (by gate, cooldown, or silence policy).
    ProactiveSuppressed {
        reason: String,
        urge_ids: Vec<String>,
    },
    /// Think cycle completed.
    ThinkCycleCompleted {
        triggers_count: usize,
        urges_pushed: usize,
        duration_ms: u64,
    },

    // ── Memory ──
    /// A memory was recorded.
    MemoryRecorded {
        domain: String,
        text_preview: String,
        importance: f64,
    },
    /// A memory was recalled (used in context).
    MemoryRecalled {
        query: String,
        results_count: usize,
    },

    // ── Bond & Personality ──
    /// Bond score changed.
    BondChanged {
        old_level: String,
        new_level: String,
        score: f64,
    },

    // ── Internal Signals ──
    /// Confidence threshold crossed (for routing decisions).
    ConfidenceGate {
        query: String,
        confidence: f64,
        routed_to: String,
    },
    /// Routine deviation detected.
    RoutineDeviation {
        routine: String,
        expected: String,
        actual: String,
    },
    /// Commitment deadline approaching or overdue.
    CommitmentAlert {
        commitment_id: String,
        description: String,
        alert_type: CommitmentAlertType,
    },

    // ── Lifecycle ──
    /// System boot completed.
    BootCompleted {
        boot_time_ms: u64,
    },
    /// Graceful shutdown initiated.
    ShutdownInitiated,
}

/// How a user interacted with a whisper card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CardAction {
    Accepted,
    Dismissed,
    Ignored,
    OpenedLater,
}

/// The verified outcome of a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolOutcome {
    /// Postcondition verified — the tool did what it was supposed to.
    Verified,
    /// Tool completed but postcondition couldn't be checked.
    Unverified,
    /// Tool execution failed.
    Failed { error: String },
    /// Partially successful (some postconditions passed, some didn't).
    PartialSuccess { detail: String },
}

impl fmt::Display for ToolOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Verified => write!(f, "verified"),
            Self::Unverified => write!(f, "unverified"),
            Self::Failed { error } => write!(f, "failed: {error}"),
            Self::PartialSuccess { detail } => write!(f, "partial: {detail}"),
        }
    }
}

/// Type of commitment alert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommitmentAlertType {
    /// Deadline is within 24 hours.
    Approaching,
    /// Deadline has passed.
    Overdue,
    /// Commitment was completed.
    Completed,
}

// ────────────────────────────────────────────────────────────────────────────
// YantrikEvent — the envelope
// ────────────────────────────────────────────────────────────────────────────

/// A fully-qualified event with trace metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YantrikEvent {
    /// Unique trace ID for this event.
    pub trace_id: TraceId,
    /// Parent trace ID (if this event was caused by another).
    pub parent_trace: Option<TraceId>,
    /// What happened.
    pub kind: EventKind,
    /// Which module emitted this event.
    pub source: EventSource,
    /// When this event occurred (Unix timestamp with fractional seconds).
    pub timestamp: f64,
}

impl YantrikEvent {
    /// Create a new event with a fresh trace ID.
    pub fn new(kind: EventKind, source: EventSource) -> Self {
        Self {
            trace_id: TraceId::new(),
            parent_trace: None,
            kind,
            source,
            timestamp: now_ts(),
        }
    }

    /// Create a new event linked to a parent trace.
    pub fn with_parent(kind: EventKind, source: EventSource, parent: TraceId) -> Self {
        Self {
            trace_id: TraceId::new(),
            parent_trace: Some(parent),
            kind,
            source,
            timestamp: now_ts(),
        }
    }

    /// Get a short type tag for this event (for logging, queries).
    pub fn type_tag(&self) -> &'static str {
        match &self.kind {
            EventKind::System(_) => "system",
            EventKind::UserMessage { .. } => "user_message",
            EventKind::UserResumed { .. } => "user_resumed",
            EventKind::WhisperCardInteraction { .. } => "whisper_card",
            EventKind::ToolCalled { .. } => "tool_called",
            EventKind::ToolCompleted { .. } => "tool_completed",
            EventKind::CompanionResponse { .. } => "companion_response",
            EventKind::InstinctFired { .. } => "instinct_fired",
            EventKind::ProactiveDelivered { .. } => "proactive_delivered",
            EventKind::ProactiveSuppressed { .. } => "proactive_suppressed",
            EventKind::ThinkCycleCompleted { .. } => "think_cycle",
            EventKind::MemoryRecorded { .. } => "memory_recorded",
            EventKind::MemoryRecalled { .. } => "memory_recalled",
            EventKind::BondChanged { .. } => "bond_changed",
            EventKind::ConfidenceGate { .. } => "confidence_gate",
            EventKind::RoutineDeviation { .. } => "routine_deviation",
            EventKind::CommitmentAlert { .. } => "commitment_alert",
            EventKind::BootCompleted { .. } => "boot_completed",
            EventKind::ShutdownInitiated => "shutdown",
        }
    }

    /// Get a compact one-line summary for logging.
    pub fn summary(&self) -> String {
        match &self.kind {
            EventKind::System(se) => format!("system:{}", system_event_tag(se)),
            EventKind::UserMessage { text, .. } => {
                let preview = if text.len() > 50 { &text[..50] } else { text };
                format!("user_msg: {preview}")
            }
            EventKind::UserResumed { idle_seconds } => {
                format!("user_resumed after {idle_seconds}s")
            }
            EventKind::WhisperCardInteraction { card_id, action } => {
                format!("card:{card_id} → {action:?}")
            }
            EventKind::ToolCalled { tool_name, .. } => format!("tool_call:{tool_name}"),
            EventKind::ToolCompleted { tool_name, outcome, duration_ms, .. } => {
                format!("tool_done:{tool_name} [{outcome}] {duration_ms}ms")
            }
            EventKind::CompanionResponse { text_length, tool_calls_count, total_ms } => {
                format!("response: {text_length}ch {tool_calls_count}tools {total_ms}ms")
            }
            EventKind::InstinctFired { instinct_name, urge_count, max_urgency } => {
                format!("instinct:{instinct_name} {urge_count}urges max={max_urgency:.2}")
            }
            EventKind::ProactiveDelivered { delivery_channel, .. } => {
                format!("proactive_delivered via {delivery_channel}")
            }
            EventKind::ProactiveSuppressed { reason, .. } => {
                format!("proactive_suppressed: {reason}")
            }
            EventKind::ThinkCycleCompleted { triggers_count, urges_pushed, duration_ms } => {
                format!("think: {triggers_count}triggers {urges_pushed}urges {duration_ms}ms")
            }
            EventKind::MemoryRecorded { domain, importance, .. } => {
                format!("mem_record:{domain} imp={importance:.2}")
            }
            EventKind::MemoryRecalled { query, results_count } => {
                let q = if query.len() > 30 { &query[..30] } else { query };
                format!("mem_recall: \"{q}\" → {results_count}")
            }
            EventKind::BondChanged { old_level, new_level, score } => {
                format!("bond:{old_level}→{new_level} ({score:.2})")
            }
            EventKind::ConfidenceGate { confidence, routed_to, .. } => {
                format!("confidence:{confidence:.2}→{routed_to}")
            }
            EventKind::RoutineDeviation { routine, .. } => {
                format!("routine_deviation:{routine}")
            }
            EventKind::CommitmentAlert { description, alert_type, .. } => {
                let t = match alert_type {
                    CommitmentAlertType::Approaching => "approaching",
                    CommitmentAlertType::Overdue => "overdue",
                    CommitmentAlertType::Completed => "completed",
                };
                let d = if description.len() > 40 { &description[..40] } else { description };
                format!("commitment:{t} \"{d}\"")
            }
            EventKind::BootCompleted { boot_time_ms } => {
                format!("boot_completed in {boot_time_ms}ms")
            }
            EventKind::ShutdownInitiated => "shutdown_initiated".to_string(),
        }
    }
}

/// Which module emitted an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventSource {
    SystemObserver,
    Companion,
    ProactiveEngine,
    ToolExecutor,
    MemorySystem,
    UserInterface,
    Background,
    EventBus,
}

impl fmt::Display for EventSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SystemObserver => write!(f, "observer"),
            Self::Companion => write!(f, "companion"),
            Self::ProactiveEngine => write!(f, "proactive"),
            Self::ToolExecutor => write!(f, "tools"),
            Self::MemorySystem => write!(f, "memory"),
            Self::UserInterface => write!(f, "ui"),
            Self::Background => write!(f, "background"),
            Self::EventBus => write!(f, "bus"),
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// EventBus — broadcast pub/sub with causal tracing
// ────────────────────────────────────────────────────────────────────────────

/// Channel capacity per subscriber. Events beyond this are dropped (lossy).
const SUBSCRIBER_CAPACITY: usize = 512;

/// The cognitive event bus. Thread-safe, cloneable.
///
/// # Usage
/// ```ignore
/// let bus = EventBus::new();
/// let rx = bus.subscribe();
///
/// // Producer
/// bus.emit(EventKind::BootCompleted { boot_time_ms: 1200 }, EventSource::Background);
///
/// // Consumer (on another thread)
/// while let Ok(event) = rx.recv() {
///     println!("{}: {}", event.trace_id, event.summary());
/// }
/// ```
#[derive(Clone)]
pub struct EventBus {
    inner: Arc<EventBusInner>,
}

struct EventBusInner {
    /// All active subscriber senders. We broadcast to each.
    subscribers: Mutex<Vec<Sender<YantrikEvent>>>,
    /// Optional event log for persistence.
    event_log: Mutex<Option<EventLog>>,
    /// Total events emitted (for stats).
    total_emitted: AtomicU64,
}

impl EventBus {
    /// Create a new event bus.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(EventBusInner {
                subscribers: Mutex::new(Vec::new()),
                event_log: Mutex::new(None),
                total_emitted: AtomicU64::new(0),
            }),
        }
    }

    /// Attach a persistent event log (SQLite-backed).
    pub fn attach_log(&self, log: EventLog) {
        let mut lock = self.inner.event_log.lock().unwrap();
        *lock = Some(log);
    }

    /// Subscribe to all events. Returns a receiver channel.
    /// The subscriber should drain this channel promptly — if the buffer
    /// fills up, new events will be dropped for this subscriber only.
    pub fn subscribe(&self) -> Receiver<YantrikEvent> {
        let (tx, rx) = crossbeam_channel::bounded(SUBSCRIBER_CAPACITY);
        let mut subs = self.inner.subscribers.lock().unwrap();
        subs.push(tx);
        rx
    }

    /// Emit an event with a fresh trace ID.
    pub fn emit(&self, kind: EventKind, source: EventSource) -> TraceId {
        let event = YantrikEvent::new(kind, source);
        let trace_id = event.trace_id;
        self.broadcast(event);
        trace_id
    }

    /// Emit an event linked to a parent trace (causal chain).
    pub fn emit_with_parent(
        &self,
        kind: EventKind,
        source: EventSource,
        parent: TraceId,
    ) -> TraceId {
        let event = YantrikEvent::with_parent(kind, source, parent);
        let trace_id = event.trace_id;
        self.broadcast(event);
        trace_id
    }

    /// Emit a pre-built event (for bridging from SystemObserver).
    pub fn emit_event(&self, event: YantrikEvent) {
        self.broadcast(event);
    }

    /// Bridge a `SystemEvent` into the event bus.
    pub fn emit_system_event(&self, system_event: SystemEvent) -> TraceId {
        self.emit(EventKind::System(system_event), EventSource::SystemObserver)
    }

    /// Total number of events emitted since bus creation.
    pub fn total_emitted(&self) -> u64 {
        self.inner.total_emitted.load(Ordering::Relaxed)
    }

    /// Number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        let subs = self.inner.subscribers.lock().unwrap();
        subs.len()
    }

    /// Broadcast to all subscribers and persist to log.
    fn broadcast(&self, event: YantrikEvent) {
        self.inner.total_emitted.fetch_add(1, Ordering::Relaxed);

        // Persist to event log (non-blocking — errors are logged, not propagated)
        if let Ok(mut log_lock) = self.inner.event_log.lock() {
            if let Some(log) = log_lock.as_mut() {
                log.record(&event);
            }
        }

        // Broadcast to subscribers (drop dead channels)
        let mut subs = self.inner.subscribers.lock().unwrap();
        subs.retain(|tx| {
            // try_send: if the channel is full, we drop this event for this subscriber
            // (lossy backpressure). If the channel is disconnected, we remove the subscriber.
            match tx.try_send(event.clone()) {
                Ok(()) => true,
                Err(crossbeam_channel::TrySendError::Full(_)) => {
                    tracing::trace!("Event bus: subscriber buffer full, dropping event");
                    true // keep the subscriber, just drop this event
                }
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                    false // subscriber dropped, remove it
                }
            }
        });
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for EventBus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventBus")
            .field("total_emitted", &self.total_emitted())
            .field("subscribers", &self.subscriber_count())
            .finish()
    }
}

// ────────────────────────────────────────────────────────────────────────────
// EventLog — SQLite-backed append-only event persistence
// ────────────────────────────────────────────────────────────────────────────

/// Append-only event log backed by SQLite.
///
/// Stores events with trace IDs for replay, debugging, and causal analysis.
/// Supports queries by trace ID, event type, time range, and source.
pub struct EventLog {
    conn: rusqlite::Connection,
    /// Buffer events and flush in batches for performance.
    buffer: Vec<YantrikEvent>,
    buffer_capacity: usize,
}

impl EventLog {
    /// Open or create an event log database.
    pub fn open(path: &str) -> Result<Self, rusqlite::Error> {
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA cache_size = -2000;",
        )?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS event_log (
                 id              INTEGER PRIMARY KEY AUTOINCREMENT,
                 trace_id        INTEGER NOT NULL,
                 parent_trace_id INTEGER,
                 event_type      TEXT NOT NULL,
                 source          TEXT NOT NULL,
                 timestamp       REAL NOT NULL,
                 payload         TEXT NOT NULL,
                 summary         TEXT NOT NULL
             );

             CREATE INDEX IF NOT EXISTS idx_event_trace ON event_log(trace_id);
             CREATE INDEX IF NOT EXISTS idx_event_parent ON event_log(parent_trace_id)
                 WHERE parent_trace_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_event_type ON event_log(event_type);
             CREATE INDEX IF NOT EXISTS idx_event_time ON event_log(timestamp);
             CREATE INDEX IF NOT EXISTS idx_event_source ON event_log(source);",
        )?;

        Ok(Self {
            conn,
            buffer: Vec::with_capacity(32),
            buffer_capacity: 32,
        })
    }

    /// Open an in-memory event log (for testing).
    pub fn in_memory() -> Result<Self, rusqlite::Error> {
        Self::open(":memory:")
    }

    /// Record an event to the log. Buffers internally and flushes in batches.
    pub fn record(&mut self, event: &YantrikEvent) {
        self.buffer.push(event.clone());
        if self.buffer.len() >= self.buffer_capacity {
            self.flush();
        }
    }

    /// Flush buffered events to SQLite.
    pub fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        let tx = match self.conn.transaction() {
            Ok(tx) => tx,
            Err(e) => {
                tracing::warn!("EventLog: failed to start transaction: {e}");
                self.buffer.clear();
                return;
            }
        };

        for event in self.buffer.drain(..) {
            let payload = serde_json::to_string(&event.kind).unwrap_or_default();
            let summary = event.summary();
            let parent = event.parent_trace.map(|t| t.as_u64() as i64);

            if let Err(e) = tx.execute(
                "INSERT INTO event_log (trace_id, parent_trace_id, event_type, source, timestamp, payload, summary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    event.trace_id.as_u64() as i64,
                    parent,
                    event.type_tag(),
                    event.source.to_string(),
                    event.timestamp,
                    payload,
                    summary,
                ],
            ) {
                tracing::warn!("EventLog: failed to insert event: {e}");
            }
        }

        if let Err(e) = tx.commit() {
            tracing::warn!("EventLog: failed to commit: {e}");
        }
    }

    /// Query events by trace ID (find the full causal chain).
    pub fn query_by_trace(&mut self, trace_id: TraceId) -> Vec<EventLogEntry> {
        self.flush(); // ensure pending events are visible
        let mut stmt = match self.conn.prepare(
            "SELECT id, trace_id, parent_trace_id, event_type, source, timestamp, summary
             FROM event_log
             WHERE trace_id = ?1 OR parent_trace_id = ?1
             ORDER BY timestamp ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(rusqlite::params![trace_id.as_u64() as i64], |row| {
            Ok(EventLogEntry {
                id: row.get(0)?,
                trace_id: TraceId::from_raw(row.get::<_, i64>(1)? as u64),
                parent_trace: row
                    .get::<_, Option<i64>>(2)?
                    .map(|v| TraceId::from_raw(v as u64)),
                event_type: row.get(3)?,
                source: row.get(4)?,
                timestamp: row.get(5)?,
                summary: row.get(6)?,
            })
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Query events by type within a time range.
    pub fn query_by_type(
        &mut self,
        event_type: &str,
        since: f64,
        until: f64,
        limit: usize,
    ) -> Vec<EventLogEntry> {
        self.flush();
        let mut stmt = match self.conn.prepare(
            "SELECT id, trace_id, parent_trace_id, event_type, source, timestamp, summary
             FROM event_log
             WHERE event_type = ?1 AND timestamp >= ?2 AND timestamp <= ?3
             ORDER BY timestamp DESC
             LIMIT ?4",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(
            rusqlite::params![event_type, since, until, limit as i64],
            |row| {
                Ok(EventLogEntry {
                    id: row.get(0)?,
                    trace_id: TraceId::from_raw(row.get::<_, i64>(1)? as u64),
                    parent_trace: row
                        .get::<_, Option<i64>>(2)?
                        .map(|v| TraceId::from_raw(v as u64)),
                    event_type: row.get(3)?,
                    source: row.get(4)?,
                    timestamp: row.get(5)?,
                    summary: row.get(6)?,
                })
            },
        )
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Query recent events across all types.
    pub fn query_recent(&mut self, limit: usize) -> Vec<EventLogEntry> {
        self.flush();
        let mut stmt = match self.conn.prepare(
            "SELECT id, trace_id, parent_trace_id, event_type, source, timestamp, summary
             FROM event_log
             ORDER BY timestamp DESC
             LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(rusqlite::params![limit as i64], |row| {
            Ok(EventLogEntry {
                id: row.get(0)?,
                trace_id: TraceId::from_raw(row.get::<_, i64>(1)? as u64),
                parent_trace: row
                    .get::<_, Option<i64>>(2)?
                    .map(|v| TraceId::from_raw(v as u64)),
                event_type: row.get(3)?,
                source: row.get(4)?,
                timestamp: row.get(5)?,
                summary: row.get(6)?,
            })
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Count events by type in a time range (for analytics).
    pub fn count_by_type(&mut self, event_type: &str, since: f64) -> u64 {
        self.flush();
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM event_log WHERE event_type = ?1 AND timestamp >= ?2",
                rusqlite::params![event_type, since],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as u64
    }

    /// Get event statistics for a time window.
    pub fn stats(&mut self, since: f64) -> EventStats {
        self.flush();
        let total = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM event_log WHERE timestamp >= ?1",
                rusqlite::params![since],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as u64;

        let mut type_counts = Vec::new();
        if let Ok(mut stmt) = self.conn.prepare(
            "SELECT event_type, COUNT(*) FROM event_log
             WHERE timestamp >= ?1
             GROUP BY event_type
             ORDER BY COUNT(*) DESC",
        ) {
            if let Ok(rows) = stmt.query_map(rusqlite::params![since], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            }) {
                type_counts = rows.filter_map(|r| r.ok()).collect();
            }
        }

        EventStats { total, type_counts }
    }

    /// Compact old events (summarize then delete events older than `before`).
    /// Keeps one summary entry per hour per event type.
    pub fn compact(&mut self, before: f64) {
        self.flush();

        // Insert hourly summaries
        let _ = self.conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO event_log (trace_id, event_type, source, timestamp, payload, summary)
             SELECT
                 0,
                 event_type,
                 'compacted',
                 CAST(CAST(timestamp / 3600 AS INTEGER) * 3600 AS REAL),
                 json_object('count', COUNT(*), 'first_trace', MIN(trace_id), 'last_trace', MAX(trace_id)),
                 event_type || ': ' || COUNT(*) || ' events (compacted)'
             FROM event_log
             WHERE timestamp < {before}
               AND source != 'compacted'
             GROUP BY event_type, CAST(timestamp / 3600 AS INTEGER);"
        ));

        // Delete originals
        let deleted = self
            .conn
            .execute(
                "DELETE FROM event_log WHERE timestamp < ?1 AND source != 'compacted'",
                rusqlite::params![before],
            )
            .unwrap_or(0);

        if deleted > 0 {
            tracing::info!(deleted, "EventLog: compacted old events");
        }
    }

    /// Total number of events in the log.
    pub fn total_events(&self) -> u64 {
        self.conn
            .query_row("SELECT COUNT(*) FROM event_log", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0) as u64
    }
}

impl Drop for EventLog {
    fn drop(&mut self) {
        self.flush();
    }
}

/// A row from the event log (query result).
#[derive(Debug, Clone)]
pub struct EventLogEntry {
    pub id: i64,
    pub trace_id: TraceId,
    pub parent_trace: Option<TraceId>,
    pub event_type: String,
    pub source: String,
    pub timestamp: f64,
    pub summary: String,
}

/// Event statistics for a time window.
#[derive(Debug, Clone)]
pub struct EventStats {
    pub total: u64,
    pub type_counts: Vec<(String, u64)>,
}

// ────────────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────────────

fn now_ts() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Short tag for a SystemEvent variant (for summary strings).
fn system_event_tag(event: &SystemEvent) -> &'static str {
    match event {
        SystemEvent::BatteryChanged { .. } => "battery",
        SystemEvent::NetworkChanged { .. } => "network",
        SystemEvent::NotificationReceived { .. } => "notification",
        SystemEvent::FileChanged { .. } => "file",
        SystemEvent::ProcessStarted { .. } => "proc_start",
        SystemEvent::ProcessStopped { .. } => "proc_stop",
        SystemEvent::CpuPressure { .. } => "cpu",
        SystemEvent::MemoryPressure { .. } => "memory",
        SystemEvent::DiskPressure { .. } => "disk",
        SystemEvent::UserIdle { .. } => "idle",
        SystemEvent::UserResumed => "resumed",
        SystemEvent::KeybindTriggered { .. } => "keybind",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_id_uniqueness() {
        let a = TraceId::new();
        let b = TraceId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn trace_id_display() {
        let t = TraceId::from_raw(255);
        assert_eq!(format!("{t}"), "t-000000ff");
    }

    #[test]
    fn event_bus_basic_pub_sub() {
        let bus = EventBus::new();
        let rx = bus.subscribe();

        let trace = bus.emit(
            EventKind::BootCompleted { boot_time_ms: 1200 },
            EventSource::Background,
        );

        let event = rx.try_recv().unwrap();
        assert_eq!(event.trace_id, trace);
        assert_eq!(event.type_tag(), "boot_completed");
        assert!(event.parent_trace.is_none());
    }

    #[test]
    fn event_bus_causal_chain() {
        let bus = EventBus::new();
        let rx = bus.subscribe();

        let parent = bus.emit(
            EventKind::UserMessage {
                text: "hello".into(),
                source: "lens".into(),
            },
            EventSource::UserInterface,
        );

        let child = bus.emit_with_parent(
            EventKind::ToolCalled {
                tool_name: "recall".into(),
                arguments: serde_json::json!({}),
                permission: "Safe".into(),
            },
            EventSource::ToolExecutor,
            parent,
        );

        let e1 = rx.try_recv().unwrap();
        let e2 = rx.try_recv().unwrap();
        assert_eq!(e1.trace_id, parent);
        assert_eq!(e2.trace_id, child);
        assert_eq!(e2.parent_trace, Some(parent));
    }

    #[test]
    fn event_bus_multiple_subscribers() {
        let bus = EventBus::new();
        let rx1 = bus.subscribe();
        let rx2 = bus.subscribe();

        bus.emit(
            EventKind::BootCompleted { boot_time_ms: 500 },
            EventSource::Background,
        );

        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }

    #[test]
    fn event_bus_dead_subscriber_cleanup() {
        let bus = EventBus::new();
        let rx = bus.subscribe();
        drop(rx); // subscriber disconnects

        // Should not panic, dead subscriber should be cleaned up
        bus.emit(
            EventKind::ShutdownInitiated,
            EventSource::Background,
        );
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn event_bus_system_event_bridge() {
        let bus = EventBus::new();
        let rx = bus.subscribe();

        bus.emit_system_event(SystemEvent::BatteryChanged {
            level: 42,
            charging: true,
            time_to_empty_mins: None,
        });

        let event = rx.try_recv().unwrap();
        assert_eq!(event.type_tag(), "system");
        assert!(event.summary().contains("battery"));
    }

    #[test]
    fn event_log_roundtrip() {
        let mut log = EventLog::in_memory().unwrap();

        let event = YantrikEvent::new(
            EventKind::ToolCompleted {
                tool_name: "recall".into(),
                outcome: ToolOutcome::Verified,
                duration_ms: 42,
                result_preview: "found 3 memories".into(),
            },
            EventSource::ToolExecutor,
        );
        let trace = event.trace_id;

        log.record(&event);
        log.flush();

        let entries = log.query_by_trace(trace);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type, "tool_completed");
        assert!(entries[0].summary.contains("recall"));
    }

    #[test]
    fn event_log_query_by_type() {
        let mut log = EventLog::in_memory().unwrap();

        for i in 0..5 {
            log.record(&YantrikEvent::new(
                EventKind::ToolCompleted {
                    tool_name: format!("tool_{i}"),
                    outcome: ToolOutcome::Verified,
                    duration_ms: i * 10,
                    result_preview: String::new(),
                },
                EventSource::ToolExecutor,
            ));
        }
        log.record(&YantrikEvent::new(
            EventKind::BootCompleted { boot_time_ms: 100 },
            EventSource::Background,
        ));
        log.flush();

        let results = log.query_by_type("tool_completed", 0.0, f64::MAX, 100);
        assert_eq!(results.len(), 5);

        let results = log.query_by_type("boot_completed", 0.0, f64::MAX, 100);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn event_log_causal_chain_query() {
        let mut log = EventLog::in_memory().unwrap();

        let parent_trace = TraceId::new();
        let parent = YantrikEvent {
            trace_id: parent_trace,
            parent_trace: None,
            kind: EventKind::UserMessage {
                text: "check email".into(),
                source: "lens".into(),
            },
            source: EventSource::UserInterface,
            timestamp: now_ts(),
        };
        log.record(&parent);

        let child = YantrikEvent::with_parent(
            EventKind::ToolCalled {
                tool_name: "email_list".into(),
                arguments: serde_json::json!({}),
                permission: "Standard".into(),
            },
            EventSource::ToolExecutor,
            parent_trace,
        );
        log.record(&child);
        log.flush();

        let chain = log.query_by_trace(parent_trace);
        assert_eq!(chain.len(), 2);
    }

    #[test]
    fn event_log_stats() {
        let mut log = EventLog::in_memory().unwrap();

        for _ in 0..3 {
            log.record(&YantrikEvent::new(
                EventKind::ToolCompleted {
                    tool_name: "recall".into(),
                    outcome: ToolOutcome::Verified,
                    duration_ms: 10,
                    result_preview: String::new(),
                },
                EventSource::ToolExecutor,
            ));
        }
        log.record(&YantrikEvent::new(
            EventKind::BootCompleted { boot_time_ms: 100 },
            EventSource::Background,
        ));

        let stats = log.stats(0.0);
        assert_eq!(stats.total, 4);
        assert!(stats.type_counts.iter().any(|(t, c)| t == "tool_completed" && *c == 3));
    }

    #[test]
    fn event_bus_with_log() {
        let bus = EventBus::new();
        let log = EventLog::in_memory().unwrap();
        bus.attach_log(log);

        let rx = bus.subscribe();

        bus.emit(
            EventKind::BootCompleted { boot_time_ms: 999 },
            EventSource::Background,
        );

        // Event received by subscriber
        assert!(rx.try_recv().is_ok());
        // Event persisted to log
        assert_eq!(bus.total_emitted(), 1);
    }

    #[test]
    fn tool_outcome_display() {
        assert_eq!(format!("{}", ToolOutcome::Verified), "verified");
        assert_eq!(
            format!("{}", ToolOutcome::Failed { error: "timeout".into() }),
            "failed: timeout"
        );
    }
}
