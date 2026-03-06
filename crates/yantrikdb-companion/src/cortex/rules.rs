//! Rules engine — heuristic attention detection.
//!
//! Pure Rust rules that detect situations worth the user's attention.
//! Each rule checks the entity graph and pulse history without any LLM calls.
//! Rules output `AttentionItem`s that get packaged into a situation briefing.
//!
//! 7 built-in rules:
//! 1. StaleTicket — ticket in progress too long with few commits
//! 2. MeetingPrep — upcoming meeting with people who have open tickets
//! 3. EmailTicketLink — email mentions a ticket key
//! 4. EndOfDaySummary — summarize the day's work
//! 5. BlockerAlert — your ticket depends on a stale ticket
//! 6. ReviewReminder — PR/review pending too long
//! 7. SprintPulse — weekly sprint health check

use std::collections::HashMap;

use rusqlite::Connection;

use super::focus::FocusContext;
use super::schema;

// ── Core Types ───────────────────────────────────────────────────────

/// Something that deserves the user's attention.
#[derive(Debug, Clone)]
pub struct AttentionItem {
    pub rule_name: &'static str,
    pub priority: f64,
    pub summary: String,
    pub suggested_action: String,
    pub entity_ids: Vec<String>,
    pub systems_involved: Vec<&'static str>,
}

// ── Rule Engine ──────────────────────────────────────────────────────

/// Evaluates all rules against the current context.
pub struct RuleEngine {
    /// Cooldown: rule_name → last_fired_ts
    cooldowns: HashMap<String, f64>,
    /// Services the user has enabled (e.g., "jira", "git", "email", "calendar").
    enabled_services: std::collections::HashSet<String>,
}

impl RuleEngine {
    pub fn new() -> Self {
        Self {
            cooldowns: HashMap::new(),
            enabled_services: std::collections::HashSet::new(),
        }
    }

    /// Create with a set of enabled services.
    pub fn with_services(services: &[String]) -> Self {
        Self {
            cooldowns: HashMap::new(),
            enabled_services: services.iter().map(|s| s.to_lowercase()).collect(),
        }
    }

    /// Replace enabled services (used by Skill Store integration).
    pub fn set_services(&mut self, services: &[String]) {
        self.enabled_services = services.iter().map(|s| s.to_lowercase()).collect();
    }

    fn has(&self, service: &str) -> bool {
        self.enabled_services.contains(service)
    }

    /// Evaluate all rules. Returns attention items for rules that fire.
    /// Rules are skipped if their required services are not enabled.
    pub fn evaluate(
        &mut self,
        conn: &Connection,
        focus: Option<&FocusContext>,
    ) -> Vec<AttentionItem> {
        let now = now_ts();
        let mut items = Vec::new();

        // Jira-dependent rules — only run if "jira" service is enabled
        if self.has("jira") {
            self.try_rule("stale_ticket", 3600.0, now, || {
                rule_stale_ticket(conn, focus)
            })
            .into_iter()
            .for_each(|i| items.push(i));

            self.try_rule("blocker_alert", 7200.0, now, || {
                rule_blocker_alert(conn, focus)
            })
            .into_iter()
            .for_each(|i| items.push(i));

            self.try_rule("review_reminder", 7200.0, now, || {
                rule_review_reminder(conn)
            })
            .into_iter()
            .for_each(|i| items.push(i));

            self.try_rule("sprint_pulse", 86400.0, now, || {
                rule_sprint_pulse(conn)
            })
            .into_iter()
            .for_each(|i| items.push(i));
        }

        // Calendar-dependent rules
        if self.has("calendar") {
            self.try_rule("meeting_prep", 1800.0, now, || {
                rule_meeting_prep(conn)
            })
            .into_iter()
            .for_each(|i| items.push(i));
        }

        // Email + Jira cross-link rule
        if self.has("email") && self.has("jira") {
            self.try_rule("email_ticket_link", 600.0, now, || {
                rule_email_ticket_link(conn)
            })
            .into_iter()
            .for_each(|i| items.push(i));
        }

        // End of day summary — works with whatever services are available
        if self.has("git") || self.has("jira") {
            self.try_rule("end_of_day", 43200.0, now, || {
                rule_end_of_day(conn)
            })
            .into_iter()
            .for_each(|i| items.push(i));
        }

        // Sort by priority (highest first)
        items.sort_by(|a, b| b.priority.partial_cmp(&a.priority).unwrap_or(std::cmp::Ordering::Equal));

        // Return top 3 at most
        items.truncate(3);
        items
    }

    /// Try running a rule if it's not on cooldown.
    fn try_rule<F>(
        &mut self,
        name: &str,
        cooldown_secs: f64,
        now: f64,
        rule_fn: F,
    ) -> Vec<AttentionItem>
    where
        F: FnOnce() -> Vec<AttentionItem>,
    {
        let last = self.cooldowns.get(name).copied().unwrap_or(0.0);
        if now - last < cooldown_secs {
            return vec![];
        }

        let items = rule_fn();
        if !items.is_empty() {
            self.cooldowns.insert(name.to_string(), now);
        }
        items
    }
}

// ── Rule Implementations ─────────────────────────────────────────────

/// Rule 1: Stale Ticket
///
/// Fires when a ticket has been "in progress" for >5 days with <3 commits.
/// Higher priority if the user is currently working on related files.
fn rule_stale_ticket(conn: &Connection, focus: Option<&FocusContext>) -> Vec<AttentionItem> {
    let now = now_ts();
    let five_days_ago = now - 5.0 * 86400.0;

    // Find ticket entities with recent "works_on" or "transitioned" activity
    let tickets = schema::get_relevant_entities(conn, 0.1, 20);
    let mut items = Vec::new();

    for entity in &tickets {
        if entity.entity_type != "ticket" {
            continue;
        }

        // Check for "in progress" status from attributes or pulses
        let recent_transitions = schema::count_recent_pulses(
            conn,
            &entity.id,
            "ticket_transitioned",
            five_days_ago,
        );

        // Check for commits related to this ticket
        let commit_count = schema::count_recent_pulses(
            conn,
            &entity.id,
            "commit_pushed",
            five_days_ago,
        );

        // Check the ticket has been viewed/worked on recently (it's relevant)
        let view_count = schema::count_recent_pulses(
            conn,
            &entity.id,
            "ticket_viewed",
            now - 7.0 * 86400.0,
        );

        // Fire if: ticket is relevant (viewed recently) + few commits + no recent transitions
        if view_count > 0 && commit_count < 3 && recent_transitions == 0 {
            let mut priority = 0.6;

            // Boost if user is currently working on a file linked to this ticket
            if let Some(ref fc) = focus {
                if let Some(ref linked) = fc.linked_ticket {
                    let ticket_key = entity.id.strip_prefix("ticket:").unwrap_or(&entity.id);
                    if linked.to_lowercase() == ticket_key.to_lowercase() {
                        priority = 0.85;
                    }
                }
            }

            items.push(AttentionItem {
                rule_name: "stale_ticket",
                priority,
                summary: format!(
                    "{} might need attention — {} commits in 5 days",
                    entity.display_name, commit_count
                ),
                suggested_action: "Check if this needs breaking down, re-scoping, or is blocked".to_string(),
                entity_ids: vec![entity.id.clone()],
                systems_involved: vec!["jira", "git"],
            });
        }
    }

    items.into_iter().take(1).collect()
}

/// Rule 2: Meeting Prep
///
/// Fires when a calendar meeting is within 2 hours and attendees have
/// open tickets or recent emails.
fn rule_meeting_prep(conn: &Connection) -> Vec<AttentionItem> {
    let now = now_ts();
    let two_hours = now + 2.0 * 3600.0;

    // Look for recent "meeting_scheduled" pulses with future timestamps
    let meetings = schema::get_relevant_entities(conn, 0.1, 10);
    let mut items = Vec::new();

    for entity in &meetings {
        if entity.entity_type != "meeting" {
            continue;
        }

        // Find people related to this meeting
        let attendees = schema::find_related(conn, &entity.id, "attends_with");
        if attendees.is_empty() {
            continue;
        }

        // Check if any attendee has open tickets or recent emails with us
        let mut attendee_context = Vec::new();
        for attendee_id in &attendees {
            let related_tickets = schema::find_related(conn, attendee_id, "assigned_to");
            let recent_emails = schema::count_recent_pulses(
                conn,
                attendee_id,
                "email_received",
                now - 7.0 * 86400.0,
            );
            if !related_tickets.is_empty() || recent_emails > 0 {
                let name = attendee_id.strip_prefix("person:").unwrap_or(attendee_id);
                attendee_context.push(format!(
                    "{} ({} tickets, {} recent emails)",
                    name,
                    related_tickets.len(),
                    recent_emails
                ));
            }
        }

        if !attendee_context.is_empty() {
            items.push(AttentionItem {
                rule_name: "meeting_prep",
                priority: 0.75,
                summary: format!(
                    "Upcoming meeting: {}. Attendees with activity: {}",
                    entity.display_name,
                    attendee_context.join(", ")
                ),
                suggested_action: "Review open tickets and recent emails with attendees".to_string(),
                entity_ids: vec![entity.id.clone()],
                systems_involved: vec!["calendar", "jira", "email"],
            });
        }
    }

    items.into_iter().take(1).collect()
}

/// Rule 3: Email-Ticket Link
///
/// Fires when a recent email mentions a Jira ticket key.
fn rule_email_ticket_link(conn: &Connection) -> Vec<AttentionItem> {
    let now = now_ts();
    let one_hour_ago = now - 3600.0;

    // Look for recent email pulses
    let mut stmt = conn.prepare(
        "SELECT p.summary, p.metadata, pe.entity_id
         FROM cortex_pulses p
         JOIN cortex_pulse_entities pe ON pe.pulse_id = p.id
         WHERE p.event_type = 'email_received' AND p.ts >= ?1
         AND pe.entity_id LIKE 'ticket:%'
         ORDER BY p.ts DESC LIMIT 5"
    ).ok();

    let mut items = Vec::new();
    if let Some(ref mut stmt) = stmt {
        if let Ok(rows) = stmt.query_map(rusqlite::params![one_hour_ago], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }) {
            for row in rows.flatten() {
                let (summary, _metadata, ticket_id) = row;
                let ticket = ticket_id.strip_prefix("ticket:").unwrap_or(&ticket_id);
                items.push(AttentionItem {
                    rule_name: "email_ticket_link",
                    priority: 0.65,
                    summary: format!("Email mentions ticket {}: {}", ticket.to_uppercase(), summary),
                    suggested_action: format!("Check {} status and respond to email if needed", ticket.to_uppercase()),
                    entity_ids: vec![ticket_id],
                    systems_involved: vec!["email", "jira"],
                });
            }
        }
    }

    items.into_iter().take(1).collect()
}

/// Rule 4: End of Day Summary
///
/// Fires after 6 PM local time if the user committed code today.
fn rule_end_of_day(conn: &Connection) -> Vec<AttentionItem> {
    let now = now_ts();

    // Simple hour check (UTC-based, not perfect but functional)
    let hour = (now as u64 / 3600) % 24;
    // Only fire between 17:00 and 22:00 UTC (adjust for timezone via offset in future)
    if hour < 17 || hour > 22 {
        return vec![];
    }

    let today_start = now - (now % 86400.0); // Midnight UTC today
    let commit_count = conn.query_row(
        "SELECT COUNT(*) FROM cortex_pulses WHERE event_type = 'commit_pushed' AND ts >= ?1",
        rusqlite::params![today_start],
        |row| row.get::<_, i64>(0),
    ).unwrap_or(0);

    if commit_count == 0 {
        return vec![];
    }

    // Count tickets touched today
    let tickets_touched: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT pe.entity_id) FROM cortex_pulses p
         JOIN cortex_pulse_entities pe ON pe.pulse_id = p.id
         WHERE p.ts >= ?1 AND pe.entity_id LIKE 'ticket:%'",
        rusqlite::params![today_start],
        |row| row.get(0),
    ).unwrap_or(0);

    vec![AttentionItem {
        rule_name: "end_of_day",
        priority: 0.5,
        summary: format!(
            "Today's activity: {} commits, {} tickets touched",
            commit_count, tickets_touched
        ),
        suggested_action: "Update ticket statuses and log progress".to_string(),
        entity_ids: vec![],
        systems_involved: vec!["git", "jira"],
    }]
}

/// Rule 5: Blocker Alert
///
/// Fires when the user is working on a ticket that depends on a stale ticket.
fn rule_blocker_alert(conn: &Connection, focus: Option<&FocusContext>) -> Vec<AttentionItem> {
    let focus = match focus {
        Some(f) => f,
        None => return vec![],
    };

    let ticket = match &focus.linked_ticket {
        Some(t) => t,
        None => return vec![],
    };

    let ticket_entity = format!("ticket:{}", ticket.to_lowercase());

    // Find "blocks" or "blocked_by" relationships
    let blockers = schema::find_related(conn, &ticket_entity, "blocks");
    let blocked_by = schema::find_related(conn, &ticket_entity, "blocked_by");

    let all_blockers: Vec<_> = blockers.iter().chain(blocked_by.iter()).collect();
    let mut items = Vec::new();

    for blocker_id in all_blockers {
        if !blocker_id.starts_with("ticket:") {
            continue;
        }

        // Check if the blocking ticket is stale (no recent activity)
        let now = now_ts();
        let recent = schema::count_recent_pulses(
            conn,
            blocker_id,
            "ticket_transitioned",
            now - 4.0 * 86400.0,
        );

        if recent == 0 {
            let blocker_key = blocker_id.strip_prefix("ticket:").unwrap_or(blocker_id);
            items.push(AttentionItem {
                rule_name: "blocker_alert",
                priority: 0.8,
                summary: format!(
                    "{} (your current ticket) is blocked by {} which hasn't moved in 4+ days",
                    ticket.to_uppercase(),
                    blocker_key.to_uppercase()
                ),
                suggested_action: "Follow up on the blocking ticket or find a workaround".to_string(),
                entity_ids: vec![ticket_entity.clone(), blocker_id.clone()],
                systems_involved: vec!["jira"],
            });
        }
    }

    items.into_iter().take(1).collect()
}

/// Rule 6: Review Reminder
///
/// Fires when there's a PR or review-related pulse older than 2 days
/// with no resolution.
fn rule_review_reminder(conn: &Connection) -> Vec<AttentionItem> {
    let now = now_ts();
    let two_days_ago = now - 2.0 * 86400.0;

    // Look for PR-related pulses that are old
    let pr_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM cortex_pulses
         WHERE event_type = 'pr_opened' AND ts < ?1 AND ts > ?2",
        rusqlite::params![two_days_ago, now - 14.0 * 86400.0],
        |row| row.get(0),
    ).unwrap_or(0);

    if pr_count == 0 {
        return vec![];
    }

    // Check if any were merged (resolved)
    let merged_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM cortex_pulses WHERE event_type = 'pr_merged' AND ts > ?1",
        rusqlite::params![two_days_ago],
        |row| row.get(0),
    ).unwrap_or(0);

    let pending = pr_count - merged_count;
    if pending <= 0 {
        return vec![];
    }

    vec![AttentionItem {
        rule_name: "review_reminder",
        priority: 0.55,
        summary: format!("{} pull request(s) open for more than 2 days", pending),
        suggested_action: "Review pending PRs or ping reviewers".to_string(),
        entity_ids: vec![],
        systems_involved: vec!["git"],
    }]
}

/// Rule 7: Sprint Pulse
///
/// Weekly check of sprint progress based on ticket activity.
fn rule_sprint_pulse(conn: &Connection) -> Vec<AttentionItem> {
    let now = now_ts();
    let one_week_ago = now - 7.0 * 86400.0;

    // Count tickets transitioned to done this week
    let done_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM cortex_pulses
         WHERE event_type = 'ticket_transitioned' AND ts >= ?1",
        rusqlite::params![one_week_ago],
        |row| row.get(0),
    ).unwrap_or(0);

    // Count total active tickets (viewed/updated in last 2 weeks)
    let active_count: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT pe.entity_id) FROM cortex_pulses p
         JOIN cortex_pulse_entities pe ON pe.pulse_id = p.id
         WHERE pe.entity_id LIKE 'ticket:%' AND p.ts >= ?1",
        rusqlite::params![now - 14.0 * 86400.0],
        |row| row.get(0),
    ).unwrap_or(0);

    if active_count == 0 {
        return vec![];
    }

    // Count commits this week
    let commit_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM cortex_pulses WHERE event_type = 'commit_pushed' AND ts >= ?1",
        rusqlite::params![one_week_ago],
        |row| row.get(0),
    ).unwrap_or(0);

    // Only report if there's actual progress to celebrate — never nag about inactivity
    if done_count == 0 && commit_count == 0 {
        return vec![];
    }

    vec![AttentionItem {
        rule_name: "sprint_pulse",
        priority: 0.4,
        summary: format!(
            "Weekly pulse: {} tickets completed, {} active tickets, {} commits",
            done_count, active_count, commit_count
        ),
        suggested_action: "Nice progress — review if any tickets need reprioritization".to_string(),
        entity_ids: vec![],
        systems_involved: vec!["jira", "git"],
    }]
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
