//! Context Recovery playbook — helps the user resume work after being away.
//!
//! Evidence signals (need 2+ for high conviction):
//! 1. User was idle for 30+ minutes and just became active
//! 2. There are recent entities with high relevance that were active before idle
//! 3. There's an unfinished task or open ticket from before the break
//!
//! Action: Notify with "Welcome back" context summary of what was happening.

use crate::playbook::{CortexAction, PlaybookState};
use crate::focus::ActivityType;

/// Evaluate context recovery needs. Pure Rust, no LLM.
pub fn evaluate(state: &PlaybookState) -> Vec<CortexAction> {
    let conn = state.conn;
    let now = state.now_ts;
    let focus = match state.current_focus {
        Some(f) => f,
        None => return vec![], // No focus data — can't detect return
    };

    // Only trigger when user just came back from idle
    // They should be active now but recently idle
    if focus.activity == ActivityType::Idle {
        return vec![]; // Still idle, wait
    }

    // Check if they were recently idle (came back within last 5 minutes)
    // We detect this by checking if there's an idle gap in recent pulses
    let five_min_ago = now - 300.0;
    let idle_threshold = 1800.0; // 30 minutes

    // Look for a gap in pulse activity indicating idle period
    let recent_pulse_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM cortex_pulses WHERE ts > ?1 AND ts < ?2",
        rusqlite::params![five_min_ago, now],
        |row| row.get(0),
    ).unwrap_or(0);

    // If there are already many recent pulses, user has been back a while
    if recent_pulse_count > 5 {
        return vec![];
    }

    // Check for an idle gap: no pulses between 30-5 minutes ago
    let gap_start = now - idle_threshold;
    let gap_end = five_min_ago;
    let gap_pulse_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM cortex_pulses WHERE ts > ?1 AND ts < ?2",
        rusqlite::params![gap_start, gap_end],
        |row| row.get(0),
    ).unwrap_or(0);

    // If there were pulses during the gap period, user wasn't really idle
    if gap_pulse_count > 3 {
        return vec![];
    }

    // ── Evidence gathering ─────────────────────────────────────────────

    let mut evidence_count = 0u32;
    let mut evidence_details = Vec::new();
    let mut context_items = Vec::new();

    // Signal 1: Return from idle (confirmed by gap detection above)
    evidence_count += 1;
    evidence_details.push("returned from idle period".to_string());

    // Signal 2: Find what was active before the idle period
    // Look for entities with recent pulses before the gap
    let before_idle = gap_start;
    let pre_idle_window = before_idle - 3600.0; // 1 hour before going idle

    let active_entities: Vec<(String, String, i64)> = {
        let query = "
            SELECT e.display_name, e.entity_type, COUNT(pe.pulse_id) as pulse_count
            FROM cortex_entities e
            JOIN cortex_pulse_entities pe ON pe.entity_id = e.id
            JOIN cortex_pulses p ON p.id = pe.pulse_id
            WHERE p.ts > ?1 AND p.ts < ?2
              AND e.entity_type IN ('ticket', 'file', 'project', 'repository')
              AND e.relevance > 0.3
            GROUP BY e.id
            ORDER BY pulse_count DESC
            LIMIT 5
        ";

        if let Ok(mut stmt) = conn.prepare(query) {
            stmt.query_map(
                rusqlite::params![pre_idle_window, before_idle],
                |row| Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                )),
            )
            .ok()
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
        } else {
            vec![]
        }
    };

    if !active_entities.is_empty() {
        evidence_count += 1;
        let names: Vec<String> = active_entities.iter()
            .take(3)
            .map(|(name, etype, _)| format!("{} ({})", name, etype))
            .collect();
        evidence_details.push(format!("active before idle: {}", names.join(", ")));

        for (name, etype, _count) in &active_entities {
            context_items.push(format!("{}: {}", etype, name));
        }
    }

    // Signal 3: Check for unfinished tasks in the task queue
    let pending_tasks: Vec<String> = {
        let query = "
            SELECT description FROM task_queue
            WHERE status IN ('pending', 'in_progress')
            ORDER BY created_at DESC
            LIMIT 3
        ";

        if let Ok(mut stmt) = conn.prepare(query) {
            stmt.query_map([], |row| row.get::<_, String>(0))
                .ok()
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
        } else {
            vec![]
        }
    };

    if !pending_tasks.is_empty() {
        evidence_count += 1;
        evidence_details.push(format!("{} pending task(s)", pending_tasks.len()));
        for task in &pending_tasks {
            context_items.push(format!("task: {}", truncate(task, 60)));
        }
    }

    // Require at least 2 evidence signals
    if evidence_count < 2 {
        return vec![];
    }

    let explanation = format!(
        "Context recovery: {}",
        evidence_details.join("; ")
    );

    // Build the body — a concise "welcome back" with context
    let body = if context_items.is_empty() {
        "You were away for a while. Welcome back!".to_string()
    } else {
        format!(
            "Before your break:\n{}",
            context_items.iter()
                .take(4)
                .map(|s| format!("  - {}", s))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    vec![CortexAction::Notify {
        title: "Welcome back".to_string(),
        body,
        explanation,
        playbook_id: "context_recovery".to_string(),
    }]
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        &s[..s.char_indices().take(max_len).last().map(|(i, _)| i).unwrap_or(0)]
    }
}
