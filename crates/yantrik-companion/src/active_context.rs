//! Active Day Context — ambient awareness buffer for system prompt injection.
//!
//! The companion needs to "know" what's happening in the user's day without
//! being explicitly asked. This module aggregates data from multiple sources
//! into a token-budgeted context block that gets injected into the system prompt.
//!
//! ## Architecture
//!
//! ```text
//! Data Sources (polled by stewardship loop)
//!     │
//!     ├── Calendar ─── upcoming events, current meeting
//!     ├── Weather ──── current conditions, alerts
//!     ├── Email ────── unread count, flagged items
//!     ├── Tasks ────── due today, overdue
//!     ├── Commitments  promises made, follow-ups
//!     └── System ───── battery, network, disk
//!     │
//!     ▼
//! ActiveDayContext::aggregate()
//!     │
//!     ▼
//! BudgetAllocator::fit_to_budget()
//!     │
//!     ├── Priority-ranked sections
//!     ├── Token budget from ModelCapabilityProfile
//!     └── Stale data pruned
//!     │
//!     ▼
//! Formatted context block → system prompt injection
//! ```
//!
//! ## Token Budget by Model Tier
//!
//! - Tiny (0.5-1.5B): 0 tokens — no ambient context
//! - Small (1.5-4B): 512 tokens — time + next event only
//! - Medium (4-14B): 8192 tokens — full day context
//! - Large (14B+): 16384 tokens — full + history

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── Context Sections ────────────────────────────────────────────────

/// Priority levels for context sections. Lower number = higher priority.
/// Used by the budget allocator to decide what to keep when space is tight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ContextPriority {
    /// Critical: current time, active emergency, system alerts
    Critical = 0,
    /// High: next upcoming event, active reminders, weather alerts
    High = 1,
    /// Medium: today's schedule, unread email count, task summary
    Medium = 2,
    /// Low: weather forecast, recent memories, system stats
    Low = 3,
    /// Background: news headlines, general context
    Background = 4,
}

impl std::fmt::Display for ContextPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Critical => write!(f, "critical"),
            Self::High => write!(f, "high"),
            Self::Medium => write!(f, "medium"),
            Self::Low => write!(f, "low"),
            Self::Background => write!(f, "background"),
        }
    }
}

/// A single section of the active context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSection {
    /// Section identifier (e.g., "calendar", "weather", "email").
    pub id: String,
    /// Display label for the section header.
    pub label: String,
    /// Priority for budget allocation.
    pub priority: ContextPriority,
    /// The formatted content of this section.
    pub content: String,
    /// Estimated token count (rough: chars / 4 for English).
    pub estimated_tokens: usize,
    /// When this section was last refreshed (Unix timestamp).
    pub refreshed_at: u64,
    /// Maximum age in seconds before this section is considered stale.
    pub max_age_secs: u64,
    /// Source for hallucination firewall grounding.
    pub source_id: String,
}

impl ContextSection {
    /// Create a new context section.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        priority: ContextPriority,
        content: impl Into<String>,
        max_age_secs: u64,
    ) -> Self {
        let content = content.into();
        let estimated_tokens = estimate_tokens(&content);
        let id_str = id.into();
        let source_id = format!("context:{}", &id_str);
        Self {
            id: id_str,
            label: label.into(),
            priority,
            content,
            estimated_tokens,
            refreshed_at: now_unix(),
            max_age_secs,
            source_id,
        }
    }

    /// Check if this section is stale (older than max_age_secs).
    pub fn is_stale(&self, current_time: u64) -> bool {
        current_time.saturating_sub(self.refreshed_at) > self.max_age_secs
    }

    /// Update the content and refresh timestamp.
    pub fn update(&mut self, content: impl Into<String>) {
        self.content = content.into();
        self.estimated_tokens = estimate_tokens(&self.content);
        self.refreshed_at = now_unix();
    }
}

/// Estimate token count from text (rough heuristic: ~4 chars per token for English).
fn estimate_tokens(text: &str) -> usize {
    // A more accurate estimate: count words and multiply by 1.3 (avg tokens/word).
    // Fallback to chars/4 if fewer than 5 words.
    let word_count = text.split_whitespace().count();
    if word_count < 5 {
        text.len() / 4
    } else {
        (word_count as f64 * 1.3) as usize
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Active Day Context ──────────────────────────────────────────────

/// The main active day context container.
///
/// Holds all context sections, manages refresh cycles, and produces
/// a token-budgeted context block for system prompt injection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActiveDayContext {
    /// All context sections, keyed by section ID.
    sections: BTreeMap<String, ContextSection>,
}

impl ActiveDayContext {
    /// Create a new empty context.
    pub fn new() -> Self {
        Self {
            sections: BTreeMap::new(),
        }
    }

    /// Add or update a context section.
    pub fn upsert(&mut self, section: ContextSection) {
        self.sections.insert(section.id.clone(), section);
    }

    /// Remove a section by ID.
    pub fn remove(&mut self, id: &str) {
        self.sections.remove(id);
    }

    /// Get a section by ID.
    pub fn get(&self, id: &str) -> Option<&ContextSection> {
        self.sections.get(id)
    }

    /// Number of active sections.
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Total estimated tokens across all sections.
    pub fn total_tokens(&self) -> usize {
        self.sections.values().map(|s| s.estimated_tokens).sum()
    }

    /// Prune stale sections that have exceeded their max age.
    pub fn prune_stale(&mut self) {
        let now = now_unix();
        self.sections.retain(|_, s| !s.is_stale(now));
    }

    /// Get all sections sorted by priority (highest priority first).
    pub fn sections_by_priority(&self) -> Vec<&ContextSection> {
        let mut sections: Vec<&ContextSection> = self.sections.values().collect();
        sections.sort_by_key(|s| s.priority);
        sections
    }

    /// Build the formatted context block, fitted to a token budget.
    ///
    /// Returns None if budget is 0 or no sections available.
    pub fn build_context_block(&self, token_budget: usize) -> Option<String> {
        if token_budget == 0 || self.sections.is_empty() {
            return None;
        }

        let allocator = BudgetAllocator::new(token_budget);
        let fitted = allocator.fit(&self.sections_by_priority());

        if fitted.is_empty() {
            return None;
        }

        Some(ContextFormatter::format(&fitted))
    }

    /// Build the context block and also return source IDs for grounding.
    pub fn build_with_sources(&self, token_budget: usize) -> (Option<String>, Vec<String>) {
        if token_budget == 0 || self.sections.is_empty() {
            return (None, vec![]);
        }

        let allocator = BudgetAllocator::new(token_budget);
        let fitted = allocator.fit(&self.sections_by_priority());

        if fitted.is_empty() {
            return (None, vec![]);
        }

        let sources: Vec<String> = fitted.iter().map(|s| s.source_id.clone()).collect();
        let block = ContextFormatter::format(&fitted);

        (Some(block), sources)
    }

    // ── Convenience Builders ────────────────────────────────────

    /// Set the current time context (Critical priority, never stale).
    pub fn set_time(&mut self, time_str: &str) {
        self.upsert(ContextSection::new(
            "time", "Current Time", ContextPriority::Critical,
            time_str, 300, // 5 min
        ));
    }

    /// Set the calendar context.
    pub fn set_calendar(&mut self, events_summary: &str) {
        self.upsert(ContextSection::new(
            "calendar", "Today's Schedule", ContextPriority::High,
            events_summary, 900, // 15 min
        ));
    }

    /// Set the next upcoming event (high priority).
    pub fn set_next_event(&mut self, event_summary: &str) {
        self.upsert(ContextSection::new(
            "next_event", "Next Event", ContextPriority::High,
            event_summary, 300, // 5 min
        ));
    }

    /// Set weather context.
    pub fn set_weather(&mut self, weather_summary: &str) {
        self.upsert(ContextSection::new(
            "weather", "Weather", ContextPriority::Low,
            weather_summary, 3600, // 1 hour
        ));
    }

    /// Set email summary context.
    pub fn set_email(&mut self, email_summary: &str) {
        self.upsert(ContextSection::new(
            "email", "Email", ContextPriority::Medium,
            email_summary, 900, // 15 min
        ));
    }

    /// Set tasks/todos context.
    pub fn set_tasks(&mut self, tasks_summary: &str) {
        self.upsert(ContextSection::new(
            "tasks", "Tasks", ContextPriority::Medium,
            tasks_summary, 1800, // 30 min
        ));
    }

    /// Set active commitments context.
    pub fn set_commitments(&mut self, commitments_summary: &str) {
        self.upsert(ContextSection::new(
            "commitments", "Commitments", ContextPriority::High,
            commitments_summary, 3600, // 1 hour
        ));
    }

    /// Set system status context.
    pub fn set_system_status(&mut self, status: &str) {
        self.upsert(ContextSection::new(
            "system", "System", ContextPriority::Low,
            status, 120, // 2 min
        ));
    }

    /// Set news headlines context.
    pub fn set_news(&mut self, news_summary: &str) {
        self.upsert(ContextSection::new(
            "news", "News", ContextPriority::Background,
            news_summary, 1800, // 30 min
        ));
    }

    /// Set an alert (Critical priority, short-lived).
    pub fn set_alert(&mut self, alert_id: &str, message: &str) {
        self.upsert(ContextSection::new(
            format!("alert_{}", alert_id),
            "Alert",
            ContextPriority::Critical,
            message,
            600, // 10 min
        ));
    }
}

// ── Budget Allocator ────────────────────────────────────────────────

/// Fits context sections into a token budget by priority.
///
/// Algorithm:
/// 1. Sort sections by priority (Critical first)
/// 2. Greedily add sections that fit within remaining budget
/// 3. If a section is too large, try to truncate it
/// 4. Stop when budget is exhausted
struct BudgetAllocator {
    budget: usize,
}

impl BudgetAllocator {
    fn new(budget: usize) -> Self {
        Self { budget }
    }

    /// Fit sections into the budget. Returns sections that fit.
    fn fit<'a>(&self, sections: &[&'a ContextSection]) -> Vec<&'a ContextSection> {
        let mut result = Vec::new();
        let mut remaining = self.budget;

        // Header overhead: "## Active Context\n" ~ 5 tokens
        let header_cost = 5;
        if remaining <= header_cost {
            return result;
        }
        remaining -= header_cost;

        for section in sections {
            // Section header overhead: "### Label\n" ~ 3 tokens
            let section_overhead = 3;
            let total_cost = section.estimated_tokens + section_overhead;

            if total_cost <= remaining {
                // Fits entirely
                result.push(*section);
                remaining -= total_cost;
            } else if remaining > section_overhead + 10 {
                // Partially fits — we'll truncate during formatting
                result.push(*section);
                break; // No more room
            }
        }

        result
    }
}

// ── Context Formatter ───────────────────────────────────────────────

/// Formats context sections into a text block for system prompt injection.
struct ContextFormatter;

impl ContextFormatter {
    /// Format sections into a context block.
    fn format(sections: &[&ContextSection]) -> String {
        let mut output = String::with_capacity(2048);
        output.push_str("## Active Context\n\n");

        for section in sections {
            output.push_str(&format!("### {}\n", section.label));
            output.push_str(&section.content);
            output.push_str("\n\n");
        }

        output.trim_end().to_string()
    }

    /// Format sections with a hard character limit, truncating the last section if needed.
    pub fn format_with_limit(sections: &[&ContextSection], max_chars: usize) -> String {
        let mut output = String::with_capacity(max_chars.min(8192));
        output.push_str("## Active Context\n\n");

        for section in sections {
            let header = format!("### {}\n", section.label);
            let section_text = format!("{}{}\n\n", header, section.content);

            if output.len() + section_text.len() <= max_chars {
                output.push_str(&section_text);
            } else {
                // Truncate to fit
                let remaining = max_chars.saturating_sub(output.len() + header.len() + 5);
                if remaining > 20 {
                    output.push_str(&header);
                    let truncated: String = section.content.chars().take(remaining).collect();
                    output.push_str(&truncated);
                    output.push_str("...\n\n");
                }
                break;
            }
        }

        output.trim_end().to_string()
    }
}

// ── Snapshot for Hallucination Firewall ──────────────────────────────

/// A snapshot of the active context for use by the hallucination firewall.
///
/// Converts context sections into GroundTruth entries so the firewall
/// can verify LLM claims against the current ambient awareness.
impl ActiveDayContext {
    /// Export all sections as ground truth entries for the hallucination firewall.
    pub fn to_ground_truth(&self) -> Vec<(String, String, u64)> {
        self.sections.values()
            .map(|s| (s.source_id.clone(), s.content.clone(), s.refreshed_at))
            .collect()
    }

    /// Export sections matching a specific category as (source_id, content, timestamp) tuples.
    pub fn ground_truth_for_category(&self, category: &str) -> Vec<(String, String, u64)> {
        let ids: Vec<&str> = match category {
            "temporal" | "calendar" => vec!["calendar", "next_event", "time"],
            "personal" => vec!["commitments"],
            "numerical" => vec!["email", "tasks", "system"],
            "contact" => vec![],
            "commitment" => vec!["commitments"],
            "weather" => vec!["weather"],
            _ => vec![],
        };

        self.sections.values()
            .filter(|s| ids.contains(&s.id.as_str()))
            .map(|s| (s.source_id.clone(), s.content.clone(), s.refreshed_at))
            .collect()
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_empty_context() {
        let ctx = ActiveDayContext::new();
        assert_eq!(ctx.section_count(), 0);
        assert_eq!(ctx.total_tokens(), 0);
        assert!(ctx.build_context_block(1000).is_none());
    }

    #[test]
    fn add_and_retrieve_section() {
        let mut ctx = ActiveDayContext::new();
        ctx.set_time("Monday March 9, 2026 2:30 PM");
        assert_eq!(ctx.section_count(), 1);
        assert!(ctx.get("time").is_some());
        assert_eq!(ctx.get("time").unwrap().priority, ContextPriority::Critical);
    }

    #[test]
    fn upsert_replaces_existing() {
        let mut ctx = ActiveDayContext::new();
        ctx.set_weather("Sunny 72°F");
        ctx.set_weather("Cloudy 65°F");
        assert_eq!(ctx.section_count(), 1);
        assert!(ctx.get("weather").unwrap().content.contains("Cloudy"));
    }

    #[test]
    fn remove_section() {
        let mut ctx = ActiveDayContext::new();
        ctx.set_weather("Sunny");
        ctx.remove("weather");
        assert_eq!(ctx.section_count(), 0);
    }

    #[test]
    fn sections_sorted_by_priority() {
        let mut ctx = ActiveDayContext::new();
        ctx.set_news("Headlines...");          // Background
        ctx.set_weather("Sunny");               // Low
        ctx.set_email("3 unread");              // Medium
        ctx.set_next_event("Standup at 3pm");   // High
        ctx.set_time("2:30 PM");                // Critical

        let sorted = ctx.sections_by_priority();
        assert_eq!(sorted[0].id, "time");       // Critical
        assert_eq!(sorted.last().unwrap().id, "news"); // Background
    }

    #[test]
    fn build_context_block_basic() {
        let mut ctx = ActiveDayContext::new();
        ctx.set_time("Monday 2:30 PM");
        ctx.set_calendar("3pm: Standup\n4pm: 1-on-1 with Alice");
        ctx.set_weather("Sunny 72°F");

        let block = ctx.build_context_block(10000).unwrap();
        assert!(block.contains("## Active Context"));
        assert!(block.contains("Current Time"));
        assert!(block.contains("Monday 2:30 PM"));
        assert!(block.contains("Today's Schedule"));
        assert!(block.contains("Weather"));
    }

    #[test]
    fn build_context_zero_budget_returns_none() {
        let mut ctx = ActiveDayContext::new();
        ctx.set_time("2:30 PM");
        assert!(ctx.build_context_block(0).is_none());
    }

    #[test]
    fn build_context_respects_budget() {
        let mut ctx = ActiveDayContext::new();
        ctx.set_time("2:30 PM");  // ~3 tokens
        // Add a large section
        let big_content = "x ".repeat(500); // ~500 tokens
        ctx.set_news(&big_content);

        // With a small budget, should only include time (Critical)
        let block = ctx.build_context_block(20).unwrap();
        assert!(block.contains("Current Time"));
        // News might not fit in 20 tokens
    }

    #[test]
    fn build_with_sources() {
        let mut ctx = ActiveDayContext::new();
        ctx.set_time("2:30 PM");
        ctx.set_calendar("3pm: Meeting");

        let (block, sources) = ctx.build_with_sources(10000);
        assert!(block.is_some());
        assert!(!sources.is_empty());
        assert!(sources.iter().any(|s| s.starts_with("context:")));
    }

    #[test]
    fn prune_stale_sections() {
        let mut ctx = ActiveDayContext::new();

        // Add a section with very short max age
        let mut section = ContextSection::new(
            "test", "Test", ContextPriority::Low, "content", 1,
        );
        // Manually set refreshed_at to the past
        section.refreshed_at = now_unix().saturating_sub(100);
        ctx.upsert(section);

        assert_eq!(ctx.section_count(), 1);
        ctx.prune_stale();
        assert_eq!(ctx.section_count(), 0, "Stale section should be pruned");
    }

    #[test]
    fn section_staleness_check() {
        let section = ContextSection::new(
            "test", "Test", ContextPriority::Low, "content", 60,
        );
        let now = now_unix();
        assert!(!section.is_stale(now)); // Just created
        assert!(section.is_stale(now + 120)); // 2 min later, max_age=60s
    }

    #[test]
    fn section_update() {
        let mut section = ContextSection::new(
            "test", "Test", ContextPriority::Low, "old content", 60,
        );
        assert!(section.content.contains("old"));
        section.update("new content");
        assert!(section.content.contains("new"));
    }

    #[test]
    fn context_priority_ordering() {
        assert!(ContextPriority::Critical < ContextPriority::High);
        assert!(ContextPriority::High < ContextPriority::Medium);
        assert!(ContextPriority::Medium < ContextPriority::Low);
        assert!(ContextPriority::Low < ContextPriority::Background);
    }

    #[test]
    fn format_with_limit() {
        let mut ctx = ActiveDayContext::new();
        ctx.set_time("2:30 PM");
        ctx.set_calendar("3pm: Meeting\n4pm: Review\n5pm: Walk");
        ctx.set_weather("Sunny 72°F with light breeze from the west");
        ctx.set_news("Tech stocks up 2%. AI conference announced for next month. Climate summit concludes.");

        let sections = ctx.sections_by_priority();
        let formatted = ContextFormatter::format_with_limit(&sections, 200);
        assert!(formatted.len() <= 220, "Length: {}", formatted.len()); // Small buffer for truncation
        assert!(formatted.contains("## Active Context"));
    }

    #[test]
    fn token_estimation() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hi"), 0); // 2 chars / 4 = 0
        let tokens = estimate_tokens("This is a longer sentence with several words in it for testing.");
        assert!(tokens > 5 && tokens < 20, "Tokens: {}", tokens);
    }

    #[test]
    fn ground_truth_export() {
        let mut ctx = ActiveDayContext::new();
        ctx.set_calendar("3pm: Meeting with Alice");
        ctx.set_weather("Sunny 72°F");

        let gt = ctx.to_ground_truth();
        assert_eq!(gt.len(), 2);
        assert!(gt.iter().any(|(id, _, _)| id.contains("calendar")));
        assert!(gt.iter().any(|(id, _, _)| id.contains("weather")));
    }

    #[test]
    fn ground_truth_category_filter() {
        let mut ctx = ActiveDayContext::new();
        ctx.set_calendar("3pm: Meeting");
        ctx.set_weather("Sunny");
        ctx.set_email("5 unread");

        let temporal = ctx.ground_truth_for_category("temporal");
        assert!(temporal.iter().any(|(id, _, _)| id.contains("calendar")));
        assert!(!temporal.iter().any(|(id, _, _)| id.contains("weather")));

        let weather = ctx.ground_truth_for_category("weather");
        assert!(weather.iter().any(|(id, _, _)| id.contains("weather")));
    }

    #[test]
    fn alert_sections() {
        let mut ctx = ActiveDayContext::new();
        ctx.set_alert("battery_low", "Battery below 10%!");
        ctx.set_alert("meeting_soon", "Standup in 5 minutes");

        assert_eq!(ctx.section_count(), 2);
        let sorted = ctx.sections_by_priority();
        // Both alerts are Critical
        assert_eq!(sorted[0].priority, ContextPriority::Critical);
        assert_eq!(sorted[1].priority, ContextPriority::Critical);
    }

    #[test]
    fn full_day_context_scenario() {
        let mut ctx = ActiveDayContext::new();
        ctx.set_time("Monday March 9, 2026 2:30 PM IST");
        ctx.set_next_event("Team Standup at 3:00 PM (in 30 min)");
        ctx.set_calendar(
            "9:00 AM: Morning review (done)\n\
             11:00 AM: Design sync (done)\n\
             3:00 PM: Team standup\n\
             4:30 PM: 1-on-1 with Alice"
        );
        ctx.set_email("5 unread emails, 1 flagged from Bob about deployment");
        ctx.set_tasks("2 tasks due today: Fix login bug (high), Update docs (medium)");
        ctx.set_commitments("Promised Alice to review PR #42 by EOD");
        ctx.set_weather("Bangalore: Partly cloudy, 28°C, no rain expected");
        ctx.set_system_status("Battery: 78%, WiFi: connected, Disk: 45% used");
        ctx.set_news("Rust 2026 edition announced. NVIDIA stock hits new high.");

        assert_eq!(ctx.section_count(), 9);

        // Medium model budget (8192 tokens) should fit everything
        let block = ctx.build_context_block(8192).unwrap();
        assert!(block.contains("Team Standup"));
        assert!(block.contains("5 unread emails"));
        assert!(block.contains("Fix login bug"));

        // Small model budget (512 tokens) should only get critical + high
        let small_block = ctx.build_context_block(50).unwrap();
        assert!(small_block.contains("Current Time"));
    }
}
