//! Pulse system — event capture from tool calls.
//!
//! Every tool call generates zero or more `Pulse` events. Each pulse
//! captures what happened (event type), in what system (source), and
//! references entities involved (via entity keys).
//!
//! Per-tool extractors parse tool results to generate structured pulses.

use serde_json::Value;

use super::entity::SystemSource;

// ── Core Types ───────────────────────────────────────────────────────

/// A single event captured from a tool call.
#[derive(Debug, Clone)]
pub struct Pulse {
    pub source: SystemSource,
    pub event_type: PulseType,
    pub summary: String,
    pub timestamp: f64,
    pub metadata: Value,
    /// Raw entity references before resolution.
    /// Each: (entity_key_hint, role)
    /// e.g., ("person:sarah@co.com", "actor"), ("ticket:YOS-142", "target")
    pub entity_refs: Vec<(String, String)>,
}

/// Types of events the cortex tracks.
#[derive(Debug, Clone, PartialEq)]
pub enum PulseType {
    // Jira events
    TicketViewed,
    TicketCreated,
    TicketUpdated,
    TicketAssigned,
    TicketCommented,
    TicketTransitioned,

    // Git events
    CommitPushed,
    BranchCheckedOut,
    PullRequestOpened,
    PullRequestMerged,

    // Email events
    EmailReceived,
    EmailSent,
    EmailRead,

    // Calendar events
    MeetingScheduled,
    MeetingStarting,

    // File system events
    FileEdited,
    FileCreated,

    // Browser events
    PageVisited,

    // Generic
    ToolCalled,
}

impl PulseType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TicketViewed => "ticket_viewed",
            Self::TicketCreated => "ticket_created",
            Self::TicketUpdated => "ticket_updated",
            Self::TicketAssigned => "ticket_assigned",
            Self::TicketCommented => "ticket_commented",
            Self::TicketTransitioned => "ticket_transitioned",
            Self::CommitPushed => "commit_pushed",
            Self::BranchCheckedOut => "branch_checkout",
            Self::PullRequestOpened => "pr_opened",
            Self::PullRequestMerged => "pr_merged",
            Self::EmailReceived => "email_received",
            Self::EmailSent => "email_sent",
            Self::EmailRead => "email_read",
            Self::MeetingScheduled => "meeting_scheduled",
            Self::MeetingStarting => "meeting_starting",
            Self::FileEdited => "file_edited",
            Self::FileCreated => "file_created",
            Self::PageVisited => "page_visited",
            Self::ToolCalled => "tool_called",
        }
    }

    /// Default relationship type that this pulse implies between actor and target.
    pub fn default_relationship(&self) -> Option<&'static str> {
        match self {
            Self::TicketAssigned => Some("assigned_to"),
            Self::TicketCommented => Some("commented_on"),
            Self::TicketTransitioned => Some("transitioned"),
            Self::CommitPushed => Some("committed_to"),
            Self::EmailSent => Some("emailed"),
            Self::EmailReceived => Some("emailed_by"),
            Self::MeetingScheduled | Self::MeetingStarting => Some("attends_with"),
            Self::FileEdited | Self::FileCreated => Some("works_on"),
            Self::TicketCreated => Some("created"),
            Self::TicketViewed | Self::TicketUpdated => Some("works_on"),
            _ => None,
        }
    }
}

// ── Pulse Collector ──────────────────────────────────────────────────

/// Extracts pulses from tool call results.
pub struct PulseCollector;

impl PulseCollector {
    pub fn new() -> Self {
        Self
    }

    /// Extract pulses from a tool call result.
    ///
    /// Each tool type has a specific extractor. Unknown tools are ignored.
    pub fn extract_pulses(
        &self,
        tool_name: &str,
        tool_args: &Value,
        tool_result: &str,
    ) -> Vec<Pulse> {
        let now = now_ts();
        match tool_name {
            // Jira tools
            "jira_get_issue" | "jira_read" => extract_jira_view(tool_args, tool_result, now),
            "jira_create_issue" => extract_jira_create(tool_args, tool_result, now),
            "jira_edit_issue" | "jira_write" => extract_jira_update(tool_args, tool_result, now),
            "jira_transition" => extract_jira_transition(tool_args, tool_result, now),
            "jira_comment" => extract_jira_comment(tool_args, tool_result, now),
            "jira_search" => extract_jira_search(tool_args, tool_result, now),

            // Git tools
            "run_command" => extract_from_shell(tool_args, tool_result, now),

            // Email tools
            "email_check" => extract_email_check(tool_result, now),
            "email_read" => extract_email_read(tool_args, tool_result, now),
            "email_send" | "email_reply" => extract_email_send(tool_args, tool_result, now),

            // Calendar tools
            "calendar_list" | "calendar_today" => extract_calendar(tool_result, now),

            // File tools
            "write_file" => extract_file_write(tool_args, now),
            "read_file" => extract_file_read(tool_args, now),

            // Browser tools
            "browse" | "web_search" => extract_browser(tool_args, tool_result, now),

            _ => vec![],
        }
    }
}

// ── Per-Tool Extractors ──────────────────────────────────────────────

fn extract_jira_view(args: &Value, result: &str, ts: f64) -> Vec<Pulse> {
    let key = args
        .get("issue_key")
        .or_else(|| args.get("issueIdOrKey"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if key.is_empty() {
        return vec![];
    }

    let mut refs = vec![
        (format!("ticket:{}", key), "target".to_string()),
    ];

    // Try to extract assignee from result
    if let Some(assignee) = extract_field(result, "Assignee") {
        refs.push((format!("person:{}", normalize_name(&assignee)), "context".to_string()));
    }

    // Extract status
    let status = extract_field(result, "Status").unwrap_or_default();
    let summary_text = extract_field(result, "Summary").unwrap_or_else(|| key.to_string());

    vec![Pulse {
        source: SystemSource::Jira,
        event_type: PulseType::TicketViewed,
        summary: format!("Viewed {} — {} [{}]", key, summary_text, status),
        timestamp: ts,
        metadata: serde_json::json!({
            "key": key,
            "status": status,
            "summary": summary_text,
        }),
        entity_refs: refs,
    }]
}

fn extract_jira_create(args: &Value, result: &str, ts: f64) -> Vec<Pulse> {
    let summary = args.get("summary").and_then(|v| v.as_str()).unwrap_or("new issue");
    // Try to extract created key from result
    let key = extract_ticket_key(result).unwrap_or_default();

    vec![Pulse {
        source: SystemSource::Jira,
        event_type: PulseType::TicketCreated,
        summary: format!("Created {} — {}", key, summary),
        timestamp: ts,
        metadata: serde_json::json!({"key": key, "summary": summary}),
        entity_refs: vec![
            (format!("ticket:{}", key), "target".to_string()),
        ],
    }]
}

fn extract_jira_update(args: &Value, result: &str, ts: f64) -> Vec<Pulse> {
    let key = args
        .get("issue_key")
        .or_else(|| args.get("issueIdOrKey"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if key.is_empty() {
        return vec![];
    }

    vec![Pulse {
        source: SystemSource::Jira,
        event_type: PulseType::TicketUpdated,
        summary: format!("Updated {}", key),
        timestamp: ts,
        metadata: serde_json::json!({"key": key}),
        entity_refs: vec![
            (format!("ticket:{}", key), "target".to_string()),
        ],
    }]
}

fn extract_jira_transition(args: &Value, result: &str, ts: f64) -> Vec<Pulse> {
    let key = args
        .get("issue_key")
        .or_else(|| args.get("issueIdOrKey"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let status = args.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");

    if key.is_empty() {
        return vec![];
    }

    vec![Pulse {
        source: SystemSource::Jira,
        event_type: PulseType::TicketTransitioned,
        summary: format!("{} → {}", key, status),
        timestamp: ts,
        metadata: serde_json::json!({"key": key, "new_status": status}),
        entity_refs: vec![
            (format!("ticket:{}", key), "target".to_string()),
        ],
    }]
}

fn extract_jira_comment(args: &Value, _result: &str, ts: f64) -> Vec<Pulse> {
    let key = args
        .get("issue_key")
        .or_else(|| args.get("issueIdOrKey"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if key.is_empty() {
        return vec![];
    }

    vec![Pulse {
        source: SystemSource::Jira,
        event_type: PulseType::TicketCommented,
        summary: format!("Commented on {}", key),
        timestamp: ts,
        metadata: serde_json::json!({"key": key}),
        entity_refs: vec![
            (format!("ticket:{}", key), "target".to_string()),
        ],
    }]
}

fn extract_jira_search(args: &Value, result: &str, ts: f64) -> Vec<Pulse> {
    // Extract ticket keys mentioned in search results
    let keys = extract_all_ticket_keys(result);
    if keys.is_empty() {
        return vec![];
    }

    let refs: Vec<_> = keys
        .iter()
        .take(5) // Limit to avoid explosion
        .map(|k| (format!("ticket:{}", k), "context".to_string()))
        .collect();

    vec![Pulse {
        source: SystemSource::Jira,
        event_type: PulseType::TicketViewed,
        summary: format!("Searched Jira, found {} tickets", keys.len()),
        timestamp: ts,
        metadata: serde_json::json!({"ticket_count": keys.len(), "keys": keys}),
        entity_refs: refs,
    }]
}

fn extract_from_shell(args: &Value, result: &str, ts: f64) -> Vec<Pulse> {
    let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");

    // Detect git commands
    if cmd.starts_with("git ") {
        return extract_git_command(cmd, result, ts);
    }

    vec![]
}

fn extract_git_command(cmd: &str, result: &str, ts: f64) -> Vec<Pulse> {
    let mut pulses = Vec::new();

    if cmd.contains("commit") || cmd.contains("push") {
        // Extract file entities from git output
        let files: Vec<String> = result
            .lines()
            .filter(|l| {
                l.contains("modified:") || l.contains("new file:") || l.contains("deleted:")
            })
            .filter_map(|l| l.split_whitespace().last().map(String::from))
            .take(5)
            .collect();

        let refs: Vec<_> = files
            .iter()
            .map(|f| (format!("file:{}", f), "target".to_string()))
            .collect();

        // Check for ticket references in commit message
        let ticket_refs: Vec<_> = extract_all_ticket_keys(cmd)
            .iter()
            .map(|k| (format!("ticket:{}", k), "context".to_string()))
            .collect();

        let mut all_refs = refs;
        all_refs.extend(ticket_refs);

        pulses.push(Pulse {
            source: SystemSource::Git,
            event_type: PulseType::CommitPushed,
            summary: format!("Git commit/push ({} files)", files.len()),
            timestamp: ts,
            metadata: serde_json::json!({"files": files, "cmd": cmd.chars().take(100).collect::<String>()}),
            entity_refs: all_refs,
        });
    } else if cmd.contains("checkout") || cmd.contains("switch") {
        let branch = cmd.split_whitespace().last().unwrap_or("unknown");
        // Check if branch name references a ticket
        let ticket_refs: Vec<_> = extract_all_ticket_keys(branch)
            .iter()
            .map(|k| (format!("ticket:{}", k), "context".to_string()))
            .collect();

        pulses.push(Pulse {
            source: SystemSource::Git,
            event_type: PulseType::BranchCheckedOut,
            summary: format!("Checked out branch: {}", branch),
            timestamp: ts,
            metadata: serde_json::json!({"branch": branch}),
            entity_refs: ticket_refs,
        });
    }

    pulses
}

fn extract_email_check(result: &str, ts: f64) -> Vec<Pulse> {
    // Parse email check results for new emails
    let mut pulses = Vec::new();
    for line in result.lines().take(10) {
        if let Some(from) = extract_email_sender(line) {
            let subject = extract_email_subject(line).unwrap_or_default();
            let ticket_refs: Vec<_> = extract_all_ticket_keys(line)
                .iter()
                .map(|k| (format!("ticket:{}", k), "context".to_string()))
                .collect();

            let mut refs = vec![
                (format!("person:{}", normalize_name(&from)), "actor".to_string()),
            ];
            refs.extend(ticket_refs);

            pulses.push(Pulse {
                source: SystemSource::Email,
                event_type: PulseType::EmailReceived,
                summary: format!("Email from {} — {}", from, truncate(&subject, 60)),
                timestamp: ts,
                metadata: serde_json::json!({"from": from, "subject": subject}),
                entity_refs: refs,
            });
        }
    }
    pulses
}

fn extract_email_read(args: &Value, result: &str, ts: f64) -> Vec<Pulse> {
    let id = args.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
    if let Some(from) = extract_email_sender(result) {
        let subject = extract_email_subject(result).unwrap_or_default();
        let ticket_refs: Vec<_> = extract_all_ticket_keys(result)
            .iter()
            .map(|k| (format!("ticket:{}", k), "context".to_string()))
            .collect();

        let mut refs = vec![
            (format!("person:{}", normalize_name(&from)), "actor".to_string()),
        ];
        refs.extend(ticket_refs);

        return vec![Pulse {
            source: SystemSource::Email,
            event_type: PulseType::EmailRead,
            summary: format!("Read email from {} — {}", from, truncate(&subject, 60)),
            timestamp: ts,
            metadata: serde_json::json!({"from": from, "subject": subject, "email_id": id}),
            entity_refs: refs,
        }];
    }
    vec![]
}

fn extract_email_send(args: &Value, _result: &str, ts: f64) -> Vec<Pulse> {
    let to = args.get("to").and_then(|v| v.as_str()).unwrap_or("");
    let subject = args.get("subject").and_then(|v| v.as_str()).unwrap_or("");
    if to.is_empty() {
        return vec![];
    }

    let ticket_refs: Vec<_> = extract_all_ticket_keys(subject)
        .iter()
        .map(|k| (format!("ticket:{}", k), "context".to_string()))
        .collect();

    let mut refs = vec![
        (format!("person:{}", normalize_name(to)), "target".to_string()),
    ];
    refs.extend(ticket_refs);

    vec![Pulse {
        source: SystemSource::Email,
        event_type: PulseType::EmailSent,
        summary: format!("Sent email to {} — {}", to, truncate(subject, 60)),
        timestamp: ts,
        metadata: serde_json::json!({"to": to, "subject": subject}),
        entity_refs: refs,
    }]
}

fn extract_calendar(result: &str, ts: f64) -> Vec<Pulse> {
    // Calendar results are typically pre-formatted text
    // Look for meeting patterns
    let mut pulses = Vec::new();
    for line in result.lines().take(10) {
        // Simple heuristic: lines with time patterns are likely events
        if line.contains(':') && (line.contains("AM") || line.contains("PM") || line.contains("UTC")) {
            let summary = line.trim().chars().take(100).collect::<String>();
            pulses.push(Pulse {
                source: SystemSource::Calendar,
                event_type: PulseType::MeetingScheduled,
                summary: format!("Calendar: {}", summary),
                timestamp: ts,
                metadata: serde_json::json!({"raw": summary}),
                entity_refs: vec![],
            });
        }
    }
    pulses
}

fn extract_file_write(args: &Value, ts: f64) -> Vec<Pulse> {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    if path.is_empty() {
        return vec![];
    }

    let ticket_refs: Vec<_> = extract_all_ticket_keys(path)
        .iter()
        .map(|k| (format!("ticket:{}", k), "context".to_string()))
        .collect();

    let mut refs = vec![
        (format!("file:{}", path), "target".to_string()),
    ];
    refs.extend(ticket_refs);

    vec![Pulse {
        source: SystemSource::FileSystem,
        event_type: PulseType::FileEdited,
        summary: format!("Wrote file: {}", truncate(path, 80)),
        timestamp: ts,
        metadata: serde_json::json!({"path": path}),
        entity_refs: refs,
    }]
}

fn extract_file_read(args: &Value, ts: f64) -> Vec<Pulse> {
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    if path.is_empty() {
        return vec![];
    }

    // Reading a file is lower signal than writing, but still useful
    vec![Pulse {
        source: SystemSource::FileSystem,
        event_type: PulseType::FileEdited,
        summary: format!("Read file: {}", truncate(path, 80)),
        timestamp: ts,
        metadata: serde_json::json!({"path": path, "read_only": true}),
        entity_refs: vec![
            (format!("file:{}", path), "target".to_string()),
        ],
    }]
}

fn extract_browser(args: &Value, result: &str, ts: f64) -> Vec<Pulse> {
    let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");

    let summary = if !query.is_empty() {
        format!("Web search: {}", truncate(query, 60))
    } else if !url.is_empty() {
        format!("Browsed: {}", truncate(url, 60))
    } else {
        return vec![];
    };

    // Extract ticket refs from search results
    let ticket_refs: Vec<_> = extract_all_ticket_keys(result)
        .iter()
        .map(|k| (format!("ticket:{}", k), "context".to_string()))
        .collect();

    vec![Pulse {
        source: SystemSource::Browser,
        event_type: PulseType::PageVisited,
        summary,
        timestamp: ts,
        metadata: serde_json::json!({"url": url, "query": query}),
        entity_refs: ticket_refs,
    }]
}

// ── Parsing Helpers ──────────────────────────────────────────────────

/// Extract a Jira ticket key (e.g., YOS-142) from text.
fn extract_ticket_key(text: &str) -> Option<String> {
    extract_all_ticket_keys(text).into_iter().next()
}

/// Extract all Jira ticket keys from text.
fn extract_all_ticket_keys(text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        // Look for pattern: UPPERCASE letters followed by '-' followed by digits
        if bytes[i].is_ascii_uppercase() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_uppercase() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'-' {
                let prefix_end = i;
                i += 1;
                let digit_start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i > digit_start && prefix_end - start >= 2 {
                    let key = &text[start..i];
                    if !keys.contains(&key.to_string()) {
                        keys.push(key.to_string());
                    }
                }
            }
        } else {
            i += 1;
        }
    }
    keys
}

/// Extract a field value from a structured text result.
/// Looks for patterns like "Field: value" or "Field:    value"
fn extract_field(text: &str, field_name: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(field_name) {
            if let Some(value) = rest.strip_prefix(':') {
                let v = value.trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Extract email sender from text.
fn extract_email_sender(text: &str) -> Option<String> {
    extract_field(text, "From")
        .or_else(|| extract_field(text, "from"))
}

/// Extract email subject from text.
fn extract_email_subject(text: &str) -> Option<String> {
    extract_field(text, "Subject")
        .or_else(|| extract_field(text, "subject"))
}

/// Normalize a person name/email for entity ID.
fn normalize_name(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .replace(' ', "-")
        .replace(['<', '>', '"', '\''], "")
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.min(s.len())])
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_ticket_keys() {
        assert_eq!(extract_all_ticket_keys("Working on YOS-142"), vec!["YOS-142"]);
        assert_eq!(
            extract_all_ticket_keys("Tickets: YOS-142, PROJ-99, AB-1"),
            vec!["YOS-142", "PROJ-99", "AB-1"]
        );
        assert_eq!(extract_all_ticket_keys("no tickets here"), Vec::<String>::new());
        assert_eq!(extract_all_ticket_keys("A-1"), Vec::<String>::new()); // Too short prefix
    }

    #[test]
    fn test_normalize_name() {
        assert_eq!(normalize_name("Sarah Chen"), "sarah-chen");
        assert_eq!(normalize_name("<sarah@co.com>"), "sarah@co.com");
    }

    #[test]
    fn test_extract_field() {
        let text = "Key: YOS-142\nStatus: In Progress\nAssignee: Sarah Chen";
        assert_eq!(extract_field(text, "Status"), Some("In Progress".into()));
        assert_eq!(extract_field(text, "Assignee"), Some("Sarah Chen".into()));
        assert_eq!(extract_field(text, "Missing"), None);
    }
}
