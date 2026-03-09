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
}
