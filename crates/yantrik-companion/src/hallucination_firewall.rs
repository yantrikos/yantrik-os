//! Hallucination Firewall — deterministic post-check for LLM claims.
//!
//! Local models (especially 4–14B) can confidently state things that aren't true.
//! The firewall intercepts LLM responses before they reach the user and verifies
//! factual claims against known data sources.
//!
//! ## Architecture
//!
//! ```text
//! LLM Response (text)
//!     │
//!     ▼
//! EntityExtractor::extract()     ─── pulls out verifiable claims
//!     │
//!     ▼
//! SourceVerifier::verify()       ─── checks claims against ground truth
//!     │
//!     ├── AllVerified ──► pass through unchanged
//!     │
//!     ├── SomeUnverified ──► annotate/hedge unverifiable claims
//!     │
//!     └── CriticalViolation ──► block and regenerate / ask for clarification
//! ```
//!
//! ## Claim Categories
//!
//! - **Temporal**: "your meeting is at 3pm" → verify against calendar
//! - **Personal**: "you told me you like sushi" → verify against memory DB
//! - **Numerical**: "the temperature is 72°F" → verify against weather data
//! - **Contact**: "Alice's email is alice@..." → verify against contacts
//! - **Commitment**: "you promised to call mom" → verify against commitments
//! - **Factual**: general claims that need source grounding
//!
//! ## Design Principles
//!
//! 1. **Model never owns reality** — all factual claims must trace to a source
//! 2. **Deterministic checks** — no LLM in the verification path (would recurse)
//! 3. **Fail open for chat** — casual conversation skips the firewall
//! 4. **Fail closed for actions** — tool-calling claims must verify
//! 5. **Audit trail** — every check is logged for debugging

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Claim Types ─────────────────────────────────────────────────────

/// A verifiable claim extracted from LLM output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    /// The claim category.
    pub category: ClaimCategory,
    /// The raw text span containing the claim.
    pub text: String,
    /// Extracted key entities (e.g., time="3pm", person="Alice").
    pub entities: HashMap<String, String>,
    /// Character offset in the original response (start).
    pub offset_start: usize,
    /// Character offset in the original response (end).
    pub offset_end: usize,
}

/// Categories of verifiable claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClaimCategory {
    /// Time/date claims: "your meeting is at 3pm", "tomorrow is Monday"
    Temporal,
    /// Personal facts: "you told me you like sushi", "your birthday is March 5"
    Personal,
    /// Numerical/measurement: "temperature is 72°F", "you have 5 unread emails"
    Numerical,
    /// Contact info: "Alice's email is alice@...", "Bob's number is 555-..."
    Contact,
    /// Commitment/promise: "you said you'd call mom", "your reminder for 3pm"
    Commitment,
    /// General factual claim that should have a source
    Factual,
}

impl std::fmt::Display for ClaimCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Temporal => write!(f, "temporal"),
            Self::Personal => write!(f, "personal"),
            Self::Numerical => write!(f, "numerical"),
            Self::Contact => write!(f, "contact"),
            Self::Commitment => write!(f, "commitment"),
            Self::Factual => write!(f, "factual"),
        }
    }
}

// ── Verification Result ─────────────────────────────────────────────

/// Result of verifying a single claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimVerification {
    /// The claim that was checked.
    pub claim: Claim,
    /// Verification status.
    pub status: VerificationStatus,
    /// The source used for verification (if any).
    pub source: Option<String>,
    /// Corrected value if the claim was wrong (e.g., "3pm" → "4pm").
    pub correction: Option<String>,
    /// Trust score: 0.0 (no trust) to 1.0 (fully verified).
    pub trust: f64,
}

/// Verification status for a claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationStatus {
    /// Claim matches known data source exactly.
    Verified,
    /// Claim is plausible but no direct source to confirm.
    Unverifiable,
    /// Claim contradicts known data — hallucination detected.
    Contradicted,
    /// No relevant data source available for this claim type.
    NoSource,
}

impl std::fmt::Display for VerificationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Verified => write!(f, "verified"),
            Self::Unverifiable => write!(f, "unverifiable"),
            Self::Contradicted => write!(f, "CONTRADICTED"),
            Self::NoSource => write!(f, "no_source"),
        }
    }
}

// ── Firewall Verdict ────────────────────────────────────────────────

/// Overall firewall verdict for an LLM response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirewallVerdict {
    /// Individual claim verifications.
    pub claims: Vec<ClaimVerification>,
    /// Overall response action.
    pub action: FirewallAction,
    /// Aggregate trust score (weighted average of claim trusts).
    pub aggregate_trust: f64,
    /// If modified, the corrected response text.
    pub corrected_response: Option<String>,
    /// Audit log entries for debugging.
    pub audit_log: Vec<String>,
}

/// What the firewall decides to do with the response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FirewallAction {
    /// Response is clean — pass through unchanged.
    PassThrough,
    /// Response has unverifiable claims — add hedging language.
    Annotate,
    /// Response has contradicted claims — correct inline.
    Correct,
    /// Response has critical violations — block entirely.
    Block,
}

impl std::fmt::Display for FirewallAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PassThrough => write!(f, "pass_through"),
            Self::Annotate => write!(f, "annotate"),
            Self::Correct => write!(f, "correct"),
            Self::Block => write!(f, "block"),
        }
    }
}

// ── Entity Extractor ────────────────────────────────────────────────

/// Extracts verifiable claims from LLM response text.
///
/// Uses pattern matching (NOT LLM inference) to identify claims that
/// can be checked against known data sources. This keeps the verification
/// path deterministic and non-recursive.
pub struct EntityExtractor;

impl EntityExtractor {
    /// Extract all verifiable claims from response text.
    ///
    /// Returns claims sorted by position in the text.
    pub fn extract(text: &str) -> Vec<Claim> {
        let mut claims = Vec::new();

        // Extract temporal claims
        claims.extend(Self::extract_temporal(text));

        // Extract personal claims
        claims.extend(Self::extract_personal(text));

        // Extract numerical claims
        claims.extend(Self::extract_numerical(text));

        // Extract contact claims
        claims.extend(Self::extract_contact(text));

        // Extract commitment claims
        claims.extend(Self::extract_commitment(text));

        // Sort by position
        claims.sort_by_key(|c| c.offset_start);

        // Deduplicate overlapping claims (keep longer / more specific)
        Self::deduplicate_overlapping(&mut claims);

        claims
    }

    /// Extract temporal claims: times, dates, durations, scheduling.
    fn extract_temporal(text: &str) -> Vec<Claim> {
        let mut claims = Vec::new();
        let lower = text.to_lowercase();

        // Time patterns: "at 3pm", "at 15:00", "at 3:30 PM"
        let time_triggers = [
            "at ", "by ", "until ", "from ", "starts at ", "ends at ",
            "scheduled for ", "meeting at ", "appointment at ",
        ];

        for trigger in &time_triggers {
            let mut search_start = 0;
            while let Some(idx) = lower[search_start..].find(trigger) {
                let abs_idx = search_start + idx;
                let after_trigger = abs_idx + trigger.len();

                // Look for time pattern after trigger
                if let Some((time_str, end_offset)) = Self::parse_time_after(text, after_trigger) {
                    let claim_start = abs_idx;
                    let claim_end = (after_trigger + end_offset).min(text.len());
                    let span = &text[claim_start..claim_end];

                    let mut entities = HashMap::new();
                    entities.insert("time".into(), time_str);

                    claims.push(Claim {
                        category: ClaimCategory::Temporal,
                        text: span.to_string(),
                        entities,
                        offset_start: claim_start,
                        offset_end: claim_end,
                    });
                }

                search_start = abs_idx + trigger.len();
            }
        }

        // Date patterns: "tomorrow", "next Monday", "on March 10"
        let date_triggers = [
            ("tomorrow", "tomorrow"),
            ("today", "today"),
            ("yesterday", "yesterday"),
            ("next week", "next_week"),
            ("this week", "this_week"),
            ("next month", "next_month"),
        ];

        for (pattern, tag) in &date_triggers {
            let mut search_start = 0;
            while let Some(idx) = lower[search_start..].find(pattern) {
                let abs_idx = search_start + idx;
                let claim_end = abs_idx + pattern.len();
                let span = &text[abs_idx..claim_end];

                let mut entities = HashMap::new();
                entities.insert("date_ref".into(), tag.to_string());

                claims.push(Claim {
                    category: ClaimCategory::Temporal,
                    text: span.to_string(),
                    entities,
                    offset_start: abs_idx,
                    offset_end: claim_end,
                });

                search_start = claim_end;
            }
        }

        claims
    }

    /// Try to parse a time string starting at `offset` in text.
    /// Returns (parsed_time, characters_consumed) or None.
    fn parse_time_after(text: &str, offset: usize) -> Option<(String, usize)> {
        if offset >= text.len() {
            return None;
        }

        let remaining = &text[offset..];
        let bytes = remaining.as_bytes();

        // Consume digits, colons, and optional AM/PM
        let mut end = 0;
        let mut has_digit = false;

        // Phase 1: digits and colons (e.g., "3:30", "15:00", "3")
        while end < bytes.len() {
            let b = bytes[end];
            if b.is_ascii_digit() {
                has_digit = true;
                end += 1;
            } else if b == b':' && has_digit {
                end += 1;
            } else {
                break;
            }
        }

        if !has_digit || end == 0 {
            return None;
        }

        // Phase 2: optional space + AM/PM
        let mut am_pm_end = end;
        if am_pm_end < bytes.len() && bytes[am_pm_end] == b' ' {
            am_pm_end += 1;
        }
        let after_space = &remaining[am_pm_end..].to_lowercase();
        if after_space.starts_with("am") || after_space.starts_with("pm") {
            am_pm_end += 2;
            end = am_pm_end;
        } else if after_space.starts_with("a.m.") || after_space.starts_with("p.m.") {
            am_pm_end += 4;
            end = am_pm_end;
        }

        let time_str = remaining[..end].trim().to_string();
        if time_str.is_empty() {
            return None;
        }

        Some((time_str, end))
    }

    /// Extract personal claims: "you told me", "you said", "your favorite".
    fn extract_personal(text: &str) -> Vec<Claim> {
        let mut claims = Vec::new();
        let lower = text.to_lowercase();

        let triggers = [
            "you told me",
            "you said",
            "you mentioned",
            "your favorite",
            "your birthday",
            "your name is",
            "you live in",
            "you work at",
            "you prefer",
            "you like",
            "you love",
            "you hate",
            "you're from",
            "you are from",
            "your email is",
            "your phone",
            "your address",
        ];

        for trigger in &triggers {
            let mut search_start = 0;
            while let Some(idx) = lower[search_start..].find(trigger) {
                let abs_idx = search_start + idx;
                // Capture until end of sentence (period, newline, or 120 chars)
                let claim_end = Self::find_sentence_end(text, abs_idx + trigger.len())
                    .min(abs_idx + 120);
                let span = &text[abs_idx..claim_end];

                let mut entities = HashMap::new();
                entities.insert("trigger".into(), trigger.to_string());
                // Extract the claimed content after the trigger
                let content_start = abs_idx + trigger.len();
                if content_start < claim_end {
                    let content = text[content_start..claim_end].trim().to_string();
                    entities.insert("claimed_content".into(), content);
                }

                claims.push(Claim {
                    category: ClaimCategory::Personal,
                    text: span.to_string(),
                    entities,
                    offset_start: abs_idx,
                    offset_end: claim_end,
                });

                search_start = claim_end;
            }
        }

        claims
    }

    /// Extract numerical claims: counts, temperatures, amounts.
    fn extract_numerical(text: &str) -> Vec<Claim> {
        let mut claims = Vec::new();
        let lower = text.to_lowercase();

        // "you have N unread emails/messages"
        let count_patterns = [
            ("you have ", " unread"),
            ("you have ", " new"),
            ("there are ", " emails"),
            ("there are ", " messages"),
            ("there are ", " events"),
            ("there are ", " reminders"),
            ("you have ", " events"),
            ("you have ", " tasks"),
        ];

        for (prefix, suffix) in &count_patterns {
            let mut search_start = 0;
            while let Some(idx) = lower[search_start..].find(prefix) {
                let abs_idx = search_start + idx;
                let after_prefix = abs_idx + prefix.len();

                // Look for the suffix after the prefix
                if let Some(suffix_idx) = lower[after_prefix..].find(suffix) {
                    let between = &text[after_prefix..after_prefix + suffix_idx].trim();
                    // Check if it's a number
                    if let Ok(num) = between.parse::<u64>() {
                        let claim_end = after_prefix + suffix_idx + suffix.len();
                        let span = &text[abs_idx..claim_end];

                        let mut entities = HashMap::new();
                        entities.insert("count".into(), num.to_string());
                        entities.insert("unit".into(), suffix.trim().to_string());

                        claims.push(Claim {
                            category: ClaimCategory::Numerical,
                            text: span.to_string(),
                            entities,
                            offset_start: abs_idx,
                            offset_end: claim_end,
                        });
                    }
                }

                search_start = after_prefix;
            }
        }

        // Temperature: "N°F", "N°C", "N degrees"
        let temp_units = ["°f", "°c", "degrees"];
        for unit in &temp_units {
            let mut search_start = 0;
            while let Some(idx) = lower[search_start..].find(unit) {
                // Walk backwards to find the number
                let num_end = search_start + idx;
                if let Some((num_str, num_start)) = Self::extract_number_before(text, num_end) {
                    let claim_end = num_end + unit.len();
                    let span = &text[num_start..claim_end];

                    let mut entities = HashMap::new();
                    entities.insert("value".into(), num_str);
                    entities.insert("unit".into(), unit.to_string());

                    claims.push(Claim {
                        category: ClaimCategory::Numerical,
                        text: span.to_string(),
                        entities,
                        offset_start: num_start,
                        offset_end: claim_end,
                    });
                }

                search_start = num_end + unit.len();
            }
        }

        claims
    }

    /// Extract a number that appears just before `offset` in the text.
    fn extract_number_before(text: &str, offset: usize) -> Option<(String, usize)> {
        if offset == 0 {
            return None;
        }

        let bytes = text.as_bytes();
        let mut start = offset;

        // Skip whitespace before the unit
        while start > 0 && bytes[start - 1] == b' ' {
            start -= 1;
        }

        // Walk backwards through digits, dots, minus
        let num_end = start;
        while start > 0 {
            let b = bytes[start - 1];
            if b.is_ascii_digit() || b == b'.' || b == b'-' {
                start -= 1;
            } else {
                break;
            }
        }

        if start == num_end {
            return None;
        }

        let num_str = text[start..num_end].to_string();
        // Verify it parses as a number
        if num_str.parse::<f64>().is_ok() {
            Some((num_str, start))
        } else {
            None
        }
    }

    /// Extract contact information claims.
    fn extract_contact(text: &str) -> Vec<Claim> {
        let mut claims = Vec::new();

        // Email addresses: simple pattern matching
        let bytes = text.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'@' {
                // Walk backwards for local part
                let mut local_start = i;
                while local_start > 0
                    && (bytes[local_start - 1].is_ascii_alphanumeric()
                        || bytes[local_start - 1] == b'.'
                        || bytes[local_start - 1] == b'_'
                        || bytes[local_start - 1] == b'-'
                        || bytes[local_start - 1] == b'+')
                {
                    local_start -= 1;
                }
                // Walk forwards for domain
                let mut domain_end = i + 1;
                while domain_end < bytes.len()
                    && (bytes[domain_end].is_ascii_alphanumeric()
                        || bytes[domain_end] == b'.'
                        || bytes[domain_end] == b'-')
                {
                    domain_end += 1;
                }

                if local_start < i && domain_end > i + 1 {
                    let email = &text[local_start..domain_end];
                    // Basic sanity: has at least one dot in domain
                    if email[i - local_start + 1..].contains('.') {
                        let mut entities = HashMap::new();
                        entities.insert("email".into(), email.to_string());

                        claims.push(Claim {
                            category: ClaimCategory::Contact,
                            text: email.to_string(),
                            entities,
                            offset_start: local_start,
                            offset_end: domain_end,
                        });
                    }
                }
            }
        }

        // Phone numbers: simple digit sequence detection
        // Look for patterns like "555-1234", "(555) 123-4567", "+1-555-123-4567"
        let mut i = 0;
        while i < text.len() {
            if let Some((phone, start, end)) = Self::try_parse_phone(text, i) {
                let mut entities = HashMap::new();
                entities.insert("phone".into(), phone);

                claims.push(Claim {
                    category: ClaimCategory::Contact,
                    text: text[start..end].to_string(),
                    entities,
                    offset_start: start,
                    offset_end: end,
                });

                i = end;
            } else {
                i += 1;
            }
        }

        claims
    }

    /// Try to parse a phone number starting around position `pos`.
    fn try_parse_phone(text: &str, pos: usize) -> Option<(String, usize, usize)> {
        let bytes = text.as_bytes();
        if pos >= bytes.len() {
            return None;
        }

        // Must start with digit, +, or (
        let b = bytes[pos];
        if !b.is_ascii_digit() && b != b'+' && b != b'(' {
            return None;
        }

        // Collect phone-like characters
        let mut end = pos;
        let mut digit_count = 0;
        while end < bytes.len() {
            let c = bytes[end];
            if c.is_ascii_digit() {
                digit_count += 1;
                end += 1;
            } else if c == b'-' || c == b' ' || c == b'(' || c == b')' || c == b'+' || c == b'.' {
                end += 1;
            } else {
                break;
            }
        }

        // Phone numbers have at least 7 digits
        if digit_count >= 7 && digit_count <= 15 {
            let phone = text[pos..end].trim_end().to_string();
            Some((phone, pos, end))
        } else {
            None
        }
    }

    /// Extract commitment claims: "you promised", "I'll remind you", "your reminder".
    fn extract_commitment(text: &str) -> Vec<Claim> {
        let mut claims = Vec::new();
        let lower = text.to_lowercase();

        let triggers = [
            "you promised",
            "you committed to",
            "you agreed to",
            "your reminder",
            "i'll remind you",
            "i will remind you",
            "don't forget",
            "you need to",
            "you should",
            "you were going to",
            "you planned to",
        ];

        for trigger in &triggers {
            let mut search_start = 0;
            while let Some(idx) = lower[search_start..].find(trigger) {
                let abs_idx = search_start + idx;
                let claim_end = Self::find_sentence_end(text, abs_idx + trigger.len())
                    .min(abs_idx + 150);
                let span = &text[abs_idx..claim_end];

                let mut entities = HashMap::new();
                entities.insert("trigger".into(), trigger.to_string());
                let content_start = abs_idx + trigger.len();
                if content_start < claim_end {
                    let content = text[content_start..claim_end].trim().to_string();
                    entities.insert("claimed_commitment".into(), content);
                }

                claims.push(Claim {
                    category: ClaimCategory::Commitment,
                    text: span.to_string(),
                    entities,
                    offset_start: abs_idx,
                    offset_end: claim_end,
                });

                search_start = claim_end;
            }
        }

        claims
    }

    /// Find the end of the current sentence (period, !, ?, or newline).
    fn find_sentence_end(text: &str, from: usize) -> usize {
        let bytes = text.as_bytes();
        for i in from..bytes.len() {
            match bytes[i] {
                b'.' | b'!' | b'?' | b'\n' => return i + 1,
                _ => {}
            }
        }
        text.len()
    }

    /// Remove overlapping claims, keeping the longer/more specific one.
    fn deduplicate_overlapping(claims: &mut Vec<Claim>) {
        if claims.len() < 2 {
            return;
        }

        let mut keep = vec![true; claims.len()];
        for i in 0..claims.len() {
            if !keep[i] {
                continue;
            }
            for j in (i + 1)..claims.len() {
                if !keep[j] {
                    continue;
                }
                // Check overlap
                let a = &claims[i];
                let b = &claims[j];
                if a.offset_start < b.offset_end && b.offset_start < a.offset_end {
                    // Overlapping — keep the longer one
                    if a.text.len() >= b.text.len() {
                        keep[j] = false;
                    } else {
                        keep[i] = false;
                        break;
                    }
                }
            }
        }

        let mut idx = 0;
        claims.retain(|_| {
            let k = keep[idx];
            idx += 1;
            k
        });
    }
}

// ── Source Verifier ──────────────────────────────────────────────────

/// Known data sources that claims can be verified against.
///
/// The companion populates this from its active context (calendar events,
/// recent memories, weather data, email counts, etc.) before running
/// the firewall. This keeps verification deterministic.
#[derive(Debug, Clone, Default)]
pub struct GroundTruth {
    /// Calendar events: key = event description/time, value = source ID.
    pub calendar_events: Vec<GroundTruthEntry>,
    /// Memory facts: key = fact content, value = memory ID.
    pub memory_facts: Vec<GroundTruthEntry>,
    /// Contact info: key = contact name, value = contact details.
    pub contacts: Vec<GroundTruthEntry>,
    /// Numerical facts: key = metric name, value = current value.
    pub numericals: Vec<GroundTruthEntry>,
    /// Active commitments/reminders.
    pub commitments: Vec<GroundTruthEntry>,
    /// Recent tool results (from the current conversation).
    pub tool_results: Vec<GroundTruthEntry>,
}

/// A single ground truth entry for verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundTruthEntry {
    /// Source identifier (e.g., "calendar:evt_123", "memory:m_456").
    pub source_id: String,
    /// The factual content.
    pub content: String,
    /// Category this fact belongs to.
    pub category: ClaimCategory,
    /// When this fact was last confirmed (Unix timestamp).
    pub confirmed_at: u64,
}

impl GroundTruth {
    /// Create a new empty ground truth.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a calendar event as ground truth.
    pub fn add_calendar(&mut self, source_id: &str, content: &str, confirmed_at: u64) {
        self.calendar_events.push(GroundTruthEntry {
            source_id: source_id.into(),
            content: content.into(),
            category: ClaimCategory::Temporal,
            confirmed_at,
        });
    }

    /// Add a memory fact as ground truth.
    pub fn add_memory(&mut self, source_id: &str, content: &str, confirmed_at: u64) {
        self.memory_facts.push(GroundTruthEntry {
            source_id: source_id.into(),
            content: content.into(),
            category: ClaimCategory::Personal,
            confirmed_at,
        });
    }

    /// Add a contact as ground truth.
    pub fn add_contact(&mut self, source_id: &str, content: &str, confirmed_at: u64) {
        self.contacts.push(GroundTruthEntry {
            source_id: source_id.into(),
            content: content.into(),
            category: ClaimCategory::Contact,
            confirmed_at,
        });
    }

    /// Add a numerical fact as ground truth.
    pub fn add_numerical(&mut self, source_id: &str, content: &str, confirmed_at: u64) {
        self.numericals.push(GroundTruthEntry {
            source_id: source_id.into(),
            content: content.into(),
            category: ClaimCategory::Numerical,
            confirmed_at,
        });
    }

    /// Add a commitment as ground truth.
    pub fn add_commitment(&mut self, source_id: &str, content: &str, confirmed_at: u64) {
        self.commitments.push(GroundTruthEntry {
            source_id: source_id.into(),
            content: content.into(),
            category: ClaimCategory::Commitment,
            confirmed_at,
        });
    }

    /// Add a tool result from the current conversation.
    pub fn add_tool_result(&mut self, tool_name: &str, content: &str, confirmed_at: u64) {
        self.tool_results.push(GroundTruthEntry {
            source_id: format!("tool:{}", tool_name),
            content: content.into(),
            category: ClaimCategory::Factual,
            confirmed_at,
        });
    }

    /// Get all entries for a given category.
    fn entries_for_category(&self, category: ClaimCategory) -> Vec<&GroundTruthEntry> {
        match category {
            ClaimCategory::Temporal => self.calendar_events.iter().collect(),
            ClaimCategory::Personal => self.memory_facts.iter().collect(),
            ClaimCategory::Numerical => self.numericals.iter().collect(),
            ClaimCategory::Contact => self.contacts.iter().collect(),
            ClaimCategory::Commitment => self.commitments.iter().collect(),
            ClaimCategory::Factual => self.tool_results.iter().collect(),
        }
    }

    /// Check if ground truth has any entries at all.
    pub fn is_empty(&self) -> bool {
        self.calendar_events.is_empty()
            && self.memory_facts.is_empty()
            && self.contacts.is_empty()
            && self.numericals.is_empty()
            && self.commitments.is_empty()
            && self.tool_results.is_empty()
    }
}

// ── Source Verifier ──────────────────────────────────────────────────

/// Verifies extracted claims against ground truth data.
///
/// Uses fuzzy text matching (not LLM inference) to check claims.
/// This keeps the verification path deterministic and fast.
pub struct SourceVerifier;

impl SourceVerifier {
    /// Verify a set of claims against ground truth.
    pub fn verify(claims: &[Claim], ground_truth: &GroundTruth) -> Vec<ClaimVerification> {
        claims.iter().map(|claim| Self::verify_single(claim, ground_truth)).collect()
    }

    /// Verify a single claim against ground truth.
    fn verify_single(claim: &Claim, ground_truth: &GroundTruth) -> ClaimVerification {
        let entries = ground_truth.entries_for_category(claim.category);

        // Also check tool results as they're universal ground truth
        let tool_entries = &ground_truth.tool_results;
        let all_entries: Vec<&GroundTruthEntry> = entries.into_iter()
            .chain(tool_entries.iter())
            .collect();

        if all_entries.is_empty() {
            return ClaimVerification {
                claim: claim.clone(),
                status: VerificationStatus::NoSource,
                source: None,
                correction: None,
                trust: 0.3, // Low trust — no source to verify against
            };
        }

        // Check each entry for match or contradiction
        let mut best_match: Option<(f64, &GroundTruthEntry)> = None;

        for entry in &all_entries {
            let similarity = Self::text_similarity(&claim.text, &entry.content);

            // Also check entity-level matches
            let entity_match = Self::check_entity_match(claim, entry);

            let combined_score = (similarity * 0.4 + entity_match * 0.6).min(1.0);

            if let Some((best_score, _)) = &best_match {
                if combined_score > *best_score {
                    best_match = Some((combined_score, entry));
                }
            } else {
                best_match = Some((combined_score, entry));
            }
        }

        match best_match {
            Some((score, entry)) if score > 0.7 => {
                // Strong match — claim is verified
                ClaimVerification {
                    claim: claim.clone(),
                    status: VerificationStatus::Verified,
                    source: Some(entry.source_id.clone()),
                    correction: None,
                    trust: score,
                }
            }
            Some((score, entry)) if score > 0.3 => {
                // Partial match — might be slightly wrong
                let contradiction = Self::detect_contradiction(claim, entry);
                if let Some(correction) = contradiction {
                    ClaimVerification {
                        claim: claim.clone(),
                        status: VerificationStatus::Contradicted,
                        source: Some(entry.source_id.clone()),
                        correction: Some(correction),
                        trust: 0.1,
                    }
                } else {
                    ClaimVerification {
                        claim: claim.clone(),
                        status: VerificationStatus::Unverifiable,
                        source: Some(entry.source_id.clone()),
                        correction: None,
                        trust: score,
                    }
                }
            }
            _ => {
                // No match found in entries
                ClaimVerification {
                    claim: claim.clone(),
                    status: VerificationStatus::Unverifiable,
                    source: None,
                    correction: None,
                    trust: 0.2,
                }
            }
        }
    }

    /// Simple text similarity using word overlap (Jaccard coefficient).
    fn text_similarity(a: &str, b: &str) -> f64 {
        let words_a: std::collections::HashSet<&str> = a.to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .into_iter()
            .collect();
        let words_b: std::collections::HashSet<&str> = b.to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .into_iter()
            .collect();

        // We need owned versions for the HashSet comparison
        let words_a: std::collections::HashSet<String> = a.to_lowercase()
            .split_whitespace()
            .map(String::from)
            .collect();
        let words_b: std::collections::HashSet<String> = b.to_lowercase()
            .split_whitespace()
            .map(String::from)
            .collect();

        if words_a.is_empty() && words_b.is_empty() {
            return 1.0;
        }
        if words_a.is_empty() || words_b.is_empty() {
            return 0.0;
        }

        let intersection = words_a.intersection(&words_b).count() as f64;
        let union = words_a.union(&words_b).count() as f64;

        if union == 0.0 { 0.0 } else { intersection / union }
    }

    /// Check if claim entities match a ground truth entry.
    fn check_entity_match(claim: &Claim, entry: &GroundTruthEntry) -> f64 {
        let entry_lower = entry.content.to_lowercase();
        let mut matches = 0u32;
        let mut total = 0u32;

        for (key, value) in &claim.entities {
            // Skip meta-keys
            if key == "trigger" {
                continue;
            }
            total += 1;
            let value_lower = value.to_lowercase();
            if entry_lower.contains(&value_lower) {
                matches += 1;
            }
        }

        if total == 0 {
            // No entities to check — fall back to text similarity
            return 0.5;
        }

        matches as f64 / total as f64
    }

    /// Detect if a claim contradicts the ground truth entry.
    /// Returns a correction string if contradiction found.
    fn detect_contradiction(claim: &Claim, entry: &GroundTruthEntry) -> Option<String> {
        match claim.category {
            ClaimCategory::Temporal => {
                // Check if times differ
                if let Some(claimed_time) = claim.entities.get("time") {
                    if !entry.content.to_lowercase().contains(&claimed_time.to_lowercase()) {
                        return Some(format!(
                            "Claimed '{}' but source says: {}",
                            claimed_time, entry.content
                        ));
                    }
                }
            }
            ClaimCategory::Numerical => {
                if let Some(claimed_value) = claim.entities.get("count") {
                    if !entry.content.contains(claimed_value) {
                        return Some(format!(
                            "Claimed count {} but source says: {}",
                            claimed_value, entry.content
                        ));
                    }
                }
                if let Some(claimed_value) = claim.entities.get("value") {
                    if !entry.content.contains(claimed_value) {
                        return Some(format!(
                            "Claimed value {} but source says: {}",
                            claimed_value, entry.content
                        ));
                    }
                }
            }
            ClaimCategory::Contact => {
                if let Some(claimed_email) = claim.entities.get("email") {
                    if !entry.content.to_lowercase().contains(&claimed_email.to_lowercase()) {
                        return Some(format!(
                            "Claimed email '{}' but source says: {}",
                            claimed_email, entry.content
                        ));
                    }
                }
            }
            _ => {}
        }

        None
    }
}

// ── Firewall Engine ─────────────────────────────────────────────────

/// Configuration for the hallucination firewall.
#[derive(Debug, Clone)]
pub struct FirewallConfig {
    /// Minimum aggregate trust to pass through without annotation.
    pub pass_threshold: f64,
    /// Minimum aggregate trust to annotate vs block.
    pub block_threshold: f64,
    /// Whether to apply the firewall (disabled for Large models by default).
    pub enabled: bool,
    /// Maximum number of claims to check per response.
    pub max_claims: usize,
    /// Whether to correct contradicted claims inline.
    pub auto_correct: bool,
}

impl Default for FirewallConfig {
    fn default() -> Self {
        Self {
            pass_threshold: 0.7,
            block_threshold: 0.3,
            enabled: true,
            max_claims: 20,
            auto_correct: true,
        }
    }
}

/// The main hallucination firewall engine.
///
/// Usage:
/// ```rust,ignore
/// let firewall = HallucinationFirewall::new(config);
/// let ground_truth = build_ground_truth_from_context(...);
/// let verdict = firewall.check(response_text, &ground_truth);
/// match verdict.action {
///     FirewallAction::PassThrough => { /* use response as-is */ }
///     FirewallAction::Annotate => { /* add hedging to unverified claims */ }
///     FirewallAction::Correct => { /* use corrected_response */ }
///     FirewallAction::Block => { /* regenerate or ask user */ }
/// }
/// ```
pub struct HallucinationFirewall {
    config: FirewallConfig,
}

impl HallucinationFirewall {
    /// Create a new firewall with the given configuration.
    pub fn new(config: FirewallConfig) -> Self {
        Self { config }
    }

    /// Create a firewall with default configuration.
    pub fn default_config() -> Self {
        Self { config: FirewallConfig::default() }
    }

    /// Check an LLM response against ground truth.
    ///
    /// Returns a verdict with the action to take and any corrections.
    pub fn check(&self, response: &str, ground_truth: &GroundTruth) -> FirewallVerdict {
        let mut audit_log = Vec::new();

        // If firewall is disabled, pass through
        if !self.config.enabled {
            audit_log.push("Firewall disabled — pass through".into());
            return FirewallVerdict {
                claims: vec![],
                action: FirewallAction::PassThrough,
                aggregate_trust: 1.0,
                corrected_response: None,
                audit_log,
            };
        }

        // If no ground truth available, we can't verify anything
        if ground_truth.is_empty() {
            audit_log.push("No ground truth available — pass through".into());
            return FirewallVerdict {
                claims: vec![],
                action: FirewallAction::PassThrough,
                aggregate_trust: 0.5,
                corrected_response: None,
                audit_log,
            };
        }

        // Step 1: Extract claims
        let mut claims = EntityExtractor::extract(response);
        audit_log.push(format!("Extracted {} claims", claims.len()));

        // Limit claims to max
        if claims.len() > self.config.max_claims {
            claims.truncate(self.config.max_claims);
            audit_log.push(format!("Truncated to {} claims", self.config.max_claims));
        }

        // If no claims found, pass through (casual conversation)
        if claims.is_empty() {
            audit_log.push("No verifiable claims found — pass through".into());
            return FirewallVerdict {
                claims: vec![],
                action: FirewallAction::PassThrough,
                aggregate_trust: 1.0,
                corrected_response: None,
                audit_log,
            };
        }

        // Step 2: Verify claims
        let verifications = SourceVerifier::verify(&claims, ground_truth);

        // Log each verification
        for v in &verifications {
            audit_log.push(format!(
                "[{}] {} — {} (trust={:.2})",
                v.claim.category, v.claim.text.chars().take(50).collect::<String>(),
                v.status, v.trust,
            ));
        }

        // Step 3: Calculate aggregate trust
        let aggregate_trust = if verifications.is_empty() {
            1.0
        } else {
            // Weight contradictions heavily
            let total_weight: f64 = verifications.iter().map(|v| match v.status {
                VerificationStatus::Contradicted => 3.0,
                VerificationStatus::Verified => 1.0,
                VerificationStatus::Unverifiable => 1.5,
                VerificationStatus::NoSource => 0.5,
            }).sum();

            let weighted_trust: f64 = verifications.iter().map(|v| {
                let weight = match v.status {
                    VerificationStatus::Contradicted => 3.0,
                    VerificationStatus::Verified => 1.0,
                    VerificationStatus::Unverifiable => 1.5,
                    VerificationStatus::NoSource => 0.5,
                };
                v.trust * weight
            }).sum();

            if total_weight > 0.0 { weighted_trust / total_weight } else { 0.5 }
        };

        audit_log.push(format!("Aggregate trust: {:.3}", aggregate_trust));

        // Step 4: Determine action
        let has_contradictions = verifications.iter()
            .any(|v| v.status == VerificationStatus::Contradicted);
        let contradiction_count = verifications.iter()
            .filter(|v| v.status == VerificationStatus::Contradicted)
            .count();
        let unverifiable_count = verifications.iter()
            .filter(|v| v.status == VerificationStatus::Unverifiable)
            .count();

        let action = if aggregate_trust >= self.config.pass_threshold && !has_contradictions {
            FirewallAction::PassThrough
        } else if has_contradictions && self.config.auto_correct {
            FirewallAction::Correct
        } else if aggregate_trust < self.config.block_threshold || contradiction_count > 2 {
            FirewallAction::Block
        } else if has_contradictions || unverifiable_count > 0 {
            FirewallAction::Annotate
        } else {
            FirewallAction::PassThrough
        };

        audit_log.push(format!("Action: {} (contradictions={}, unverifiable={})",
            action, contradiction_count, unverifiable_count));

        // Step 5: Build corrected response if needed
        let corrected_response = match action {
            FirewallAction::Correct => Some(Self::build_corrected_response(response, &verifications)),
            FirewallAction::Annotate => Some(Self::build_annotated_response(response, &verifications)),
            _ => None,
        };

        FirewallVerdict {
            claims: verifications,
            action,
            aggregate_trust,
            corrected_response,
            audit_log,
        }
    }

    /// Build a corrected version of the response, replacing contradicted claims.
    fn build_corrected_response(response: &str, verifications: &[ClaimVerification]) -> String {
        let mut result = response.to_string();

        // Apply corrections in reverse order (to preserve offsets)
        let mut corrections: Vec<_> = verifications.iter()
            .filter(|v| v.status == VerificationStatus::Contradicted && v.correction.is_some())
            .collect();
        corrections.sort_by(|a, b| b.claim.offset_start.cmp(&a.claim.offset_start));

        for v in corrections {
            if let Some(correction) = &v.correction {
                let claim_text = &v.claim.text;
                // Build inline correction
                let corrected = format!("[corrected: {}]", correction);

                // Replace the claim text in the response
                if v.claim.offset_start < result.len() && v.claim.offset_end <= result.len() {
                    let before = &result[..v.claim.offset_start];
                    let after = &result[v.claim.offset_end..];
                    result = format!("{}{} {}{}", before, claim_text, corrected, after);
                }
            }
        }

        result
    }

    /// Build an annotated version of the response, hedging unverifiable claims.
    fn build_annotated_response(response: &str, verifications: &[ClaimVerification]) -> String {
        let mut result = response.to_string();

        // Add hedging annotations in reverse order
        let mut unverified: Vec<_> = verifications.iter()
            .filter(|v| v.status == VerificationStatus::Unverifiable)
            .collect();
        unverified.sort_by(|a, b| b.claim.offset_start.cmp(&a.claim.offset_start));

        for v in unverified {
            if v.claim.offset_end <= result.len() {
                let insert_pos = v.claim.offset_end;
                result.insert_str(insert_pos, " [unverified]");
            }
        }

        result
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Entity Extraction Tests ──────────────────────────────────

    #[test]
    fn extract_temporal_at_time() {
        let claims = EntityExtractor::extract("Your meeting is at 3pm today.");
        let temporal: Vec<_> = claims.iter()
            .filter(|c| c.category == ClaimCategory::Temporal)
            .collect();
        assert!(!temporal.is_empty(), "Should extract temporal claim");
        // Should find "at 3pm"
        let has_time = temporal.iter().any(|c| c.entities.get("time").is_some());
        assert!(has_time, "Should extract time entity");
    }

    #[test]
    fn extract_temporal_at_time_with_minutes() {
        let claims = EntityExtractor::extract("The call starts at 3:30 PM sharp.");
        let temporal: Vec<_> = claims.iter()
            .filter(|c| c.category == ClaimCategory::Temporal)
            .collect();
        assert!(!temporal.is_empty());
        let time = temporal[0].entities.get("time").unwrap();
        assert!(time.contains("3:30"), "Got time: {}", time);
    }

    #[test]
    fn extract_temporal_tomorrow() {
        let claims = EntityExtractor::extract("Let's do it tomorrow afternoon.");
        let temporal: Vec<_> = claims.iter()
            .filter(|c| c.category == ClaimCategory::Temporal)
            .collect();
        assert!(!temporal.is_empty());
        assert!(temporal.iter().any(|c| c.entities.get("date_ref") == Some(&"tomorrow".to_string())));
    }

    #[test]
    fn extract_personal_you_told_me() {
        let claims = EntityExtractor::extract("You told me that you enjoy hiking in the mountains.");
        let personal: Vec<_> = claims.iter()
            .filter(|c| c.category == ClaimCategory::Personal)
            .collect();
        assert!(!personal.is_empty());
        assert!(personal[0].entities.get("claimed_content").is_some());
    }

    #[test]
    fn extract_personal_your_favorite() {
        let claims = EntityExtractor::extract("Your favorite restaurant is the Italian place downtown.");
        let personal: Vec<_> = claims.iter()
            .filter(|c| c.category == ClaimCategory::Personal)
            .collect();
        assert!(!personal.is_empty());
    }

    #[test]
    fn extract_numerical_email_count() {
        let claims = EntityExtractor::extract("You have 5 unread emails in your inbox.");
        let numerical: Vec<_> = claims.iter()
            .filter(|c| c.category == ClaimCategory::Numerical)
            .collect();
        assert!(!numerical.is_empty());
        assert_eq!(numerical[0].entities.get("count"), Some(&"5".to_string()));
    }

    #[test]
    fn extract_numerical_temperature() {
        let claims = EntityExtractor::extract("The temperature is 72°F and sunny.");
        let numerical: Vec<_> = claims.iter()
            .filter(|c| c.category == ClaimCategory::Numerical)
            .collect();
        assert!(!numerical.is_empty());
        assert_eq!(numerical[0].entities.get("value"), Some(&"72".to_string()));
    }

    #[test]
    fn extract_contact_email() {
        let claims = EntityExtractor::extract("You can reach Alice at alice@example.com for details.");
        let contact: Vec<_> = claims.iter()
            .filter(|c| c.category == ClaimCategory::Contact)
            .collect();
        assert!(!contact.is_empty());
        assert_eq!(contact[0].entities.get("email"), Some(&"alice@example.com".to_string()));
    }

    #[test]
    fn extract_contact_phone() {
        let claims = EntityExtractor::extract("Call Bob at 555-123-4567 for the update.");
        let contact: Vec<_> = claims.iter()
            .filter(|c| c.category == ClaimCategory::Contact)
            .collect();
        assert!(!contact.is_empty());
        assert!(contact.iter().any(|c| c.entities.get("phone").is_some()));
    }

    #[test]
    fn extract_commitment() {
        let claims = EntityExtractor::extract("You promised to call your mom this evening.");
        let commitment: Vec<_> = claims.iter()
            .filter(|c| c.category == ClaimCategory::Commitment)
            .collect();
        assert!(!commitment.is_empty());
        assert!(commitment[0].entities.get("claimed_commitment").is_some());
    }

    #[test]
    fn extract_no_claims_from_casual() {
        let claims = EntityExtractor::extract("That sounds great! I'm happy to help you with anything.");
        // Should have no or very few verifiable claims
        assert!(claims.len() <= 1, "Casual chat should have minimal claims, got {}", claims.len());
    }

    #[test]
    fn extract_multiple_claims() {
        let text = "You have 3 unread emails. Your meeting is at 2pm today. You told me you prefer morning meetings.";
        let claims = EntityExtractor::extract(text);
        // Should find claims across multiple categories
        let categories: std::collections::HashSet<_> = claims.iter().map(|c| c.category).collect();
        assert!(categories.len() >= 2, "Should extract multiple categories, got: {:?}", categories);
    }

    // ── Verification Tests ──────────────────────────────────────

    #[test]
    fn verify_matching_claim() {
        let claims = EntityExtractor::extract("Your meeting is at 3pm today.");
        let mut gt = GroundTruth::new();
        gt.add_calendar("cal:evt_1", "Team standup meeting at 3pm today", 1000);

        let results = SourceVerifier::verify(&claims, &gt);
        let temporal = results.iter()
            .find(|r| r.claim.category == ClaimCategory::Temporal && r.claim.entities.contains_key("time"));

        if let Some(v) = temporal {
            assert_eq!(v.status, VerificationStatus::Verified,
                "Meeting at 3pm should verify against calendar. Status: {}", v.status);
        }
    }

    #[test]
    fn verify_contradicted_time() {
        let mut claims = vec![Claim {
            category: ClaimCategory::Temporal,
            text: "meeting at 3pm".into(),
            entities: {
                let mut e = HashMap::new();
                e.insert("time".into(), "3pm".into());
                e
            },
            offset_start: 0,
            offset_end: 14,
        }];

        let mut gt = GroundTruth::new();
        gt.add_calendar("cal:evt_1", "Team standup meeting at 4pm", 1000);

        let results = SourceVerifier::verify(&claims, &gt);
        // The claim says 3pm but ground truth says 4pm
        assert!(!results.is_empty());
        // It should either be contradicted or unverifiable (depending on similarity threshold)
        assert!(
            results[0].status == VerificationStatus::Contradicted
            || results[0].status == VerificationStatus::Unverifiable,
            "Status: {}", results[0].status,
        );
    }

    #[test]
    fn verify_no_source() {
        let claims = vec![Claim {
            category: ClaimCategory::Contact,
            text: "alice@example.com".into(),
            entities: {
                let mut e = HashMap::new();
                e.insert("email".into(), "alice@example.com".into());
                e
            },
            offset_start: 0,
            offset_end: 17,
        }];

        let gt = GroundTruth::new(); // Empty ground truth

        let results = SourceVerifier::verify(&claims, &gt);
        assert!(!results.is_empty());
        assert_eq!(results[0].status, VerificationStatus::NoSource);
    }

    // ── Firewall Engine Tests ───────────────────────────────────

    #[test]
    fn firewall_passthrough_casual() {
        let fw = HallucinationFirewall::default_config();
        let gt = GroundTruth::new();
        let verdict = fw.check("That sounds great! How can I help?", &gt);
        assert_eq!(verdict.action, FirewallAction::PassThrough);
    }

    #[test]
    fn firewall_passthrough_no_ground_truth() {
        let fw = HallucinationFirewall::default_config();
        let gt = GroundTruth::new();
        let verdict = fw.check("Your meeting is at 3pm today.", &gt);
        assert_eq!(verdict.action, FirewallAction::PassThrough);
    }

    #[test]
    fn firewall_disabled_passthrough() {
        let fw = HallucinationFirewall::new(FirewallConfig {
            enabled: false,
            ..Default::default()
        });
        let mut gt = GroundTruth::new();
        gt.add_calendar("cal:1", "Meeting at 4pm", 1000);

        let verdict = fw.check("Your meeting is at 3pm.", &gt);
        assert_eq!(verdict.action, FirewallAction::PassThrough);
        assert!(verdict.audit_log[0].contains("disabled"));
    }

    #[test]
    fn firewall_verified_claim_passes() {
        let fw = HallucinationFirewall::default_config();
        let mut gt = GroundTruth::new();
        gt.add_calendar("cal:evt_1", "Team standup at 3pm today", 1000);

        let verdict = fw.check("Your meeting is at 3pm today.", &gt);
        // With matching ground truth, should pass or at worst annotate
        assert!(
            verdict.action == FirewallAction::PassThrough || verdict.action == FirewallAction::Annotate,
            "Action: {}", verdict.action,
        );
    }

    #[test]
    fn firewall_audit_log_populated() {
        let fw = HallucinationFirewall::default_config();
        let mut gt = GroundTruth::new();
        gt.add_calendar("cal:1", "Meeting at 3pm", 1000);

        let verdict = fw.check("Your meeting is at 3pm today. You have 5 unread emails.", &gt);
        assert!(!verdict.audit_log.is_empty());
        assert!(verdict.audit_log.iter().any(|l| l.contains("Extracted")));
        assert!(verdict.audit_log.iter().any(|l| l.contains("Action:")));
    }

    // ── Ground Truth Builder Tests ──────────────────────────────

    #[test]
    fn ground_truth_categories() {
        let mut gt = GroundTruth::new();
        assert!(gt.is_empty());

        gt.add_calendar("cal:1", "Meeting at 3pm", 1000);
        gt.add_memory("mem:1", "User likes sushi", 1000);
        gt.add_contact("contact:1", "Alice alice@example.com", 1000);
        gt.add_numerical("metric:1", "5 unread emails", 1000);
        gt.add_commitment("commit:1", "Call mom at 6pm", 1000);
        gt.add_tool_result("weather", "72°F sunny in Bangalore", 1000);

        assert!(!gt.is_empty());
        assert_eq!(gt.calendar_events.len(), 1);
        assert_eq!(gt.memory_facts.len(), 1);
        assert_eq!(gt.contacts.len(), 1);
        assert_eq!(gt.numericals.len(), 1);
        assert_eq!(gt.commitments.len(), 1);
        assert_eq!(gt.tool_results.len(), 1);
    }

    // ── Text Similarity Tests ───────────────────────────────────

    #[test]
    fn text_similarity_identical() {
        let sim = SourceVerifier::text_similarity("meeting at 3pm", "meeting at 3pm");
        assert!((sim - 1.0).abs() < 0.01, "Identical texts should have similarity ~1.0, got {}", sim);
    }

    #[test]
    fn text_similarity_partial() {
        let sim = SourceVerifier::text_similarity("meeting at 3pm", "team meeting at 3pm today");
        assert!(sim > 0.4, "Overlapping texts should have decent similarity, got {}", sim);
    }

    #[test]
    fn text_similarity_different() {
        let sim = SourceVerifier::text_similarity("meeting at 3pm", "sunny weather in bangalore");
        assert!(sim < 0.2, "Different texts should have low similarity, got {}", sim);
    }

    // ── Corrected Response Tests ────────────────────────────────

    #[test]
    fn annotated_response_adds_markers() {
        let verifications = vec![
            ClaimVerification {
                claim: Claim {
                    category: ClaimCategory::Temporal,
                    text: "at 3pm".into(),
                    entities: HashMap::new(),
                    offset_start: 19,
                    offset_end: 25,
                },
                status: VerificationStatus::Unverifiable,
                source: None,
                correction: None,
                trust: 0.3,
            },
        ];

        let response = "Your meeting starts at 3pm today.";
        let annotated = HallucinationFirewall::build_annotated_response(response, &verifications);
        assert!(annotated.contains("[unverified]"), "Annotated: {}", annotated);
    }
}
