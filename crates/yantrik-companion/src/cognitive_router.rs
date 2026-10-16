//! Cognitive Router — offline NLP first-pass with LLM fallback.
//!
//! Pipeline:
//! 1. Offline engine attempts classification + Plan IR generation
//! 2. If confidence >= 0.85 and all slots filled → execute directly
//! 3. If confidence 0.5–0.85 → use LLM to refine
//! 4. If confidence < 0.5 or creative/emotional → full LLM path
//!
//! Also handles the Trace Learning Flywheel:
//! - After successful LLM tool executions, records the trace
//! - Periodically analyzes traces to discover repeating patterns
//! - Distills patterns into Motifs for future offline handling

use rusqlite::{params, Connection};
use crate::offline_nlp::{
    self, ClassifiedIntent, Intent, MotifMemory, MotifStep, Plan, PlanNode, PlanSource, Slots,
};

/// Minimum confidence to execute a plan without LLM.
const OFFLINE_THRESHOLD: f64 = 0.85;
/// Confidence range where LLM refinement is attempted.
const REFINE_THRESHOLD: f64 = 0.50;

/// Routing decision made by the cognitive router.
#[derive(Debug, Clone)]
pub enum RoutingDecision {
    /// Execute plan directly via offline engine (no LLM needed).
    Offline {
        plan: Plan,
        reason: String,
    },
    /// Use LLM but with plan hints to guide tool selection.
    LlmWithHints {
        plan: Plan,
        hint_text: String,
        reason: String,
    },
    /// Full LLM path — open-ended, creative, or low confidence.
    FullLlm {
        reason: String,
    },
}

impl RoutingDecision {
    pub fn is_offline(&self) -> bool {
        matches!(self, Self::Offline { .. })
    }

    pub fn type_tag(&self) -> &'static str {
        match self {
            Self::Offline { .. } => "offline",
            Self::LlmWithHints { .. } => "llm_with_hints",
            Self::FullLlm { .. } => "full_llm",
        }
    }
}

/// Route a user query through the cognitive router.
pub fn route(query: &str, conn: &Connection) -> RoutingDecision {
    let classified = offline_nlp::classify_intent(query);
    let slots = offline_nlp::extract_slots(query);

    // Check motif memory for learned patterns (3+ observations)
    let motifs = MotifMemory::find_matching(conn, query, 3);
    if let Some(motif) = motifs.first() {
        if motif.success_rate >= 0.8 {
            // Build plan from motif
            let root = if motif.tool_chain.len() == 1 {
                PlanNode::ToolCall {
                    tool_name: motif.tool_chain[0].tool_name.clone(),
                    arguments: resolve_template(&motif.tool_chain[0].argument_template, &slots),
                }
            } else {
                PlanNode::Sequence(
                    motif
                        .tool_chain
                        .iter()
                        .map(|step| PlanNode::ToolCall {
                            tool_name: step.tool_name.clone(),
                            arguments: resolve_template(&step.argument_template, &slots),
                        })
                        .collect(),
                )
            };

            let plan = Plan {
                intent: classified.intent,
                confidence: motif.success_rate,
                root,
                slots,
                source: PlanSource::Motif,
            };

            return RoutingDecision::Offline {
                plan,
                reason: format!(
                    "Motif match: '{}' (observed {}x, {:.0}% success)",
                    motif.intent_pattern, motif.observation_count,
                    motif.success_rate * 100.0,
                ),
            };
        }
    }

    // Check conversational intents (greetings, etc.)
    if matches!(
        classified.intent,
        Intent::Greeting | Intent::Farewell | Intent::Thanks
    ) && classified.confidence >= 0.8
    {
        return RoutingDecision::FullLlm {
            reason: "Conversational — let LLM handle naturally".into(),
        };
    }

    // Check if intent is open-ended / creative
    if classified.intent == Intent::Unknown {
        return RoutingDecision::FullLlm {
            reason: format!(
                "Unknown intent (confidence {:.2}) — full LLM path",
                classified.confidence
            ),
        };
    }

    // Try to generate a plan
    match offline_nlp::generate_plan(&classified, &slots) {
        Some(plan) if plan.confidence >= OFFLINE_THRESHOLD => {
            RoutingDecision::Offline {
                reason: format!(
                    "High confidence ({:.2}) intent: {:?}",
                    plan.confidence, plan.intent
                ),
                plan,
            }
        }
        Some(plan) if plan.confidence >= REFINE_THRESHOLD => {
            let hint = format_plan_hint(&plan);
            RoutingDecision::LlmWithHints {
                hint_text: hint,
                reason: format!(
                    "Medium confidence ({:.2}) — LLM with tool hints",
                    plan.confidence
                ),
                plan,
            }
        }
        _ => RoutingDecision::FullLlm {
            reason: format!(
                "Low confidence ({:.2}) or no plan generated",
                classified.confidence
            ),
        },
    }
}

/// Format a plan as a hint for the LLM system prompt.
fn format_plan_hint(plan: &Plan) -> String {
    let tools = plan.intent.primary_tools();
    if tools.is_empty() {
        return String::new();
    }

    format!(
        "[Router hint: query likely needs {:?}. Suggested tools: {}]",
        plan.intent,
        tools.join(", ")
    )
}

/// Resolve slot placeholders in a template.
fn resolve_template(template: &serde_json::Value, slots: &Slots) -> serde_json::Value {
    let s = serde_json::to_string(template).unwrap_or_default();
    let resolved = s
        .replace("{query}", &slots.raw_query)
        .replace("{email}", slots.emails.first().map(|s| s.as_str()).unwrap_or(""))
        .replace("{url}", slots.urls.first().map(|s| s.as_str()).unwrap_or(""))
        .replace("{path}", slots.paths.first().map(|s| s.as_str()).unwrap_or(""))
        .replace("{topic}", slots.topic.as_deref().unwrap_or(&slots.raw_query));

    serde_json::from_str(&resolved).unwrap_or(template.clone())
}

// ── Trace Learning Flywheel ─────────────────────────────────────────

/// Record a successful tool execution trace for future motif learning.
pub fn record_trace(
    conn: &Connection,
    query: &str,
    tool_calls: &[String],
    success: bool,
    duration_ms: u64,
) {
    if tool_calls.is_empty() || !success {
        return;
    }

    let classified = offline_nlp::classify_intent(query);

    // Only record traces for recognizable intents (not Unknown)
    if classified.intent == Intent::Unknown {
        return;
    }

    let keywords: Vec<String> = classified.matched_keywords.clone();
    let steps: Vec<MotifStep> = tool_calls
        .iter()
        .map(|name| MotifStep {
            tool_name: name.clone(),
            argument_template: serde_json::json!({}),
        })
        .collect();

    MotifMemory::record(
        conn,
        classified.intent.as_str(),
        &keywords,
        &steps,
        duration_ms as f64,
        success,
    );
}

/// Ensure the trace learning tables exist.
pub fn ensure_tables(conn: &Connection) {
    MotifMemory::ensure_table(conn);

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS nlp_routing_log (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            query_text  TEXT NOT NULL,
            decision    TEXT NOT NULL,
            intent      TEXT NOT NULL,
            confidence  REAL NOT NULL,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            success     INTEGER NOT NULL DEFAULT 1,
            created_at  REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_nlp_routing_time ON nlp_routing_log(created_at);",
    )
    .expect("failed to create routing log table");
}

/// Log a routing decision for analysis.
pub fn log_routing(
    conn: &Connection,
    query: &str,
    decision: &RoutingDecision,
    intent: Intent,
    confidence: f64,
) {
    let now = now_ts();
    let _ = conn.execute(
        "INSERT INTO nlp_routing_log (query_text, decision, intent, confidence, created_at)
         VALUES (?1,?2,?3,?4,?5)",
        params![
            query.chars().take(200).collect::<String>(),
            decision.type_tag(),
            intent.as_str(),
            confidence,
            now,
        ],
    );
}

/// Get routing statistics (for dashboard).
#[derive(Debug, Clone)]
pub struct RoutingStats {
    pub total_queries: u64,
    pub offline_count: u64,
    pub llm_with_hints_count: u64,
    pub full_llm_count: u64,
    pub offline_pct: f64,
}

pub fn routing_stats(conn: &Connection, since_hours: f64) -> RoutingStats {
    let since = now_ts() - since_hours * 3600.0;

    let total: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM nlp_routing_log WHERE created_at >= ?1",
            params![since],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let offline: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM nlp_routing_log WHERE created_at >= ?1 AND decision = 'offline'",
            params![since],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let hints: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM nlp_routing_log WHERE created_at >= ?1 AND decision = 'llm_with_hints'",
            params![since],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let offline_pct = if total > 0 {
        offline as f64 / total as f64 * 100.0
    } else {
        0.0
    };

    RoutingStats {
        total_queries: total,
        offline_count: offline,
        llm_with_hints_count: hints,
        full_llm_count: total - offline - hints,
        offline_pct,
    }
}

/// One intent's share of the routing log.
#[derive(Debug, Clone)]
pub struct IntentCount {
    pub intent: String,
    pub count: u64,
    pub offline: u64,
}

/// The routing log read whole: headline numbers, per-intent breakdown, quality.
#[derive(Debug, Clone)]
pub struct RoutingReport {
    pub stats: RoutingStats,
    /// Highest count first.
    pub by_intent: Vec<IntentCount>,
    pub avg_duration_ms: f64,
    pub failures: u64,
    pub window_hours: f64,
}

pub fn routing_report(conn: &Connection, since_hours: f64) -> RoutingReport {
    let since = now_ts() - since_hours * 3600.0;

    RoutingReport {
        stats: routing_stats(conn, since_hours),
        by_intent: intent_counts(conn, since),
        avg_duration_ms: conn
            .query_row(
                "SELECT AVG(duration_ms) FROM nlp_routing_log WHERE created_at >= ?1",
                params![since],
                |r| r.get::<_, Option<f64>>(0),
            )
            .unwrap_or(None)
            .unwrap_or(0.0),
        failures: conn
            .query_row(
                "SELECT COUNT(*) FROM nlp_routing_log WHERE created_at >= ?1 AND success = 0",
                params![since],
                |r| r.get(0),
            )
            .unwrap_or(0),
        window_hours: since_hours,
    }
}

/// Per-intent counts over the window, highest count first.
fn intent_counts(conn: &Connection, since: f64) -> Vec<IntentCount> {
    let mut stmt = match conn.prepare(
        "SELECT intent,
                COUNT(*),
                COALESCE(SUM(CASE WHEN decision = 'offline' THEN 1 ELSE 0 END), 0)
         FROM nlp_routing_log
         WHERE created_at >= ?1
         GROUP BY intent
         ORDER BY COUNT(*) DESC, intent ASC",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };

    let rows = match stmt.query_map(params![since], |r| {
        Ok(IntentCount {
            intent: r.get(0)?,
            count: r.get(1)?,
            offline: r.get(2)?,
        })
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };

    rows.flatten().collect()
}

/// Plain-text routing report — the shape the routing log was always meant to be read in.
pub fn routing_report_text(conn: &Connection, since_hours: f64) -> String {
    let report = routing_report(conn, since_hours);

    let window = if report.window_hours.fract() == 0.0 {
        format!("{:.0}h", report.window_hours)
    } else {
        format!("{:.1}h", report.window_hours)
    };

    let mut out = format!(
        "Routing, last {}: {} queries, {:.0}% offline, avg {:.0}ms, {} failures\n",
        window,
        report.stats.total_queries,
        report.stats.offline_pct,
        report.avg_duration_ms,
        report.failures,
    );

    if report.by_intent.is_empty() {
        out.push_str("  no routing decisions recorded\n");
        return out;
    }

    for row in &report.by_intent {
        out.push_str(&format!(
            "  {:<16} {:>3}  {} offline\n",
            row.intent, row.count, row.offline
        ));
    }

    out
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
        ensure_tables(&conn);
        conn
    }

    #[test]
    fn route_unknown_to_full_llm() {
        let conn = setup();
        let decision = route("what's the meaning of life?", &conn);
        assert!(matches!(decision, RoutingDecision::FullLlm { .. }));
    }

    #[test]
    fn route_high_confidence_offline() {
        let conn = setup();
        let decision = route("check my email inbox", &conn);
        // Email check has multiple keyword matches → high confidence
        match &decision {
            RoutingDecision::Offline { plan, .. } => {
                assert_eq!(plan.intent, Intent::CheckEmail);
            }
            RoutingDecision::LlmWithHints { plan, .. } => {
                assert_eq!(plan.intent, Intent::CheckEmail);
            }
            RoutingDecision::FullLlm { reason } => {
                panic!("Expected Offline/Hints, got FullLlm: {}", reason);
            }
        }
    }

    #[test]
    fn trace_learning_builds_motifs() {
        let conn = setup();

        // Record 5 traces of the same pattern
        for _ in 0..5 {
            record_trace(&conn, "check my email", &["email_check".into()], true, 150);
        }

        // Now motif should exist
        let motifs = MotifMemory::all_active(&conn, 3);
        assert!(!motifs.is_empty());
        assert_eq!(motifs[0].observation_count, 5);
    }

    #[test]
    fn routing_stats_tracking() {
        let conn = setup();

        log_routing(
            &conn,
            "check email",
            &RoutingDecision::Offline {
                plan: Plan {
                    intent: Intent::CheckEmail,
                    confidence: 0.9,
                    root: PlanNode::DirectResponse("test".into()),
                    slots: offline_nlp::extract_slots(""),
                    source: PlanSource::Classified,
                },
                reason: "test".into(),
            },
            Intent::CheckEmail,
            0.9,
        );
        log_routing(
            &conn,
            "tell me a joke",
            &RoutingDecision::FullLlm { reason: "unknown".into() },
            Intent::Unknown,
            0.1,
        );

        let stats = routing_stats(&conn, 1.0);
        assert_eq!(stats.total_queries, 2);
        assert_eq!(stats.offline_count, 1);
        assert_eq!(stats.full_llm_count, 1);
        assert!((stats.offline_pct - 50.0).abs() < 0.1);
    }

    /// One offline-routed check_email through the real writer.
    fn log_offline_email(conn: &Connection) {
        log_routing(
            conn,
            "check my email",
            &RoutingDecision::Offline {
                plan: Plan {
                    intent: Intent::CheckEmail,
                    confidence: 0.9,
                    root: PlanNode::DirectResponse("test".into()),
                    slots: offline_nlp::extract_slots(""),
                    source: PlanSource::Classified,
                },
                reason: "test".into(),
            },
            Intent::CheckEmail,
            0.9,
        );
    }

    #[test]
    fn routing_report_groups_by_intent() {
        let conn = setup();
        log_offline_email(&conn);
        log_offline_email(&conn);
        log_routing(
            &conn,
            "what is the meaning of life",
            &RoutingDecision::FullLlm { reason: "unknown".into() },
            Intent::Unknown,
            0.1,
        );

        let report = routing_report(&conn, 1.0);
        assert_eq!(report.stats.total_queries, 3);
        assert_eq!(report.by_intent.len(), 2);
        assert_eq!(report.by_intent[0].intent, "check_email");
        assert_eq!(report.by_intent[0].count, 2);
        assert_eq!(report.by_intent[0].offline, 2);
        assert_eq!(report.by_intent[1].intent, "unknown");
        assert_eq!(report.by_intent[1].count, 1);
        assert_eq!(report.window_hours, 1.0);
    }

    #[test]
    fn routing_report_text_is_readable() {
        let conn = setup();
        log_offline_email(&conn);
        log_offline_email(&conn);
        log_routing(
            &conn,
            "what is the meaning of life",
            &RoutingDecision::FullLlm { reason: "unknown".into() },
            Intent::Unknown,
            0.1,
        );

        let text = routing_report_text(&conn, 24.0);
        assert!(text.contains("3 queries"), "{text}");
        assert!(text.contains("check_email"), "{text}");
        assert!(text.contains("2 offline"), "{text}");
        assert!(!text.contains('\u{1b}'));
    }

    #[test]
    fn routing_report_empty_window() {
        let conn = setup();

        let report = routing_report(&conn, 24.0);
        assert_eq!(report.stats.total_queries, 0);
        assert!(report.by_intent.is_empty());
        assert_eq!(report.failures, 0);
        assert!(!report.avg_duration_ms.is_nan());
        assert!(!report.avg_duration_ms.is_infinite());

        let text = routing_report_text(&conn, 24.0);
        assert!(text.contains("0 queries"), "{text}");
        assert!(text.contains("no routing decisions recorded"), "{text}");
        assert!(!text.contains('\u{1b}'));
    }
}
