//! Structured Decision Protocol — typed output format for model-adaptive tool calling.
//!
//! When the ModelCapabilityProfile specifies `ToolCallMode::StructuredJSON`, the LLM
//! is prompted to emit a structured decision JSON instead of raw function calls.
//! This module provides:
//!
//! 1. **Decision schema** — the typed output the LLM must produce
//! 2. **Parser** — extracts structured decisions from LLM text output
//! 3. **Validator** — checks schema correctness, entity resolution, required fields
//! 4. **Repair prompt** — generates a focused retry prompt when validation fails
//! 5. **Converter** — transforms validated decisions into standard ToolCall objects
//!
//! ## Architecture
//!
//! ```text
//! LLM Output (raw text or JSON)
//!     │
//!     ▼
//! StructuredDecisionParser::parse()
//!     │
//!     ├── Ok(StructuredDecision) ──► Validator::validate()
//!     │                                  │
//!     │                                  ├── Valid ──► to_tool_calls()
//!     │                                  │
//!     │                                  └── Invalid ──► RepairPrompt::build()
//!     │                                                      │
//!     │                                                      └── Retry LLM
//!     │
//!     └── Err(ParseError) ──► FallbackParser (try native tool calls)
//! ```
//!
//! ## Decision JSON Schema
//!
//! ```json
//! {
//!   "decision": "use_tool | answer_directly | ask_clarification | refuse",
//!   "family": "COMMUNICATE",
//!   "tool": "email_send",
//!   "args": { "to": "alice@example.com", "subject": "Lunch" },
//!   "confidence": 0.88,
//!   "grounding": ["calendar:evt_123", "contact:alice"],
//!   "missing_slots": ["time"],
//!   "needs_confirmation": true,
//!   "reasoning": "User asked to email Alice about lunch"
//! }
//! ```

use serde::{Deserialize, Serialize};
use yantrik_ml::ToolCall;

// ── Decision Types ────────────────────────────────────────────────────

/// The decision type the model must choose from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionType {
    /// Model should call a tool to fulfill the request.
    UseTool,
    /// Model can answer directly from context without tools.
    AnswerDirectly,
    /// Model needs more information from the user.
    AskClarification,
    /// Model should decline the request (out of scope, unsafe, etc.).
    Refuse,
    /// Model should escalate to a more capable model or cloud.
    Escalate,
}

impl DecisionType {
    /// Parse from string, case-insensitive, with common aliases.
    pub fn from_str_lenient(s: &str) -> Option<Self> {
        match s.to_lowercase().trim() {
            "use_tool" | "tool" | "call_tool" => Some(Self::UseTool),
            "answer_directly" | "answer" | "direct" => Some(Self::AnswerDirectly),
            "ask_clarification" | "clarify" | "ask" => Some(Self::AskClarification),
            "refuse" | "decline" | "reject" => Some(Self::Refuse),
            "escalate" | "delegate" | "cloud" => Some(Self::Escalate),
            _ => None,
        }
    }
}

impl std::fmt::Display for DecisionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UseTool => write!(f, "use_tool"),
            Self::AnswerDirectly => write!(f, "answer_directly"),
            Self::AskClarification => write!(f, "ask_clarification"),
            Self::Refuse => write!(f, "refuse"),
            Self::Escalate => write!(f, "escalate"),
        }
    }
}

// ── Structured Decision ───────────────────────────────────────────────

/// A structured decision emitted by the LLM in StructuredJSON mode.
///
/// This is the single-pass output that replaces raw function calling
/// for Medium-tier models, providing explicit decision type, confidence,
/// grounding references, and missing slot detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredDecision {
    /// What action the model decided to take.
    pub decision: DecisionType,

    /// Which tool family this maps to (e.g., "COMMUNICATE", "SCHEDULE").
    /// Required when decision == UseTool.
    #[serde(default)]
    pub family: String,

    /// The specific tool to call (e.g., "email_send").
    /// Required when decision == UseTool.
    #[serde(default)]
    pub tool: String,

    /// Tool arguments as a JSON object.
    /// Required when decision == UseTool.
    #[serde(default = "default_args")]
    pub args: serde_json::Value,

    /// Model's self-assessed confidence (0.0–1.0).
    /// Used by the confidence policy to decide whether to execute or ask.
    #[serde(default = "default_confidence")]
    pub confidence: f64,

    /// Grounding references — sources backing this decision.
    /// Format: "type:id" (e.g., "calendar:evt_123", "memory:m_456", "pwg:node:fuel").
    #[serde(default)]
    pub grounding: Vec<String>,

    /// Slots that are missing or ambiguous — triggers clarification.
    #[serde(default)]
    pub missing_slots: Vec<String>,

    /// Whether the action requires explicit user confirmation before execution.
    #[serde(default)]
    pub needs_confirmation: bool,

    /// Brief reasoning for audit trail and repair prompts.
    #[serde(default)]
    pub reasoning: String,

    /// Direct answer text (when decision == AnswerDirectly or AskClarification).
    #[serde(default)]
    pub answer: String,
}

fn default_args() -> serde_json::Value {
    serde_json::json!({})
}

fn default_confidence() -> f64 {
    0.5
}

impl StructuredDecision {
    /// Convert a valid UseTool decision into a standard ToolCall.
    pub fn to_tool_call(&self) -> Option<ToolCall> {
        if self.decision != DecisionType::UseTool || self.tool.is_empty() {
            return None;
        }
        Some(ToolCall {
            name: self.tool.clone(),
            arguments: self.args.clone(),
        })
    }

    /// Whether this decision requires tool execution.
    pub fn needs_tool(&self) -> bool {
        self.decision == DecisionType::UseTool && !self.tool.is_empty()
    }

    /// Whether this decision has a direct text answer.
    pub fn has_answer(&self) -> bool {
        matches!(self.decision, DecisionType::AnswerDirectly | DecisionType::AskClarification | DecisionType::Refuse)
            && !self.answer.is_empty()
    }

    /// Whether the model detected missing information.
    pub fn has_missing_slots(&self) -> bool {
        !self.missing_slots.is_empty()
    }
}

// ── Parse Error ───────────────────────────────────────────────────────

/// Errors that can occur during structured output parsing.
#[derive(Debug, Clone)]
pub enum ParseError {
    /// No JSON block found in model output.
    NoJsonFound,
    /// JSON found but failed to deserialize.
    InvalidJson(String),
    /// JSON deserialized but missing required fields.
    MissingRequiredField(String),
    /// Unknown decision type.
    UnknownDecision(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoJsonFound => write!(f, "No JSON block found in model output"),
            Self::InvalidJson(e) => write!(f, "Invalid JSON: {}", e),
            Self::MissingRequiredField(field) => write!(f, "Missing required field: {}", field),
            Self::UnknownDecision(d) => write!(f, "Unknown decision type: {}", d),
        }
    }
}

impl std::error::Error for ParseError {}

// ── Validation Result ─────────────────────────────────────────────────

/// Result of validating a structured decision.
#[derive(Debug, Clone)]
pub enum ValidationResult {
    /// Decision is valid and ready for execution.
    Valid,
    /// Decision has issues that can be auto-repaired.
    Repairable(Vec<ValidationIssue>),
    /// Decision has critical issues — needs LLM retry.
    Invalid(Vec<ValidationIssue>),
}

/// A specific validation issue found in a structured decision.
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    /// Which field has the issue.
    pub field: String,
    /// What's wrong.
    pub message: String,
    /// Severity: warning (repairable) or error (retry needed).
    pub severity: IssueSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    Warning,
    Error,
}

impl std::fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let level = match self.severity {
            IssueSeverity::Warning => "WARN",
            IssueSeverity::Error => "ERROR",
        };
        write!(f, "[{}] {}: {}", level, self.field, self.message)
    }
}

// ── Parser ────────────────────────────────────────────────────────────

/// Parses structured decision JSON from LLM output text.
///
/// Handles multiple formats:
/// 1. Pure JSON response
/// 2. JSON wrapped in markdown code blocks (```json ... ```)
/// 3. JSON embedded in prose text (finds first `{...}` block)
/// 4. Lenient field parsing (handles typos in decision type)
pub struct StructuredDecisionParser;

impl StructuredDecisionParser {
    /// Parse a structured decision from LLM output text.
    ///
    /// Tries multiple extraction strategies in order:
    /// 1. Direct JSON parse of the full text
    /// 2. Extract from markdown code block
    /// 3. Find first balanced JSON object in text
    pub fn parse(text: &str) -> Result<StructuredDecision, ParseError> {
        let trimmed = text.trim();

        // Strategy 1: Direct parse (model returned pure JSON)
        if let Ok(decision) = Self::try_parse_json(trimmed) {
            return Ok(decision);
        }

        // Strategy 2: Extract from markdown code block
        if let Some(json_str) = Self::extract_code_block(trimmed) {
            if let Ok(decision) = Self::try_parse_json(&json_str) {
                return Ok(decision);
            }
        }

        // Strategy 3: Find first balanced JSON object
        if let Some(json_str) = Self::extract_first_json_object(trimmed) {
            match Self::try_parse_json(&json_str) {
                Ok(decision) => return Ok(decision),
                Err(e) => return Err(e),
            }
        }

        Err(ParseError::NoJsonFound)
    }

    /// Try to parse a JSON string into a StructuredDecision.
    /// Handles lenient parsing of the decision field.
    fn try_parse_json(json_str: &str) -> Result<StructuredDecision, ParseError> {
        // First try strict deserialization
        if let Ok(decision) = serde_json::from_str::<StructuredDecision>(json_str) {
            return Ok(decision);
        }

        // Lenient parsing: deserialize as generic Value, then manually extract
        let value: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| ParseError::InvalidJson(e.to_string()))?;

        let obj = value.as_object()
            .ok_or_else(|| ParseError::InvalidJson("Expected JSON object".into()))?;

        // Parse decision type leniently
        let decision_str = obj.get("decision")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ParseError::MissingRequiredField("decision".into()))?;

        let decision = DecisionType::from_str_lenient(decision_str)
            .ok_or_else(|| ParseError::UnknownDecision(decision_str.into()))?;

        // Extract remaining fields with defaults
        let family = obj.get("family")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let tool = obj.get("tool")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let args = obj.get("args")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));

        let confidence = obj.get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.5);

        let grounding = obj.get("grounding")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let missing_slots = obj.get("missing_slots")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let needs_confirmation = obj.get("needs_confirmation")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let reasoning = obj.get("reasoning")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let answer = obj.get("answer")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(StructuredDecision {
            decision,
            family,
            tool,
            args,
            confidence,
            grounding,
            missing_slots,
            needs_confirmation,
            reasoning,
            answer,
        })
    }

    /// Extract JSON from a markdown code block: ```json ... ``` or ``` ... ```
    fn extract_code_block(text: &str) -> Option<String> {
        // Try ```json first, then plain ```
        for marker in &["```json", "```"] {
            if let Some(start_idx) = text.find(marker) {
                let content_start = start_idx + marker.len();
                if let Some(end_idx) = text[content_start..].find("```") {
                    let json_str = text[content_start..content_start + end_idx].trim();
                    if !json_str.is_empty() {
                        return Some(json_str.to_string());
                    }
                }
            }
        }
        None
    }

    /// Find the first balanced JSON object `{...}` in text.
    /// Handles nested braces correctly.
    fn extract_first_json_object(text: &str) -> Option<String> {
        let bytes = text.as_bytes();
        let mut start = None;
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escape_next = false;

        for (i, &b) in bytes.iter().enumerate() {
            if escape_next {
                escape_next = false;
                continue;
            }
            if b == b'\\' && in_string {
                escape_next = true;
                continue;
            }
            if b == b'"' {
                in_string = !in_string;
                continue;
            }
            if in_string {
                continue;
            }
            if b == b'{' {
                if start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            } else if b == b'}' {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start {
                        return Some(text[s..=i].to_string());
                    }
                }
            }
        }
        None
    }
}

// ── Validator ─────────────────────────────────────────────────────────

/// Validates a structured decision for correctness before execution.
///
/// Checks:
/// 1. Required fields present for the decision type
/// 2. Tool name is non-empty for UseTool decisions
/// 3. Confidence is in valid range
/// 4. Args is an object (not array or scalar)
/// 5. Tool exists in the known tool set (if tool_names provided)
pub struct StructuredDecisionValidator;

impl StructuredDecisionValidator {
    /// Validate a decision against the schema and optional known tool names.
    pub fn validate(
        decision: &StructuredDecision,
        known_tools: Option<&[&str]>,
        confidence_threshold: f64,
    ) -> ValidationResult {
        let mut issues = Vec::new();

        // 1. Decision-specific field requirements
        match decision.decision {
            DecisionType::UseTool => {
                if decision.tool.is_empty() {
                    issues.push(ValidationIssue {
                        field: "tool".into(),
                        message: "Tool name is required for use_tool decisions".into(),
                        severity: IssueSeverity::Error,
                    });
                }
                if !decision.args.is_object() {
                    issues.push(ValidationIssue {
                        field: "args".into(),
                        message: format!("Args must be a JSON object, got: {}", decision.args),
                        severity: IssueSeverity::Error,
                    });
                }
            }
            DecisionType::AnswerDirectly | DecisionType::AskClarification | DecisionType::Refuse => {
                if decision.answer.is_empty() {
                    issues.push(ValidationIssue {
                        field: "answer".into(),
                        message: "Answer text is required for non-tool decisions".into(),
                        severity: IssueSeverity::Warning,
                    });
                }
            }
            DecisionType::Escalate => {} // No specific requirements
        }

        // 2. Tool existence check
        if decision.decision == DecisionType::UseTool && !decision.tool.is_empty() {
            if let Some(tools) = known_tools {
                if !tools.iter().any(|&t| t == decision.tool) {
                    issues.push(ValidationIssue {
                        field: "tool".into(),
                        message: format!("Unknown tool '{}'. Available: {:?}", decision.tool, tools),
                        severity: IssueSeverity::Error,
                    });
                }
            }
        }

        // 3. Confidence range
        if decision.confidence < 0.0 || decision.confidence > 1.0 {
            issues.push(ValidationIssue {
                field: "confidence".into(),
                message: format!("Confidence {} is out of range [0.0, 1.0]", decision.confidence),
                severity: IssueSeverity::Warning,
            });
        }

        // 4. Confidence threshold check
        if decision.decision == DecisionType::UseTool && decision.confidence < confidence_threshold {
            issues.push(ValidationIssue {
                field: "confidence".into(),
                message: format!(
                    "Confidence {:.2} is below threshold {:.2}. Consider asking for clarification instead.",
                    decision.confidence, confidence_threshold,
                ),
                severity: IssueSeverity::Warning,
            });
        }

        // 5. Missing slots should trigger clarification, not tool execution
        if decision.decision == DecisionType::UseTool && !decision.missing_slots.is_empty() {
            issues.push(ValidationIssue {
                field: "missing_slots".into(),
                message: format!(
                    "Decision is use_tool but has missing slots: {:?}. Should ask_clarification instead.",
                    decision.missing_slots,
                ),
                severity: IssueSeverity::Warning,
            });
        }

        // Classify result
        if issues.is_empty() {
            ValidationResult::Valid
        } else {
            let has_errors = issues.iter().any(|i| i.severity == IssueSeverity::Error);
            if has_errors {
                ValidationResult::Invalid(issues)
            } else {
                ValidationResult::Repairable(issues)
            }
        }
    }
}

// ── Repair Prompt ─────────────────────────────────────────────────────

/// Generates a focused repair prompt when validation fails.
///
/// Instead of re-prompting the entire conversation, this generates a targeted
/// error message that tells the model exactly what went wrong and how to fix it.
pub struct RepairPrompt;

impl RepairPrompt {
    /// Build a repair prompt from validation issues.
    ///
    /// Returns a user message to append to the conversation that instructs
    /// the model to fix only the broken fields.
    pub fn build(decision: &StructuredDecision, issues: &[ValidationIssue]) -> String {
        let mut prompt = String::from(
            "Your previous structured decision had validation errors. Please fix and re-emit the JSON.\n\n"
        );

        prompt.push_str("ERRORS:\n");
        for issue in issues {
            prompt.push_str(&format!("- {}\n", issue));
        }

        prompt.push_str(&format!(
            "\nYour previous decision was: {}\nTool: {}\nReasoning: {}\n\n",
            decision.decision, decision.tool, decision.reasoning,
        ));

        prompt.push_str(
            "Please output a corrected JSON decision object. Only output the JSON, no other text."
        );

        prompt
    }
}

// ── System Prompt Fragment ────────────────────────────────────────────

/// Generates the structured output instruction to inject into the system prompt.
///
/// This instructs the model to emit structured JSON decisions instead of
/// raw function calls. Called when `ToolCallMode::StructuredJSON` is active.
pub fn structured_output_system_prompt(
    available_tools: &[&str],
    family_hint: Option<&str>,
) -> String {
    let tools_list = available_tools.join(", ");
    let family_line = family_hint
        .map(|f| format!("\nCurrent tool family: {f}"))
        .unwrap_or_default();

    format!(
r#"## Output Format

You MUST respond with a single JSON object for every user message. No prose before or after.
{family_line}

Available tools: [{tools_list}]

Schema:
```json
{{
  "decision": "use_tool | answer_directly | ask_clarification | refuse",
  "family": "<TOOL_FAMILY>",
  "tool": "<tool_name>",
  "args": {{ "<key>": "<value>" }},
  "confidence": 0.0-1.0,
  "grounding": ["source:id", ...],
  "missing_slots": ["<field_name>", ...],
  "needs_confirmation": true|false,
  "reasoning": "<brief explanation>",
  "answer": "<direct response text if not calling a tool>"
}}
```

Rules:
- If you can answer from the provided context, use "answer_directly" and put your response in "answer".
- If you need a tool, use "use_tool" with the tool name and arguments.
- If required information is missing, use "ask_clarification" and explain what you need in "answer".
- If the request is unsafe or out of scope, use "refuse" with explanation in "answer".
- "grounding" must list sources for any factual claims (calendar:id, memory:id, weather:today, etc.).
- "confidence" should reflect how certain you are. Below 0.7 = consider clarifying instead.
- "needs_confirmation" should be true for any action that changes data (send email, create event, delete file).
- NEVER invent personal facts. If not in context, set decision to "ask_clarification"."#
    )
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Parser Tests ──────────────────────────────────────────────

    #[test]
    fn parse_pure_json() {
        let json = r#"{"decision": "use_tool", "tool": "email_send", "args": {"to": "alice@example.com"}, "confidence": 0.9, "reasoning": "User wants to send email"}"#;
        let d = StructuredDecisionParser::parse(json).unwrap();
        assert_eq!(d.decision, DecisionType::UseTool);
        assert_eq!(d.tool, "email_send");
        assert_eq!(d.confidence, 0.9);
        assert_eq!(d.args["to"], "alice@example.com");
    }

    #[test]
    fn parse_code_block() {
        let text = "Here's my decision:\n```json\n{\"decision\": \"answer_directly\", \"answer\": \"It will rain tomorrow.\", \"confidence\": 0.85, \"grounding\": [\"weather:forecast_today\"]}\n```\n";
        let d = StructuredDecisionParser::parse(text).unwrap();
        assert_eq!(d.decision, DecisionType::AnswerDirectly);
        assert_eq!(d.answer, "It will rain tomorrow.");
        assert_eq!(d.grounding, vec!["weather:forecast_today"]);
    }

    #[test]
    fn parse_embedded_json() {
        let text = "I think you should do this: {\"decision\": \"use_tool\", \"tool\": \"recall\", \"args\": {\"query\": \"trip plans\"}, \"confidence\": 0.7} and that should help.";
        let d = StructuredDecisionParser::parse(text).unwrap();
        assert_eq!(d.decision, DecisionType::UseTool);
        assert_eq!(d.tool, "recall");
    }

    #[test]
    fn parse_lenient_decision_type() {
        let json = r#"{"decision": "tool", "tool": "web_search", "args": {"query": "rust async"}}"#;
        let d = StructuredDecisionParser::parse(json).unwrap();
        assert_eq!(d.decision, DecisionType::UseTool);
    }

    #[test]
    fn parse_with_defaults() {
        let json = r#"{"decision": "answer_directly", "answer": "Hello!"}"#;
        let d = StructuredDecisionParser::parse(json).unwrap();
        assert_eq!(d.confidence, 0.5); // default
        assert!(d.grounding.is_empty()); // default
        assert!(d.missing_slots.is_empty()); // default
        assert!(!d.needs_confirmation); // default
    }

    #[test]
    fn parse_nested_args() {
        let json = r#"{"decision": "use_tool", "tool": "calendar_create_event", "args": {"title": "Lunch", "attendees": ["alice@example.com", "bob@example.com"], "start_time": "2026-03-10T12:00:00"}, "confidence": 0.92, "needs_confirmation": true}"#;
        let d = StructuredDecisionParser::parse(json).unwrap();
        assert!(d.needs_confirmation);
        assert_eq!(d.args["attendees"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn parse_no_json_fails() {
        let text = "I don't know what to do, please help me.";
        let err = StructuredDecisionParser::parse(text).unwrap_err();
        assert!(matches!(err, ParseError::NoJsonFound));
    }

    #[test]
    fn parse_invalid_json_fails() {
        let text = "{decision: use_tool, tool: recall}"; // not valid JSON
        let err = StructuredDecisionParser::parse(text).unwrap_err();
        assert!(matches!(err, ParseError::InvalidJson(_)));
    }

    #[test]
    fn parse_unknown_decision_fails() {
        let json = r#"{"decision": "fly_to_moon", "tool": "rocket"}"#;
        let err = StructuredDecisionParser::parse(json).unwrap_err();
        assert!(matches!(err, ParseError::UnknownDecision(_)));
    }

    #[test]
    fn parse_missing_decision_fails() {
        let json = r#"{"tool": "recall", "args": {}}"#;
        let err = StructuredDecisionParser::parse(json).unwrap_err();
        assert!(matches!(err, ParseError::MissingRequiredField(_)));
    }

    // ── Validator Tests ───────────────────────────────────────────

    #[test]
    fn validate_valid_tool_call() {
        let d = StructuredDecision {
            decision: DecisionType::UseTool,
            family: "REMEMBER".into(),
            tool: "recall".into(),
            args: serde_json::json!({"query": "trip plans"}),
            confidence: 0.85,
            grounding: vec!["user_request".into()],
            missing_slots: vec![],
            needs_confirmation: false,
            reasoning: "User asked about trip plans".into(),
            answer: String::new(),
        };

        let result = StructuredDecisionValidator::validate(
            &d, Some(&["recall", "remember", "web_search"]), 0.7,
        );
        assert!(matches!(result, ValidationResult::Valid));
    }

    #[test]
    fn validate_missing_tool_name() {
        let d = StructuredDecision {
            decision: DecisionType::UseTool,
            tool: String::new(),
            ..default_decision()
        };

        let result = StructuredDecisionValidator::validate(&d, None, 0.7);
        assert!(matches!(result, ValidationResult::Invalid(_)));
    }

    #[test]
    fn validate_unknown_tool() {
        let d = StructuredDecision {
            decision: DecisionType::UseTool,
            tool: "nonexistent_tool".into(),
            args: serde_json::json!({}),
            ..default_decision()
        };

        let result = StructuredDecisionValidator::validate(
            &d, Some(&["recall", "remember"]), 0.7,
        );
        assert!(matches!(result, ValidationResult::Invalid(_)));
    }

    #[test]
    fn validate_low_confidence_warning() {
        let d = StructuredDecision {
            decision: DecisionType::UseTool,
            tool: "recall".into(),
            args: serde_json::json!({"query": "test"}),
            confidence: 0.3,
            ..default_decision()
        };

        let result = StructuredDecisionValidator::validate(
            &d, Some(&["recall"]), 0.7,
        );
        assert!(matches!(result, ValidationResult::Repairable(_)));
    }

    #[test]
    fn validate_missing_slots_warning() {
        let d = StructuredDecision {
            decision: DecisionType::UseTool,
            tool: "email_send".into(),
            args: serde_json::json!({"to": "alice@example.com"}),
            confidence: 0.8,
            missing_slots: vec!["subject".into()],
            ..default_decision()
        };

        let result = StructuredDecisionValidator::validate(
            &d, Some(&["email_send"]), 0.7,
        );
        assert!(matches!(result, ValidationResult::Repairable(_)));
    }

    #[test]
    fn validate_answer_directly() {
        let d = StructuredDecision {
            decision: DecisionType::AnswerDirectly,
            answer: "The meeting is at 3pm.".into(),
            confidence: 0.95,
            grounding: vec!["calendar:evt_123".into()],
            ..default_decision()
        };

        let result = StructuredDecisionValidator::validate(&d, None, 0.7);
        assert!(matches!(result, ValidationResult::Valid));
    }

    #[test]
    fn validate_answer_without_text_warns() {
        let d = StructuredDecision {
            decision: DecisionType::AnswerDirectly,
            answer: String::new(),
            ..default_decision()
        };

        let result = StructuredDecisionValidator::validate(&d, None, 0.7);
        assert!(matches!(result, ValidationResult::Repairable(_)));
    }

    // ── Conversion Tests ──────────────────────────────────────────

    #[test]
    fn to_tool_call_success() {
        let d = StructuredDecision {
            decision: DecisionType::UseTool,
            tool: "recall".into(),
            args: serde_json::json!({"query": "trip"}),
            ..default_decision()
        };

        let tc = d.to_tool_call().unwrap();
        assert_eq!(tc.name, "recall");
        assert_eq!(tc.arguments["query"], "trip");
    }

    #[test]
    fn to_tool_call_none_for_answer() {
        let d = StructuredDecision {
            decision: DecisionType::AnswerDirectly,
            answer: "Hello!".into(),
            ..default_decision()
        };

        assert!(d.to_tool_call().is_none());
    }

    // ── Repair Prompt Tests ───────────────────────────────────────

    #[test]
    fn repair_prompt_includes_errors() {
        let d = default_decision();
        let issues = vec![
            ValidationIssue {
                field: "tool".into(),
                message: "Unknown tool 'foobar'".into(),
                severity: IssueSeverity::Error,
            },
        ];

        let prompt = RepairPrompt::build(&d, &issues);
        assert!(prompt.contains("Unknown tool 'foobar'"));
        assert!(prompt.contains("corrected JSON"));
    }

    // ── System Prompt Tests ───────────────────────────────────────

    #[test]
    fn system_prompt_contains_tools() {
        let prompt = structured_output_system_prompt(
            &["recall", "remember", "web_search"],
            Some("REMEMBER"),
        );
        assert!(prompt.contains("recall, remember, web_search"));
        assert!(prompt.contains("REMEMBER"));
        assert!(prompt.contains("use_tool"));
        assert!(prompt.contains("answer_directly"));
    }

    #[test]
    fn system_prompt_without_family() {
        let prompt = structured_output_system_prompt(&["recall"], None);
        assert!(!prompt.contains("Current tool family:"));
    }

    // ── Helper ────────────────────────────────────────────────────

    fn default_decision() -> StructuredDecision {
        StructuredDecision {
            decision: DecisionType::UseTool,
            family: String::new(),
            tool: String::new(),
            args: serde_json::json!({}),
            confidence: 0.5,
            grounding: vec![],
            missing_slots: vec![],
            needs_confirmation: false,
            reasoning: String::new(),
            answer: String::new(),
        }
    }
}
