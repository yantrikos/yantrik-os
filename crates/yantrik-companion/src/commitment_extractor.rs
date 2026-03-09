//! Commitment Extractor — identifies promises and action items in text.
//!
//! Uses regex-based pattern matching to detect commitments in:
//! - Chat conversations
//! - Email bodies
//! - Calendar event descriptions
//! - Meeting notes
//!
//! Extracted commitments are stored in the World Model for tracking.

use std::sync::OnceLock;
use regex::Regex;
use crate::world_model::{
    Commitment, CommitmentSource, CommitmentStatus,
};

// ── Cached regex patterns ───────────────────────────────────────────

fn re_first_person() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(
        r"(?i)\b(?:I'?ll|I\s+will|I'?m\s+going\s+to|I\s+need\s+to|I\s+have\s+to|I\s+should|I\s+must|I\s+plan\s+to|I\s+promise\s+to|let\s+me)\s+(.{10,120})"
    ).unwrap())
}

fn re_action_item() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(
        r"(?i)(?:action\s+item|todo|task|action\s*:|to.do\s*:|next\s+step)\s*:?\s+(.{5,150})"
    ).unwrap())
}

fn re_checkbox() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(
        r"(?m)^[\-\*]\s*\[\s*\]\s+(.{5,150})"
    ).unwrap())
}

fn re_request() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(
        r"(?i)(?:can\s+you|could\s+you|would\s+you|please)\s+(.{10,150})"
    ).unwrap())
}

fn re_follow_up() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(
        r"(?i)(?:let'?s\s+follow\s+up|we\s+should\s+(?:follow\s+up|revisit|circle\s+back)|don'?t\s+forget\s+to|make\s+sure\s+(?:to|we)|remind\s+me\s+to)\s+(.{5,150})"
    ).unwrap())
}

fn re_deadline_stmt() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(
        r"(?i)(.{10,100})\s+(?:by|before|no\s+later\s+than|due\s+(?:by|on)?)\s+(tomorrow|today|tonight|monday|tuesday|wednesday|thursday|friday|saturday|sunday|next\s+week|end\s+of\s+(?:day|week|month)|(?:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)\w*\s+\d{1,2})"
    ).unwrap())
}

fn re_deadline_extract() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(
        r"(?i)(?:by|before|due|until|no\s+later\s+than)\s+(tomorrow|today|tonight|monday|tuesday|wednesday|thursday|friday|saturday|sunday|next\s+week|end\s+of\s+(?:day|week|month)|this\s+(?:afternoon|evening|week)|(?:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)\w*\s+\d{1,2}(?:\s*,?\s*\d{4})?|\d{1,2}[/\-]\d{1,2})"
    ).unwrap())
}

fn re_promisee() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(
        r"(?i)(?:send|give|tell|email|message|call|update|notify|inform|show|share\s+with|get\s+back\s+to)\s+(\w+)"
    ).unwrap())
}

fn re_sender() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(
        r"(?i)(?:from|per|as\s+(?:\w+\s+)?(?:asked|requested|mentioned))\s+(\w+)"
    ).unwrap())
}

fn re_sentence_split() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[.!?]+\s+|\n+").unwrap())
}

// ── Types ───────────────────────────────────────────────────────────

/// A raw extracted commitment before storage.
#[derive(Debug, Clone)]
pub struct ExtractedCommitment {
    pub action: String,
    pub promisor: Option<String>,
    pub promisee: Option<String>,
    pub deadline_text: Option<String>,
    pub deadline_ts: f64,
    pub confidence: f64,
    pub evidence: String,
}

// ── Public API ──────────────────────────────────────────────────────

/// Extract commitments from text.
pub fn extract_commitments(
    text: &str,
    user_name: &str,
    _source: &CommitmentSource,
) -> Vec<ExtractedCommitment> {
    let mut results = Vec::new();

    for sentence in split_sentences(text) {
        let s = sentence.trim();
        if s.len() < 10 {
            continue;
        }

        if let Some(c) = match_first_person_promise(s, user_name) {
            results.push(c);
        } else if let Some(c) = match_action_item(s, user_name) {
            results.push(c);
        } else if let Some(c) = match_request_promise(s, user_name) {
            results.push(c);
        } else if let Some(c) = match_follow_up(s, user_name) {
            results.push(c);
        } else if let Some(c) = match_deadline_statement(s, user_name) {
            results.push(c);
        }
    }

    dedup_commitments(&mut results);
    results
}

/// Convert extracted commitments into World Model commitments ready for storage.
pub fn to_world_model_commitments(
    extracted: &[ExtractedCommitment],
    user_name: &str,
    source: CommitmentSource,
) -> Vec<Commitment> {
    let now = now_ts();
    extracted
        .iter()
        .map(|e| Commitment {
            id: 0,
            promisor: e.promisor.clone().unwrap_or_else(|| user_name.to_string()),
            promisee: e.promisee.clone().unwrap_or_else(|| "unknown".to_string()),
            action: e.action.clone(),
            deadline: e.deadline_ts,
            status: CommitmentStatus::Pending,
            confidence: e.confidence,
            source: source.clone(),
            evidence_text: e.evidence.clone(),
            related_entities: vec![],
            created_at: now,
            updated_at: now,
            completion_evidence: None,
        })
        .collect()
}

// ── Pattern Matchers ────────────────────────────────────────────────

fn match_first_person_promise(sentence: &str, user_name: &str) -> Option<ExtractedCommitment> {
    let caps = re_first_person().captures(sentence)?;
    let action = clean_action(caps.get(1)?.as_str().trim());
    if is_excluded_action(&action) { return None; }

    let deadline = extract_deadline(sentence);
    Some(ExtractedCommitment {
        action,
        promisor: Some(user_name.to_string()),
        promisee: extract_promisee(sentence),
        deadline_text: deadline.0,
        deadline_ts: deadline.1,
        confidence: 0.75,
        evidence: sentence.to_string(),
    })
}

fn match_action_item(sentence: &str, user_name: &str) -> Option<ExtractedCommitment> {
    let action = if let Some(caps) = re_action_item().captures(sentence) {
        caps.get(1)?.as_str().trim().to_string()
    } else if let Some(caps) = re_checkbox().captures(sentence) {
        caps.get(1)?.as_str().trim().to_string()
    } else {
        return None;
    };

    let action = clean_action(&action);
    if is_excluded_action(&action) { return None; }

    let deadline = extract_deadline(sentence);
    Some(ExtractedCommitment {
        action,
        promisor: Some(user_name.to_string()),
        promisee: None,
        deadline_text: deadline.0,
        deadline_ts: deadline.1,
        confidence: 0.85,
        evidence: sentence.to_string(),
    })
}

fn match_request_promise(sentence: &str, user_name: &str) -> Option<ExtractedCommitment> {
    let caps = re_request().captures(sentence)?;
    let action = clean_action(caps.get(1)?.as_str().trim());
    if is_excluded_action(&action) { return None; }

    let deadline = extract_deadline(sentence);
    Some(ExtractedCommitment {
        action,
        promisor: Some(user_name.to_string()),
        promisee: extract_sender(sentence).or_else(|| Some("requester".to_string())),
        deadline_text: deadline.0,
        deadline_ts: deadline.1,
        confidence: 0.60,
        evidence: sentence.to_string(),
    })
}

fn match_follow_up(sentence: &str, user_name: &str) -> Option<ExtractedCommitment> {
    let caps = re_follow_up().captures(sentence)?;
    let action = clean_action(caps.get(1)?.as_str().trim());
    if is_excluded_action(&action) { return None; }

    let deadline = extract_deadline(sentence);
    Some(ExtractedCommitment {
        action: format!("Follow up: {}", action),
        promisor: Some(user_name.to_string()),
        promisee: None,
        deadline_text: deadline.0,
        deadline_ts: deadline.1,
        confidence: 0.70,
        evidence: sentence.to_string(),
    })
}

fn match_deadline_statement(sentence: &str, user_name: &str) -> Option<ExtractedCommitment> {
    let caps = re_deadline_stmt().captures(sentence)?;
    let action = clean_action(caps.get(1)?.as_str().trim());
    let deadline_text = caps.get(2)?.as_str().trim();

    if is_excluded_action(&action) { return None; }

    Some(ExtractedCommitment {
        action,
        promisor: Some(user_name.to_string()),
        promisee: None,
        deadline_text: Some(deadline_text.to_string()),
        deadline_ts: parse_deadline_text(deadline_text),
        confidence: 0.80,
        evidence: sentence.to_string(),
    })
}

// ── Helpers ─────────────────────────────────────────────────────────

fn clean_action(raw: &str) -> String {
    let mut s = raw.to_string();
    while s.ends_with('.') || s.ends_with(',') || s.ends_with('!') || s.ends_with('?') {
        s.pop();
    }
    let lower = s.to_lowercase();
    for prefix in &["just ", "also ", "probably ", "definitely ", "certainly "] {
        if lower.starts_with(prefix) {
            s = s[prefix.len()..].to_string();
            break;
        }
    }
    s.trim().to_string()
}

fn is_excluded_action(action: &str) -> bool {
    let lower = action.to_lowercase();
    if lower.len() < 8 { return true; }

    const EXCLUDED: &[&str] = &[
        "think about it", "let you know", "see what happens", "check it out",
        "look into it", "get back to you", "keep that in mind", "take a look",
        "be there", "be right back", "be fine", "be okay", "have a good",
        "see you", "talk to you", "catch up later", "know what you mean",
        "try to remember",
    ];
    EXCLUDED.iter().any(|e| lower.contains(e))
}

fn extract_deadline(sentence: &str) -> (Option<String>, f64) {
    if let Some(caps) = re_deadline_extract().captures(sentence) {
        let text = caps.get(1).map(|m| m.as_str().to_string());
        let ts = text.as_deref().map(parse_deadline_text).unwrap_or(0.0);
        (text, ts)
    } else {
        (None, 0.0)
    }
}

fn parse_deadline_text(text: &str) -> f64 {
    let now = now_ts();
    let lower = text.to_lowercase();

    if lower.contains("today") || lower.contains("tonight") || lower.contains("end of day") {
        let day_start = (now / 86400.0).floor() * 86400.0;
        return day_start + 86399.0;
    }
    if lower.contains("tomorrow") {
        let day_start = (now / 86400.0).floor() * 86400.0;
        return day_start + 86400.0 + 64800.0;
    }
    if lower.contains("this afternoon") {
        let day_start = (now / 86400.0).floor() * 86400.0;
        return day_start + 54000.0;
    }
    if lower.contains("this evening") {
        let day_start = (now / 86400.0).floor() * 86400.0;
        return day_start + 72000.0;
    }
    if lower.contains("next week") || lower.contains("end of week") {
        return now + 7.0 * 86400.0;
    }
    if lower.contains("end of month") {
        return now + 30.0 * 86400.0;
    }
    if lower.contains("this week") {
        return now + 5.0 * 86400.0;
    }

    let days = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
    for (i, day) in days.iter().enumerate() {
        if lower.contains(day) {
            let current_day = ((now / 86400.0).floor() as i64 + 3) % 7;
            let target = i as i64;
            let days_ahead = if target > current_day {
                target - current_day
            } else {
                7 + target - current_day
            };
            return now + (days_ahead as f64) * 86400.0;
        }
    }

    0.0
}

fn extract_promisee(sentence: &str) -> Option<String> {
    let caps = re_promisee().captures(sentence)?;
    let name = caps.get(1)?.as_str();
    let lower = name.to_lowercase();
    if ["you", "me", "him", "her", "them", "it", "that", "this", "the", "a", "an"]
        .contains(&lower.as_str())
    {
        return None;
    }
    Some(capitalize(name))
}

fn extract_sender(sentence: &str) -> Option<String> {
    re_sender()
        .captures(sentence)
        .and_then(|caps| caps.get(1))
        .map(|m| capitalize(m.as_str()))
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + &chars.as_str().to_lowercase(),
    }
}

fn split_sentences(text: &str) -> Vec<String> {
    re_sentence_split()
        .split(text)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn dedup_commitments(commitments: &mut Vec<ExtractedCommitment>) {
    commitments.dedup_by(|a, b| {
        let a_lower = a.action.to_lowercase();
        let b_lower = b.action.to_lowercase();
        a_lower == b_lower || a_lower.contains(&b_lower) || b_lower.contains(&a_lower)
    });
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

    fn extract(text: &str) -> Vec<ExtractedCommitment> {
        extract_commitments(text, "Pranab", &CommitmentSource::Conversation { turn_id: None })
    }

    #[test]
    fn first_person_promises() {
        let results = extract("I'll send the quarterly report to Sarah by tomorrow");
        assert_eq!(results.len(), 1);
        assert!(results[0].action.contains("send the quarterly report"));
        assert!(results[0].confidence >= 0.7);
        assert!(results[0].deadline_ts > 0.0);
    }

    #[test]
    fn action_items() {
        let results = extract("Action item: Review the PR and merge by Friday");
        assert_eq!(results.len(), 1);
        assert!(results[0].action.contains("Review the PR"));
        assert!(results[0].confidence >= 0.8);
    }

    #[test]
    fn follow_ups() {
        let results = extract("Let's follow up on the budget proposal next week");
        assert_eq!(results.len(), 1);
        assert!(results[0].action.contains("Follow up"));
        assert!(results[0].action.contains("budget proposal"));
    }

    #[test]
    fn excludes_filler() {
        let results = extract("I'll think about it and let you know later");
        assert!(results.is_empty(), "Should exclude conversational filler");
    }

    #[test]
    fn multiple_commitments() {
        let text = "I need to finish the slides by Monday. Also, I'll email the client about the delay. Don't forget to update the tracking spreadsheet.";
        let results = extract(text);
        assert!(results.len() >= 2, "Should find multiple commitments, got {}", results.len());
    }

    #[test]
    fn deadline_parsing() {
        let results = extract("I must complete the migration before end of week");
        assert_eq!(results.len(), 1);
        assert!(results[0].deadline_ts > 0.0, "Should parse 'end of week' deadline");
    }

    #[test]
    fn request_pattern() {
        let results = extract("Can you review the design document and provide feedback");
        assert_eq!(results.len(), 1);
        assert!(results[0].confidence >= 0.5);
    }

    #[test]
    fn checkbox_items() {
        let results = extract("- [ ] Deploy the staging environment with new configs");
        assert_eq!(results.len(), 1);
        assert!(results[0].action.contains("Deploy"));
    }
}
