//! Event-driven proactive activation.
//!
//! Replaces fixed-interval Think polling with reactive event processing.
//! Significant events (user resumed, tool completed, commitment alert)
//! trigger immediate lightweight instinct evaluation.
//! Periodic maintenance (memory consolidation, decay) remains on a slower timer.

use std::time::{Duration, Instant};

/// Events that should trigger immediate instinct evaluation.
#[derive(Debug, Clone)]
pub enum SignificantEvent {
    /// User sent a message — may contain commitments, requests.
    UserMessage { text_length: usize },
    /// User returned from idle — good time for briefing.
    UserResumed { idle_seconds: u64 },
    /// Tool completed — may have produced new information.
    ToolCompleted { tool_name: String, success: bool },
    /// Commitment deadline approaching or passed.
    CommitmentAlert { description: String },
    /// System state changed significantly.
    SystemChange { kind: String },
    /// Periodic maintenance tick (reduced frequency).
    MaintenanceTick,
}

/// Configuration for event-driven activation.
#[derive(Debug, Clone)]
pub struct EventDrivenConfig {
    /// Minimum interval between reactive evaluations (prevent thrashing).
    pub min_eval_interval: Duration,
    /// Batch window — collect events for this long before evaluating.
    pub batch_window: Duration,
    /// Maintenance interval (replaces the old 60s think timer).
    pub maintenance_interval: Duration,
    /// Maximum events to batch before forcing evaluation.
    pub max_batch_size: usize,
}

impl Default for EventDrivenConfig {
    fn default() -> Self {
        Self {
            min_eval_interval: Duration::from_secs(10),
            batch_window: Duration::from_secs(3),
            maintenance_interval: Duration::from_secs(120),
            max_batch_size: 5,
        }
    }
}

/// Tracks event-driven activation state.
pub struct EventDrivenState {
    pub config: EventDrivenConfig,
    /// When the last evaluation was performed.
    pub last_eval: Instant,
    /// When the last maintenance cycle ran.
    pub last_maintenance: Instant,
    /// Pending events waiting to be batched.
    pub pending_events: Vec<SignificantEvent>,
    /// When the first pending event arrived (batch window start).
    pub batch_start: Option<Instant>,
    /// Total reactive evaluations performed.
    pub reactive_eval_count: u64,
    /// Total maintenance cycles performed.
    pub maintenance_count: u64,
}

impl EventDrivenState {
    /// Create a new state tracker. Both `last_eval` and `last_maintenance`
    /// are backdated by their respective intervals so the first event/tick
    /// can fire immediately.
    pub fn new(config: EventDrivenConfig) -> Self {
        let now = Instant::now();
        Self {
            last_eval: now - config.min_eval_interval,
            last_maintenance: now - config.maintenance_interval,
            pending_events: Vec::new(),
            batch_start: None,
            reactive_eval_count: 0,
            maintenance_count: 0,
            config,
        }
    }

    /// Record an incoming significant event. Returns `true` if evaluation
    /// should trigger now.
    pub fn push_event(&mut self, event: SignificantEvent) -> bool {
        // High-priority events bypass batching.
        let immediate = matches!(
            &event,
            SignificantEvent::UserResumed { idle_seconds } if *idle_seconds > 300
        ) || matches!(&event, SignificantEvent::CommitmentAlert { .. });

        self.pending_events.push(event);

        if self.batch_start.is_none() {
            self.batch_start = Some(Instant::now());
        }

        if immediate {
            return self.can_evaluate();
        }

        // Check batch-full condition.
        if self.pending_events.len() >= self.config.max_batch_size {
            return self.can_evaluate();
        }

        // Check batch window expiry.
        if let Some(start) = self.batch_start {
            if start.elapsed() >= self.config.batch_window {
                return self.can_evaluate();
            }
        }

        false
    }

    /// Check if enough time has passed since last evaluation.
    fn can_evaluate(&self) -> bool {
        self.last_eval.elapsed() >= self.config.min_eval_interval
    }

    /// Check if maintenance is due.
    pub fn maintenance_due(&self) -> bool {
        self.last_maintenance.elapsed() >= self.config.maintenance_interval
    }

    /// Drain pending events and mark evaluation as performed.
    pub fn drain_for_eval(&mut self) -> Vec<SignificantEvent> {
        self.last_eval = Instant::now();
        self.batch_start = None;
        self.reactive_eval_count += 1;
        std::mem::take(&mut self.pending_events)
    }

    /// Mark maintenance as performed.
    pub fn mark_maintenance(&mut self) {
        self.last_maintenance = Instant::now();
        self.maintenance_count += 1;
    }

    /// Summary for logging/tracing.
    pub fn stats_summary(&self) -> String {
        format!(
            "reactive_evals={}, maintenance={}, pending={}",
            self.reactive_eval_count,
            self.maintenance_count,
            self.pending_events.len()
        )
    }
}

/// Map an EventBus `EventKind` type tag and payload to a `SignificantEvent`.
///
/// Returns `None` for events that should not trigger reactive evaluation
/// (e.g. memory recalls, instinct firings, proactive deliveries — these are
/// *outputs* of the companion, not inputs that should re-trigger it).
pub fn classify_event(kind: &str, payload: &str) -> Option<SignificantEvent> {
    match kind {
        k if k.contains("user_message") || k.contains("UserMessage") => {
            Some(SignificantEvent::UserMessage {
                text_length: payload.len(),
            })
        }
        k if k.contains("user_resumed") || k.contains("UserResumed") => {
            let idle = extract_idle_seconds(payload);
            Some(SignificantEvent::UserResumed {
                idle_seconds: idle,
            })
        }
        k if k.contains("tool_completed") || k.contains("ToolCompleted") => {
            let tool_name = extract_field(payload, "tool_name");
            let success =
                payload.contains("Verified") || payload.contains("verified") || payload.contains("success");
            Some(SignificantEvent::ToolCompleted { tool_name, success })
        }
        k if k.contains("commitment_alert") || k.contains("CommitmentAlert") => {
            let desc = extract_field(payload, "description");
            Some(SignificantEvent::CommitmentAlert { description: desc })
        }
        k if k.contains("BatteryChanged")
            || k.contains("battery")
            || k.contains("NetworkChanged")
            || k.contains("network") =>
        {
            Some(SignificantEvent::SystemChange {
                kind: k.to_string(),
            })
        }
        _ => None,
    }
}

/// Parse idle seconds from a payload string.
///
/// Tries to find a number following "idle_seconds" (JSON-style or summary-style).
/// Falls back to 0 if parsing fails.
fn extract_idle_seconds(payload: &str) -> u64 {
    // Try JSON-style: "idle_seconds":123 or "idle_seconds": 123
    if let Some(pos) = payload.find("idle_seconds") {
        let after = &payload[pos..];
        // Skip past the key and any ": or "= or whitespace
        for segment in after.split(|c: char| !c.is_ascii_digit()) {
            if !segment.is_empty() {
                if let Ok(v) = segment.parse::<u64>() {
                    return v;
                }
            }
        }
    }

    // Try summary-style: "user_resumed after 300s"
    if let Some(pos) = payload.find("after ") {
        let after = &payload[pos + 6..];
        let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(v) = num_str.parse::<u64>() {
            return v;
        }
    }

    0
}

/// Extract a named field value from a payload string.
///
/// Handles both JSON-style (`"field":"value"`) and summary-style (`field:value`).
fn extract_field(payload: &str, field: &str) -> String {
    // Try JSON-style: "field":"value" or "field": "value"
    if let Some(pos) = payload.find(field) {
        let after = &payload[pos + field.len()..];
        // Skip past separator chars (":, =, whitespace, quotes)
        let trimmed = after.trim_start_matches(|c: char| c == '"' || c == ':' || c == ' ' || c == '=');
        // Read until next delimiter
        let value: String = trimmed
            .chars()
            .take_while(|c| *c != '"' && *c != ',' && *c != '}' && *c != '\n')
            .collect();
        let v = value.trim().to_string();
        if !v.is_empty() {
            return v;
        }
    }

    // Fallback: return the full payload truncated
    let truncated: String = payload.chars().take(100).collect();
    truncated
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn config_fast() -> EventDrivenConfig {
        EventDrivenConfig {
            min_eval_interval: Duration::from_millis(50),
            batch_window: Duration::from_millis(20),
            maintenance_interval: Duration::from_millis(100),
            max_batch_size: 3,
        }
    }

    #[test]
    fn push_event_returns_false_within_min_eval_interval() {
        let mut config = config_fast();
        config.min_eval_interval = Duration::from_secs(60); // very long
        let mut state = EventDrivenState::new(config);
        // Force last_eval to now so the interval hasn't elapsed.
        state.last_eval = Instant::now();

        let result = state.push_event(SignificantEvent::UserMessage { text_length: 10 });
        assert!(!result, "should not trigger eval within min_eval_interval");
    }

    #[test]
    fn push_event_returns_true_for_immediate_user_resumed_long_idle() {
        let mut state = EventDrivenState::new(config_fast());
        // last_eval is backdated in new(), so can_evaluate() should be true.

        let result = state.push_event(SignificantEvent::UserResumed { idle_seconds: 600 });
        assert!(result, "UserResumed with long idle should trigger immediately");
    }

    #[test]
    fn push_event_returns_true_for_immediate_commitment_alert() {
        let mut state = EventDrivenState::new(config_fast());

        let result = state.push_event(SignificantEvent::CommitmentAlert {
            description: "meeting in 10 min".to_string(),
        });
        assert!(result, "CommitmentAlert should trigger immediately");
    }

    #[test]
    fn user_resumed_short_idle_does_not_trigger_immediately() {
        let mut state = EventDrivenState::new(config_fast());
        // Short idle (< 300s) should NOT be immediate priority.
        // With batch_size=3 and only 1 event, and fresh batch window, it won't fire.
        state.last_eval = Instant::now(); // reset so can_evaluate is false
        let config_interval = state.config.min_eval_interval;
        state.config.min_eval_interval = Duration::from_secs(60);

        let result = state.push_event(SignificantEvent::UserResumed { idle_seconds: 30 });
        assert!(!result, "short idle should not trigger immediately");

        // Restore for cleanup
        state.config.min_eval_interval = config_interval;
    }

    #[test]
    fn batch_fills_up_and_triggers_evaluation() {
        let mut state = EventDrivenState::new(config_fast());

        // Push events up to max_batch_size (3)
        let r1 = state.push_event(SignificantEvent::SystemChange { kind: "cpu".into() });
        let r2 = state.push_event(SignificantEvent::SystemChange { kind: "mem".into() });
        // First two should not trigger (batch not full, window not expired)
        // Third should trigger (batch full)
        let r3 = state.push_event(SignificantEvent::SystemChange { kind: "disk".into() });

        assert!(r3, "third event should trigger evaluation (batch full)");
        assert_eq!(state.pending_events.len(), 3);
    }

    #[test]
    fn batch_window_timeout_triggers_evaluation() {
        let config = EventDrivenConfig {
            min_eval_interval: Duration::from_millis(10),
            batch_window: Duration::from_millis(30),
            maintenance_interval: Duration::from_secs(120),
            max_batch_size: 100, // large so batch-full doesn't trigger
        };
        let mut state = EventDrivenState::new(config);

        // Push first event — sets batch_start
        let r1 = state.push_event(SignificantEvent::UserMessage { text_length: 5 });
        // Might or might not trigger depending on timing — we just need batch_start set.

        // Wait for batch window to expire
        thread::sleep(Duration::from_millis(40));

        // Push another event — batch window should have expired
        let r2 = state.push_event(SignificantEvent::UserMessage { text_length: 10 });
        assert!(r2, "should trigger after batch window expires");
    }

    #[test]
    fn maintenance_due_respects_interval() {
        let config = config_fast();
        let mut state = EventDrivenState::new(config);

        // Freshly created state has last_maintenance backdated, so maintenance is due.
        assert!(state.maintenance_due(), "maintenance should be due on fresh state");

        state.mark_maintenance();
        assert!(!state.maintenance_due(), "maintenance should not be due right after marking");

        thread::sleep(Duration::from_millis(110));
        assert!(state.maintenance_due(), "maintenance should be due after interval elapsed");
    }

    #[test]
    fn drain_for_eval_resets_state() {
        let mut state = EventDrivenState::new(config_fast());

        state.push_event(SignificantEvent::UserMessage { text_length: 42 });
        state.push_event(SignificantEvent::SystemChange { kind: "net".into() });

        assert_eq!(state.pending_events.len(), 2);
        assert!(state.batch_start.is_some());
        assert_eq!(state.reactive_eval_count, 0);

        let drained = state.drain_for_eval();

        assert_eq!(drained.len(), 2);
        assert!(state.pending_events.is_empty());
        assert!(state.batch_start.is_none());
        assert_eq!(state.reactive_eval_count, 1);
    }

    #[test]
    fn classify_event_maps_user_message() {
        let result = classify_event("user_message", "Hello, how are you?");
        assert!(result.is_some());
        match result.unwrap() {
            SignificantEvent::UserMessage { text_length } => {
                assert_eq!(text_length, 19);
            }
            other => panic!("expected UserMessage, got {:?}", other),
        }
    }

    #[test]
    fn classify_event_maps_tool_completed() {
        let payload = r#"tool_done:recall [verified] 42ms"#;
        let result = classify_event("tool_completed", payload);
        assert!(result.is_some());
        match result.unwrap() {
            SignificantEvent::ToolCompleted { tool_name, success } => {
                assert!(success, "payload contains 'verified'");
                // tool_name extracted from payload
                assert!(!tool_name.is_empty());
            }
            other => panic!("expected ToolCompleted, got {:?}", other),
        }
    }

    #[test]
    fn classify_event_maps_commitment_alert() {
        let payload = r#"{"description":"submit report","alert_type":"Approaching"}"#;
        let result = classify_event("commitment_alert", payload);
        assert!(result.is_some());
        match result.unwrap() {
            SignificantEvent::CommitmentAlert { description } => {
                assert!(description.contains("submit report"));
            }
            other => panic!("expected CommitmentAlert, got {:?}", other),
        }
    }

    #[test]
    fn classify_event_maps_user_resumed_with_idle_parsing() {
        let payload = "user_resumed after 450s";
        let result = classify_event("user_resumed", payload);
        assert!(result.is_some());
        match result.unwrap() {
            SignificantEvent::UserResumed { idle_seconds } => {
                assert_eq!(idle_seconds, 450);
            }
            other => panic!("expected UserResumed, got {:?}", other),
        }
    }

    #[test]
    fn classify_event_returns_none_for_irrelevant_events() {
        assert!(classify_event("memory_recorded", "some memory").is_none());
        assert!(classify_event("instinct_fired", "curiosity").is_none());
        assert!(classify_event("proactive_delivered", "greeting").is_none());
        assert!(classify_event("companion_response", "hello").is_none());
        assert!(classify_event("boot_completed", "1200ms").is_none());
    }

    #[test]
    fn classify_event_maps_system_change_battery() {
        let result = classify_event("battery", "level=42 charging=true");
        assert!(result.is_some());
        match result.unwrap() {
            SignificantEvent::SystemChange { kind } => {
                assert!(kind.contains("battery"));
            }
            other => panic!("expected SystemChange, got {:?}", other),
        }
    }

    #[test]
    fn stats_summary_format() {
        let mut state = EventDrivenState::new(config_fast());
        state.reactive_eval_count = 7;
        state.maintenance_count = 3;
        state.pending_events.push(SignificantEvent::MaintenanceTick);
        state.pending_events.push(SignificantEvent::MaintenanceTick);

        let summary = state.stats_summary();
        assert_eq!(summary, "reactive_evals=7, maintenance=3, pending=2");
    }

    #[test]
    fn extract_idle_seconds_json_format() {
        let payload = r#"{"idle_seconds":1234,"other":"field"}"#;
        assert_eq!(extract_idle_seconds(payload), 1234);
    }

    #[test]
    fn extract_idle_seconds_summary_format() {
        assert_eq!(extract_idle_seconds("user_resumed after 999s idle"), 999);
    }

    #[test]
    fn extract_idle_seconds_fallback_zero() {
        assert_eq!(extract_idle_seconds("no numbers here"), 0);
    }

    #[test]
    fn extract_field_json_format() {
        let payload = r#"{"tool_name":"recall","outcome":"verified"}"#;
        let name = extract_field(payload, "tool_name");
        assert_eq!(name, "recall");
    }

    #[test]
    fn extract_field_summary_format() {
        let payload = "description:submit the quarterly report by Friday";
        let desc = extract_field(payload, "description");
        assert!(desc.contains("submit"));
    }

    #[test]
    fn multiple_drain_cycles() {
        let mut state = EventDrivenState::new(config_fast());

        state.push_event(SignificantEvent::UserMessage { text_length: 10 });
        let first = state.drain_for_eval();
        assert_eq!(first.len(), 1);
        assert_eq!(state.reactive_eval_count, 1);

        state.push_event(SignificantEvent::SystemChange { kind: "cpu".into() });
        state.push_event(SignificantEvent::SystemChange { kind: "mem".into() });
        let second = state.drain_for_eval();
        assert_eq!(second.len(), 2);
        assert_eq!(state.reactive_eval_count, 2);

        // Empty drain
        let third = state.drain_for_eval();
        assert!(third.is_empty());
        assert_eq!(state.reactive_eval_count, 3);
    }
}
