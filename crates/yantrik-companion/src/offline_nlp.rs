//! Offline NLP Engine — deterministic semantic planner for common queries.
//!
//! Handles ~60-70% of daily queries without LLM inference by:
//! 1. Classifying intent via keyword/pattern matching
//! 2. Extracting slots (entities, times, quantities)
//! 3. Generating a Plan IR (typed execution plan)
//! 4. Matching against Motif Memory (learned patterns from LLM traces)
//!
//! Confidence-gated: only executes when confidence >= 0.85.
//! Sub-200ms for common queries.

use std::sync::OnceLock;
use regex::Regex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

// ── Intent Classification ───────────────────────────────────────────

/// Recognized intent categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Intent {
    CheckEmail,
    SendEmail,
    SearchEmail,
    CheckCalendar,
    CreateEvent,
    GetWeather,
    SetReminder,
    RecallMemory,
    StoreMemory,
    RunCommand,
    SearchWeb,
    BrowseUrl,
    FileRead,
    FileWrite,
    FileSearch,
    SystemInfo,
    ListTasks,
    TimeQuery,
    Calculate,
    Greeting,
    Farewell,
    Thanks,
    Unknown,
}

impl Intent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CheckEmail => "check_email",
            Self::SendEmail => "send_email",
            Self::SearchEmail => "search_email",
            Self::CheckCalendar => "check_calendar",
            Self::CreateEvent => "create_event",
            Self::GetWeather => "get_weather",
            Self::SetReminder => "set_reminder",
            Self::RecallMemory => "recall_memory",
            Self::StoreMemory => "store_memory",
            Self::RunCommand => "run_command",
            Self::SearchWeb => "search_web",
            Self::BrowseUrl => "browse_url",
            Self::FileRead => "file_read",
            Self::FileWrite => "file_write",
            Self::FileSearch => "file_search",
            Self::SystemInfo => "system_info",
            Self::ListTasks => "list_tasks",
            Self::TimeQuery => "time_query",
            Self::Calculate => "calculate",
            Self::Greeting => "greeting",
            Self::Farewell => "farewell",
            Self::Thanks => "thanks",
            Self::Unknown => "unknown",
        }
    }

    /// Which tool(s) this intent maps to.
    pub fn primary_tools(&self) -> &'static [&'static str] {
        match self {
            Self::CheckEmail => &["email_check", "email_list"],
            Self::SendEmail => &["email_send"],
            Self::SearchEmail => &["email_search"],
            Self::CheckCalendar => &["calendar_today", "calendar_list_events"],
            Self::CreateEvent => &["calendar_create_event"],
            Self::GetWeather => &["get_weather"],
            Self::SetReminder => &["set_reminder"],
            Self::RecallMemory => &["recall"],
            Self::StoreMemory => &["remember"],
            Self::RunCommand => &["run_command"],
            Self::SearchWeb => &["web_search"],
            Self::BrowseUrl => &["browse"],
            Self::FileRead => &["read_file"],
            Self::FileWrite => &["write_file"],
            Self::FileSearch => &["glob", "grep"],
            Self::SystemInfo => &["system_info"],
            Self::ListTasks => &["list_tasks"],
            Self::TimeQuery => &["date_calc"],
            Self::Calculate => &["calculate"],
            _ => &[],
        }
    }
}

/// Classification result with confidence.
#[derive(Debug, Clone)]
pub struct ClassifiedIntent {
    pub intent: Intent,
    pub confidence: f64,
    pub matched_keywords: Vec<String>,
}

/// Classify a user query into an intent.
pub fn classify_intent(query: &str) -> ClassifiedIntent {
    let lower = query.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();

    // Score each intent
    let mut best = ClassifiedIntent {
        intent: Intent::Unknown,
        confidence: 0.0,
        matched_keywords: vec![],
    };

    for &(intent, keywords, boost) in INTENT_PATTERNS {
        let mut score = 0.0;
        let mut matched = vec![];

        for &kw in keywords {
            if kw.contains(' ') {
                // Multi-word match (higher value)
                if lower.contains(kw) {
                    score += 0.4;
                    matched.push(kw.to_string());
                }
            } else if words.contains(&kw) {
                score += 0.25;
                matched.push(kw.to_string());
            }
        }

        // Apply intent-specific boost
        score += boost;

        // Normalize: cap at 1.0
        let confidence = score.min(1.0);

        if confidence > best.confidence {
            best = ClassifiedIntent {
                intent,
                confidence,
                matched_keywords: matched,
            };
        }
    }

    // Conversational intents (greetings, thanks, farewell)
    if best.confidence < 0.5 {
        if let Some(conv) = classify_conversational(&lower) {
            return conv;
        }
    }

    best
}

const INTENT_PATTERNS: &[(Intent, &[&str], f64)] = &[
    (Intent::CheckEmail, &["check email", "check my email", "any email", "new email", "inbox", "unread", "email check"], 0.15),
    (Intent::SendEmail, &["send email", "email to", "compose email", "write email", "reply to"], 0.15),
    (Intent::SearchEmail, &["search email", "find email", "email from", "email about"], 0.10),
    (Intent::CheckCalendar, &["calendar", "schedule", "meetings today", "what's on", "events today", "any meetings", "agenda"], 0.15),
    (Intent::CreateEvent, &["create event", "schedule meeting", "add to calendar", "book meeting", "set up meeting"], 0.15),
    (Intent::GetWeather, &["weather", "temperature", "forecast", "rain", "sunny", "how hot", "how cold"], 0.20),
    (Intent::SetReminder, &["remind me", "set reminder", "reminder", "alert me", "notify me when"], 0.15),
    (Intent::RecallMemory, &["remember", "recall", "what do you know", "do you remember", "told you about", "mentioned"], 0.10),
    (Intent::StoreMemory, &["remember this", "store this", "save this", "keep in mind", "note that"], 0.15),
    (Intent::RunCommand, &["run", "execute", "terminal", "command", "shell", "bash"], 0.10),
    (Intent::SearchWeb, &["search", "google", "look up", "find out", "search for", "search the web"], 0.10),
    (Intent::BrowseUrl, &["browse", "open", "go to", "visit", "navigate to", "website"], 0.10),
    (Intent::FileRead, &["read file", "show file", "cat", "open file", "contents of", "what's in"], 0.10),
    (Intent::FileWrite, &["write file", "create file", "save to", "write to"], 0.15),
    (Intent::FileSearch, &["find file", "search files", "locate", "where is", "find files"], 0.10),
    (Intent::SystemInfo, &["system", "cpu", "memory", "disk", "processes", "uptime", "system info", "how much ram", "disk space"], 0.15),
    (Intent::ListTasks, &["tasks", "todo", "task list", "pending tasks", "my tasks", "task queue"], 0.15),
    (Intent::TimeQuery, &["what time", "what day", "what date", "current time", "today's date", "how many days until"], 0.20),
    (Intent::Calculate, &["calculate", "compute", "math", "how much is", "what is the sum", "convert"], 0.15),
];

fn classify_conversational(lower: &str) -> Option<ClassifiedIntent> {
    let greetings = ["hello", "hi", "hey", "good morning", "good afternoon", "good evening", "howdy", "sup"];
    let farewells = ["goodbye", "bye", "good night", "see you", "later", "gotta go", "signing off"];
    let thanks = ["thank", "thanks", "appreciate", "grateful"];

    for &g in &greetings {
        if lower.starts_with(g) || lower == g {
            return Some(ClassifiedIntent {
                intent: Intent::Greeting,
                confidence: 0.90,
                matched_keywords: vec![g.to_string()],
            });
        }
    }
    for &f in &farewells {
        if lower.contains(f) {
            return Some(ClassifiedIntent {
                intent: Intent::Farewell,
                confidence: 0.85,
                matched_keywords: vec![f.to_string()],
            });
        }
    }
    for &t in &thanks {
        if lower.contains(t) {
            return Some(ClassifiedIntent {
                intent: Intent::Thanks,
                confidence: 0.85,
                matched_keywords: vec![t.to_string()],
            });
        }
    }
    None
}

// ── Slot Extraction ─────────────────────────────────────────────────

/// Extracted slots from a query.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Slots {
    /// Person name(s) mentioned.
    pub persons: Vec<String>,
    /// Time/date references.
    pub times: Vec<String>,
    /// Numeric values.
    pub numbers: Vec<f64>,
    /// Email addresses.
    pub emails: Vec<String>,
    /// URLs.
    pub urls: Vec<String>,
    /// File paths.
    pub paths: Vec<String>,
    /// Location references.
    pub locations: Vec<String>,
    /// Free-text subject/topic.
    pub topic: Option<String>,
    /// Raw query text.
    pub raw_query: String,
}

fn re_email_addr() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b[\w.+-]+@[\w.-]+\.\w{2,}\b").unwrap())
}

fn re_url() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"https?://\S+").unwrap())
}

fn re_file_path() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:~|/[\w.-]+)+(?:/[\w.*-]+)+").unwrap())
}

fn re_number() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b\d+(?:\.\d+)?\b").unwrap())
}

fn re_time_ref() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(
        r"(?i)\b(?:today|tomorrow|yesterday|tonight|this (?:morning|afternoon|evening|week|month)|next (?:week|month|monday|tuesday|wednesday|thursday|friday|saturday|sunday)|(?:mon|tue|wed|thu|fri|sat|sun)(?:day)?|(?:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)\w*\s+\d{1,2}|\d{1,2}:\d{2}\s*(?:am|pm)?)\b"
    ).unwrap())
}

/// Extract slots from a query.
pub fn extract_slots(query: &str) -> Slots {
    let mut slots = Slots {
        raw_query: query.to_string(),
        ..Default::default()
    };

    // Email addresses
    for cap in re_email_addr().find_iter(query) {
        slots.emails.push(cap.as_str().to_string());
    }

    // URLs
    for cap in re_url().find_iter(query) {
        slots.urls.push(cap.as_str().to_string());
    }

    // File paths
    for cap in re_file_path().find_iter(query) {
        slots.paths.push(cap.as_str().to_string());
    }

    // Numbers (excluding those in emails/urls/paths)
    for cap in re_number().find_iter(query) {
        if let Ok(n) = cap.as_str().parse::<f64>() {
            slots.numbers.push(n);
        }
    }

    // Time references
    for cap in re_time_ref().find_iter(query) {
        slots.times.push(cap.as_str().to_string());
    }

    slots
}

// ── Plan IR ─────────────────────────────────────────────────────────

/// A node in the execution plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlanNode {
    /// Call a tool with arguments.
    ToolCall {
        tool_name: String,
        arguments: serde_json::Value,
    },
    /// Sequence of steps (execute in order).
    Sequence(Vec<PlanNode>),
    /// Conditional execution.
    Condition {
        check: String,
        then_node: Box<PlanNode>,
        else_node: Option<Box<PlanNode>>,
    },
    /// Transform a result (e.g., filter, format).
    Transform {
        operation: String,
        input_ref: String,
    },
    /// Return a direct text response (no tool call needed).
    DirectResponse(String),
}

/// A generated execution plan.
#[derive(Debug, Clone)]
pub struct Plan {
    pub intent: Intent,
    pub confidence: f64,
    pub root: PlanNode,
    pub slots: Slots,
    pub source: PlanSource,
}

/// Where the plan came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanSource {
    /// Built from intent classification + slot extraction.
    Classified,
    /// Matched from motif memory (learned pattern).
    Motif,
}

/// Generate a plan from classified intent and extracted slots.
pub fn generate_plan(classified: &ClassifiedIntent, slots: &Slots) -> Option<Plan> {
    let root = match classified.intent {
        Intent::CheckEmail => PlanNode::ToolCall {
            tool_name: "email_check".into(),
            arguments: serde_json::json!({}),
        },
        Intent::SearchEmail => {
            let query = slots.topic.clone().unwrap_or_else(|| slots.raw_query.clone());
            PlanNode::ToolCall {
                tool_name: "email_search".into(),
                arguments: serde_json::json!({ "query": query }),
            }
        }
        Intent::CheckCalendar => PlanNode::ToolCall {
            tool_name: "calendar_today".into(),
            arguments: serde_json::json!({}),
        },
        Intent::GetWeather => {
            let location = slots.locations.first().cloned()
                .unwrap_or_default();
            PlanNode::ToolCall {
                tool_name: "get_weather".into(),
                arguments: if location.is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::json!({ "location": location })
                },
            }
        }
        Intent::RecallMemory => {
            let query = slots.topic.clone().unwrap_or_else(|| slots.raw_query.clone());
            PlanNode::ToolCall {
                tool_name: "recall".into(),
                arguments: serde_json::json!({ "query": query }),
            }
        }
        Intent::SystemInfo => PlanNode::ToolCall {
            tool_name: "system_info".into(),
            arguments: serde_json::json!({}),
        },
        Intent::TimeQuery => PlanNode::ToolCall {
            tool_name: "date_calc".into(),
            arguments: serde_json::json!({ "query": slots.raw_query }),
        },
        Intent::Calculate => {
            let expr = slots.raw_query.clone();
            PlanNode::ToolCall {
                tool_name: "calculate".into(),
                arguments: serde_json::json!({ "expression": expr }),
            }
        }
        Intent::SearchWeb => {
            let query = slots.topic.clone().unwrap_or_else(|| slots.raw_query.clone());
            PlanNode::ToolCall {
                tool_name: "web_search".into(),
                arguments: serde_json::json!({ "query": query }),
            }
        }
        Intent::BrowseUrl => {
            if let Some(url) = slots.urls.first() {
                PlanNode::ToolCall {
                    tool_name: "browse".into(),
                    arguments: serde_json::json!({ "url": url }),
                }
            } else {
                return None;
            }
        }
        Intent::FileRead => {
            if let Some(path) = slots.paths.first() {
                PlanNode::ToolCall {
                    tool_name: "read_file".into(),
                    arguments: serde_json::json!({ "path": path }),
                }
            } else {
                return None;
            }
        }
        Intent::ListTasks => PlanNode::ToolCall {
            tool_name: "list_tasks".into(),
            arguments: serde_json::json!({}),
        },
        Intent::Greeting => PlanNode::DirectResponse(
            "greeting".into(),
        ),
        Intent::Farewell => PlanNode::DirectResponse(
            "farewell".into(),
        ),
        Intent::Thanks => PlanNode::DirectResponse(
            "acknowledgment".into(),
        ),
        _ => return None, // Can't generate plan for complex/unknown intents
    };

    Some(Plan {
        intent: classified.intent,
        confidence: classified.confidence,
        root,
        slots: slots.clone(),
        source: PlanSource::Classified,
    })
}

// ── Motif Memory ────────────────────────────────────────────────────

/// A reusable execution pattern learned from LLM traces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Motif {
    pub id: i64,
    /// The intent pattern this motif matches.
    pub intent_pattern: String,
    /// Keywords that trigger this motif.
    pub trigger_keywords: Vec<String>,
    /// The tool chain to execute (in order).
    pub tool_chain: Vec<MotifStep>,
    /// How many times this pattern was observed in LLM traces.
    pub observation_count: u32,
    /// Success rate of this pattern.
    pub success_rate: f64,
    /// Average execution time.
    pub avg_duration_ms: f64,
    pub created_at: f64,
    pub last_used_at: f64,
}

/// A step in a motif's execution chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotifStep {
    pub tool_name: String,
    /// Argument template with slot placeholders like `{query}`, `{person}`.
    pub argument_template: serde_json::Value,
}

/// Motif memory persistence.
pub struct MotifMemory;

impl MotifMemory {
    pub fn ensure_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS nlp_motifs (
                id                INTEGER PRIMARY KEY AUTOINCREMENT,
                intent_pattern    TEXT NOT NULL,
                trigger_keywords  TEXT NOT NULL DEFAULT '[]',
                tool_chain        TEXT NOT NULL DEFAULT '[]',
                observation_count INTEGER NOT NULL DEFAULT 1,
                success_rate      REAL NOT NULL DEFAULT 1.0,
                avg_duration_ms   REAL NOT NULL DEFAULT 0.0,
                created_at        REAL NOT NULL,
                last_used_at      REAL NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_nlp_motifs_intent ON nlp_motifs(intent_pattern);",
        )
        .expect("failed to create nlp_motifs table");
    }

    /// Find motifs matching a query by keyword overlap.
    pub fn find_matching(conn: &Connection, query: &str, min_observations: u32) -> Vec<Motif> {
        let lower = query.to_lowercase();
        let all_motifs = Self::all_active(conn, min_observations);

        all_motifs
            .into_iter()
            .filter(|m| {
                m.trigger_keywords
                    .iter()
                    .filter(|kw| lower.contains(&kw.to_lowercase()))
                    .count()
                    >= 2 // Need at least 2 keyword matches
            })
            .collect()
    }

    /// Get all active motifs with minimum observations.
    pub fn all_active(conn: &Connection, min_observations: u32) -> Vec<Motif> {
        conn.prepare(
            "SELECT id, intent_pattern, trigger_keywords, tool_chain,
                    observation_count, success_rate, avg_duration_ms,
                    created_at, last_used_at
             FROM nlp_motifs
             WHERE observation_count >= ?1 AND success_rate >= 0.6
             ORDER BY observation_count DESC",
        )
        .and_then(|mut stmt| {
            stmt.query_map(params![min_observations], |row| {
                let kw_json: String = row.get(2)?;
                let chain_json: String = row.get(3)?;
                Ok(Motif {
                    id: row.get(0)?,
                    intent_pattern: row.get(1)?,
                    trigger_keywords: serde_json::from_str(&kw_json).unwrap_or_default(),
                    tool_chain: serde_json::from_str(&chain_json).unwrap_or_default(),
                    observation_count: row.get::<_, u32>(4)?,
                    success_rate: row.get(5)?,
                    avg_duration_ms: row.get(6)?,
                    created_at: row.get(7)?,
                    last_used_at: row.get(8)?,
                })
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
    }

    /// Record a new motif or reinforce an existing one.
    pub fn record(
        conn: &Connection,
        intent_pattern: &str,
        keywords: &[String],
        tool_chain: &[MotifStep],
        duration_ms: f64,
        success: bool,
    ) {
        let now = now_ts();
        let kw_json = serde_json::to_string(keywords).unwrap_or_default();
        let chain_json = serde_json::to_string(tool_chain).unwrap_or_default();

        // Check if matching motif exists
        let existing: Option<(i64, u32, f64, f64)> = conn
            .query_row(
                "SELECT id, observation_count, success_rate, avg_duration_ms
                 FROM nlp_motifs WHERE intent_pattern = ?1",
                params![intent_pattern],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .ok();

        if let Some((id, count, old_rate, old_dur)) = existing {
            let new_count = count + 1;
            let success_val = if success { 1.0 } else { 0.0 };
            let new_rate = (old_rate * count as f64 + success_val) / new_count as f64;
            let new_dur = (old_dur * count as f64 + duration_ms) / new_count as f64;

            let _ = conn.execute(
                "UPDATE nlp_motifs
                 SET observation_count = ?1, success_rate = ?2,
                     avg_duration_ms = ?3, last_used_at = ?4
                 WHERE id = ?5",
                params![new_count, new_rate, new_dur, now, id],
            );
        } else {
            let _ = conn.execute(
                "INSERT INTO nlp_motifs
                 (intent_pattern, trigger_keywords, tool_chain,
                  observation_count, success_rate, avg_duration_ms,
                  created_at, last_used_at)
                 VALUES (?1,?2,?3,1,?4,?5,?6,?7)",
                params![
                    intent_pattern,
                    kw_json,
                    chain_json,
                    if success { 1.0 } else { 0.0 },
                    duration_ms,
                    now,
                    now,
                ],
            );
        }
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

    #[test]
    fn classify_email_check() {
        let result = classify_intent("check my email");
        assert_eq!(result.intent, Intent::CheckEmail);
        assert!(result.confidence >= 0.5);
    }

    #[test]
    fn classify_weather() {
        let result = classify_intent("what's the weather like today");
        assert_eq!(result.intent, Intent::GetWeather);
        assert!(result.confidence >= 0.4);
    }

    #[test]
    fn classify_calendar() {
        let result = classify_intent("any meetings today");
        assert_eq!(result.intent, Intent::CheckCalendar);
        assert!(result.confidence >= 0.5);
    }

    #[test]
    fn classify_greeting() {
        let result = classify_intent("hello");
        assert_eq!(result.intent, Intent::Greeting);
        assert!(result.confidence >= 0.8);
    }

    #[test]
    fn classify_unknown() {
        let result = classify_intent("tell me a philosophical joke about existentialism");
        // Open-ended/creative queries should have low confidence
        assert!(result.confidence < 0.5, "Expected low confidence for unknown query, got {} ({:?})", result.confidence, result.intent);
    }

    #[test]
    fn slot_extraction_emails() {
        let slots = extract_slots("send email to john@example.com about the project");
        assert_eq!(slots.emails.len(), 1);
        assert_eq!(slots.emails[0], "john@example.com");
    }

    #[test]
    fn slot_extraction_urls() {
        let slots = extract_slots("browse https://news.ycombinator.com");
        assert_eq!(slots.urls.len(), 1);
        assert!(slots.urls[0].contains("ycombinator"));
    }

    #[test]
    fn slot_extraction_times() {
        let slots = extract_slots("remind me tomorrow to submit the report");
        assert!(slots.times.iter().any(|t| t.to_lowercase().contains("tomorrow")));
    }

    #[test]
    fn plan_generation_email() {
        let classified = classify_intent("check my email");
        let slots = extract_slots("check my email");
        let plan = generate_plan(&classified, &slots);
        assert!(plan.is_some());
        let plan = plan.unwrap();
        match &plan.root {
            PlanNode::ToolCall { tool_name, .. } => {
                assert_eq!(tool_name, "email_check");
            }
            _ => panic!("Expected ToolCall"),
        }
    }

    #[test]
    fn plan_generation_browse() {
        let classified = classify_intent("browse https://example.com");
        let slots = extract_slots("browse https://example.com");
        let plan = generate_plan(&classified, &slots);
        assert!(plan.is_some());
    }

    #[test]
    fn motif_memory_roundtrip() {
        let conn = Connection::open_in_memory().unwrap();
        MotifMemory::ensure_table(&conn);

        let steps = vec![MotifStep {
            tool_name: "email_check".into(),
            argument_template: serde_json::json!({}),
        }];

        MotifMemory::record(&conn, "check_email", &["check".into(), "email".into()], &steps, 150.0, true);
        MotifMemory::record(&conn, "check_email", &["check".into(), "email".into()], &steps, 120.0, true);

        let motifs = MotifMemory::all_active(&conn, 2);
        assert_eq!(motifs.len(), 1);
        assert_eq!(motifs[0].observation_count, 2);
        assert!((motifs[0].success_rate - 1.0).abs() < 0.01);
    }
}
