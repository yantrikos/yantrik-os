//! Brain Loop — bridges the autonomous brain (yantrikdb-core) with the
//! companion's instinct evaluation and urge queue.
//!
//! This module:
//! 1. Wraps instinct UrgeSpec outputs into BrainCandidates with signal semantics
//! 2. Runs brain_tick() to score, orient, and filter candidates
//! 3. Feeds survivors back into the urge queue
//! 4. Persists BrainState across ticks (JSON blob in cognitive_nodes)
//! 5. Reports whether external data fetching should be triggered
//!
//! The brain loop does NOT replace the existing think cycle — it augments step 6
//! (instinct evaluation → urge push) with signal-aware scoring and homeostatic
//! drive modulation.

use yantrikdb_core::cognition::brain::{
    self, BrainCandidate, BrainState, BrainTickResult, CandidateSource,
    OrientationInput, SignalType,
};
use yantrikdb_core::cognition::brain_consolidation;
use yantrikdb_core::cognition::curiosity;
use yantrikdb_core::cognition::detectors;

use crate::companion::CompanionService;
use crate::types::UrgeSpec;

// ══════════════════════════════════════════════════════════════════════════════
// § 1  UrgeSpec → BrainCandidate conversion
// ══════════════════════════════════════════════════════════════════════════════

/// Convert an instinct's UrgeSpec into a BrainCandidate with inferred signal type.
fn wrap_urge(spec: &UrgeSpec, now: f64) -> BrainCandidate {
    let signal_type = brain::infer_signal_type(
        &spec.instinct_name,
        &spec.reason,
        &spec.context,
    );

    BrainCandidate {
        candidate_id: format!("{}:{:.0}", spec.cooldown_key, now),
        source: CandidateSource::Instinct {
            instinct_name: spec.instinct_name.clone(),
            category: format!("{:?}", spec.category),
        },
        signal_type,
        raw_urgency: spec.urgency,
        brain_score: 0.0, // computed by brain_tick
        reason: spec.reason.clone(),
        suggested_message: spec.suggested_message.clone(),
        action: spec.action.clone(),
        context: spec.context.clone(),
        cooldown_key: spec.cooldown_key.clone(),
        orientation: None,
        created_at: now,
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// § 2  BrainState persistence
// ══════════════════════════════════════════════════════════════════════════════

const BRAIN_STATE_KEY: &str = "__brain_state_v1";

/// Load BrainState from the DB. Returns default if not found.
fn load_brain_state(conn: &rusqlite::Connection) -> BrainState {
    let json: Option<String> = conn
        .query_row(
            "SELECT context FROM cognitive_nodes WHERE node_id = ?1",
            rusqlite::params![BRAIN_STATE_KEY],
            |row| row.get(0),
        )
        .ok();

    match json {
        Some(s) => serde_json::from_str(&s).unwrap_or_default(),
        None => BrainState::default(),
    }
}

/// Save BrainState to the DB.
fn save_brain_state(conn: &rusqlite::Connection, state: &BrainState) {
    let json = match serde_json::to_string(state) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("Failed to serialize BrainState: {e}");
            return;
        }
    };

    let now = now_ts();
    conn.execute(
        "INSERT INTO cognitive_nodes (node_id, node_type, label, context, confidence, activation, salience, created_at, updated_at)
         VALUES (?1, 'brain_state', 'brain', ?2, 1.0, 1.0, 1.0, ?3, ?3)
         ON CONFLICT(node_id) DO UPDATE SET context = ?2, updated_at = ?3",
        rusqlite::params![BRAIN_STATE_KEY, json, now],
    )
    .ok();
}

// ══════════════════════════════════════════════════════════════════════════════
// § 3  Brain-augmented instinct evaluation
// ══════════════════════════════════════════════════════════════════════════════

/// Result of running the brain loop — returned to background.rs.
pub struct BrainLoopResult {
    /// Urges that passed the brain's scoring and orientation.
    pub emitted_urges: Vec<UrgeSpec>,

    /// Number of instinct outputs that were suppressed by the brain.
    pub suppressed_count: usize,

    /// Whether the brain wants external data (RSS, weather, news).
    pub should_fetch_external: bool,

    /// Recommended next tick interval (seconds).
    pub next_tick_secs: f64,

    /// Drive snapshot for logging.
    pub drives: brain::Homeostasis,
}

/// Run the brain loop: evaluate instincts, wrap into candidates, score via
/// brain_tick, and return the survivors as UrgeSpecs.
///
/// This replaces the direct `evaluate_instincts → push` flow in background.rs
/// with a signal-aware, homeostasis-modulated pipeline.
pub fn run_brain_loop(service: &CompanionService) -> BrainLoopResult {
    let now = now_ts();
    let conn = service.db.conn();

    // 1. Load persisted brain state
    let mut brain_state = load_brain_state(conn);

    // 2. Evaluate all instincts (unchanged from before)
    let state = service.build_state();
    let instinct_urges = service.evaluate_instincts(&state);

    // 3. Wrap each UrgeSpec into a BrainCandidate
    let mut candidates: Vec<BrainCandidate> = instinct_urges
        .iter()
        .map(|spec| wrap_urge(spec, now))
        .collect();

    // 3b. Run native detectors (pattern breaks, baseline deviations, forward projections)
    detectors::ensure_detector_tables(conn);
    let detector_candidates = detectors::run_all_detectors(conn, now);
    candidates.extend(detector_candidates);

    // 3c. Curiosity engine — plan external fetches when drives warrant it
    curiosity::ensure_curiosity_tables(conn);
    let fetch_budget = 5.0; // max fetch cost per tick
    let curiosity_candidates = curiosity::plan_fetches(conn, &brain_state.drives, now, fetch_budget);
    candidates.extend(curiosity_candidates);

    // 4. Count unresolved tensions for drive updates
    let unresolved_tensions = state.open_loops_count as usize
        + state.overdue_commitment_count
        + state.open_conflicts_count;

    // 5. Update homeostatic drives
    brain_state.update_drives(now, unresolved_tensions);

    // 6. Build orientation input from companion state
    let active_cooldowns: Vec<String> = conn
        .prepare(
            "SELECT DISTINCT cooldown_key FROM urges
             WHERE status IN ('pending', 'delivered')
             AND cooldown_key != ''",
        )
        .ok()
        .map(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(0))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let pending_urge_count = service.urge_queue.count_pending(conn);

    let orientation_input = OrientationInput {
        now,
        idle_secs: state.idle_seconds,
        pending_urge_count: pending_urge_count as usize,
        active_cooldowns,
        current_hour: state.current_hour,
    };

    // 7. Run brain tick
    let max_emit = 5; // max urges per tick
    let tick_result: BrainTickResult = brain::brain_tick(
        &mut brain_state,
        candidates,
        &orientation_input,
        max_emit,
    );

    // 8. Convert emitted BrainCandidates back to UrgeSpecs
    let emitted_urges: Vec<UrgeSpec> = tick_result
        .emitted
        .iter()
        .map(|candidate| {
            // Try to find the original instinct UrgeSpec
            if let Some(original) = instinct_urges
                .iter()
                .find(|spec| spec.cooldown_key == candidate.cooldown_key)
            {
                let mut spec = original.clone();
                spec.urgency = candidate.brain_score;
                spec
            } else {
                // Detector candidate — synthesize an UrgeSpec
                candidate_to_urge_spec(candidate)
            }
        })
        .collect();

    // 9. If brain had no candidates and info hunger is high, mark data event as stale
    if tick_result.should_fetch_external {
        tracing::info!(
            hunger = tick_result.drives_snapshot.information_hunger,
            "Brain is information-hungry — external fetch recommended"
        );
    }

    // 10. Persist brain state
    save_brain_state(conn, &brain_state);

    // 10b. Nightly consolidation (runs once per 24h during idle)
    if brain_consolidation::should_consolidate(conn, now, state.idle_seconds) {
        let report = brain_consolidation::run_consolidation(conn, now);
        if report.expectations_pruned > 0 || report.baselines_pruned > 0 {
            tracing::info!(
                pruned = report.expectations_pruned + report.baselines_pruned,
                "Brain nightly consolidation"
            );
        }
    }

    // 11. Log brain diagnostics
    tracing::debug!(
        tick = brain_state.tick_count,
        emitted = tick_result.emitted.len(),
        suppressed = tick_result.suppressed_count,
        threshold = format!("{:.2}", brain_state.drives.surfacing_threshold()),
        info_hunger = format!("{:.2}", tick_result.drives_snapshot.information_hunger),
        tension = format!("{:.2}", tick_result.drives_snapshot.tension_pressure),
        usefulness = format!("{:.2}", tick_result.drives_snapshot.usefulness_pressure),
        next_tick = format!("{:.0}s", tick_result.next_tick_secs),
        "Brain tick complete"
    );

    BrainLoopResult {
        emitted_urges,
        suppressed_count: tick_result.suppressed_count,
        should_fetch_external: tick_result.should_fetch_external,
        next_tick_secs: tick_result.next_tick_secs,
        drives: tick_result.drives_snapshot,
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// § 3b  Detector candidate → UrgeSpec conversion
// ══════════════════════════════════════════════════════════════════════════════

/// Convert a detector-generated BrainCandidate into an UrgeSpec for the urge queue.
fn candidate_to_urge_spec(candidate: &BrainCandidate) -> UrgeSpec {
    use yantrik_companion_core::types::{
        AutonomyTier, InstinctCategory, TimeSensitivity,
    };

    let detector_name = match &candidate.source {
        CandidateSource::Detector { detector_name } => detector_name.as_str(),
        CandidateSource::ExternalFetch { fetch_type } => fetch_type.as_str(),
        CandidateSource::Instinct { instinct_name, .. } => instinct_name.as_str(),
    };

    let category = match candidate.signal_type {
        SignalType::PredictionError => InstinctCategory::Awareness,
        SignalType::Tension => InstinctCategory::Anticipatory,
        SignalType::Opportunity => InstinctCategory::Growth,
        SignalType::Uncertainty => InstinctCategory::Meta,
    };

    let time_sensitivity = match candidate.signal_type {
        SignalType::PredictionError => TimeSensitivity::Today,
        SignalType::Tension => TimeSensitivity::Soon,
        SignalType::Opportunity => TimeSensitivity::Today,
        SignalType::Uncertainty => TimeSensitivity::Ambient,
    };

    UrgeSpec {
        instinct_name: format!("brain:{}", detector_name),
        reason: candidate.reason.clone(),
        urgency: candidate.brain_score,
        suggested_message: candidate.suggested_message.clone(),
        action: candidate.action.clone(),
        context: candidate.context.clone(),
        cooldown_key: candidate.cooldown_key.clone(),
        time_sensitivity,
        category,
        guaranteed: false,
        autonomy: AutonomyTier::NotifySuggestion,
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// § 4  Feedback recording
// ══════════════════════════════════════════════════════════════════════════════

/// Record user feedback for a brain candidate's source.
/// Called when the user acts on, reads, or ignores a proactive message.
///
/// - `acted_on`: 1.0 (user engaged, clicked, replied)
/// - `read`: 0.5 (user saw it, dismissed without negative reaction)
/// - `ignored`: 0.0 (user didn't interact, or explicitly dismissed)
pub fn record_brain_feedback(
    conn: &rusqlite::Connection,
    instinct_name: &str,
    signal_type_str: &str,
    reward: f64,
) {
    let mut brain_state = load_brain_state(conn);

    let signal_type = match signal_type_str {
        "prediction_error" => SignalType::PredictionError,
        "tension" => SignalType::Tension,
        "opportunity" => SignalType::Opportunity,
        "uncertainty" => SignalType::Uncertainty,
        _ => SignalType::Uncertainty,
    };

    let source_key = format!("instinct:{instinct_name}");
    brain_state.feedback.record(&source_key, signal_type, reward);

    // Update usefulness pressure from recent acceptance rate
    let total: u64 = brain_state.feedback.outcome_count.values().sum();
    if total > 5 {
        // Compute rough acceptance rate from EMA values
        let emas: Vec<f64> = brain_state.feedback.ema.values()
            .flat_map(|m| m.values())
            .copied()
            .collect();
        if !emas.is_empty() {
            let avg_ema: f64 = emas.iter().sum::<f64>() / emas.len() as f64;
            brain_state.drives.update_usefulness(avg_ema);
        }
    }

    save_brain_state(conn, &brain_state);
}

/// Notify the brain that fresh external data arrived (RSS, weather, etc.).
/// This feeds the information_hunger drive.
pub fn record_data_event(conn: &rusqlite::Connection) {
    let mut brain_state = load_brain_state(conn);
    let now = now_ts();
    brain_state.last_data_event_at = now;
    brain_state.drives.feed_information(0.3);
    save_brain_state(conn, &brain_state);
}

// ══════════════════════════════════════════════════════════════════════════════
// § 4b  Entity event recording (feeds detectors)
// ══════════════════════════════════════════════════════════════════════════════

/// Record an entity event for pattern break detection.
/// Call this when a contact messages, a routine fires, an app opens, etc.
///
/// Examples:
/// - `record_entity_event(conn, "contact:mom", "call")` — mom called
/// - `record_entity_event(conn, "routine:morning_walk", "completed")` — walked
/// - `record_entity_event(conn, "app:slack", "opened")` — opened Slack
pub fn record_entity_event(conn: &rusqlite::Connection, entity_id: &str, kind: &str) {
    let now = now_ts();
    detectors::ensure_detector_tables(conn);
    detectors::PatternBreakDetector::record_event(conn, entity_id, kind, now);

    // Also feed information hunger
    let mut state = load_brain_state(conn);
    state.last_data_event_at = now;
    state.drives.feed_information(0.1);
    save_brain_state(conn, &state);
}

/// Record a metric observation for baseline deviation detection.
/// Call this when quantitative data changes (spending, sleep, email count, etc.).
///
/// Examples:
/// - `record_metric(conn, "user", "daily_spending", 47.50)` — today's spend
/// - `record_metric(conn, "user", "sleep_start_hour", 23.5)` — went to bed 11:30pm
/// - `record_metric(conn, "user", "emails_received", 42.0)` — emails today
pub fn record_metric(
    conn: &rusqlite::Connection,
    entity_id: &str,
    metric_name: &str,
    value: f64,
) -> Option<detectors::DeviationResult> {
    let now = now_ts();
    detectors::ensure_detector_tables(conn);
    detectors::BaselineDeviationDetector::observe(conn, entity_id, metric_name, value, now)
}

// ══════════════════════════════════════════════════════════════════════════════
// § 5  Diagnostics
// ══════════════════════════════════════════════════════════════════════════════

/// Get a snapshot of the brain state for diagnostics/UI display.
pub fn brain_diagnostics(conn: &rusqlite::Connection) -> serde_json::Value {
    let state = load_brain_state(conn);
    serde_json::json!({
        "tick_count": state.tick_count,
        "drives": {
            "information_hunger": state.drives.information_hunger,
            "novelty_hunger": state.drives.novelty_hunger,
            "tension_pressure": state.drives.tension_pressure,
            "usefulness_pressure": state.drives.usefulness_pressure,
        },
        "threshold": state.drives.surfacing_threshold(),
        "scan_interval_secs": state.drives.scan_interval_secs(),
        "total_emitted": state.total_emitted,
        "total_suppressed": state.total_suppressed,
        "source_diversity": state.source_diversity(),
        "feedback_sources": state.feedback.outcome_count.len(),
    })
}

// ══════════════════════════════════════════════════════════════════════════════
// § 6  Seeding (bootstrap detectors from existing data)
// ══════════════════════════════════════════════════════════════════════════════

/// Seed entity_expectations from existing person_profiles and wm_routines.
/// Call once at startup or after first install to bootstrap the pattern break
/// detector with existing behavioral data.
pub fn seed_from_existing_data(conn: &rusqlite::Connection) {
    let now = now_ts();
    detectors::ensure_detector_tables(conn);
    curiosity::ensure_curiosity_tables(conn);

    let mut seeded = 0;

    // 1. Seed contact expectations from person_profiles
    //    If we know someone's avg_response_hours and they have enough communications,
    //    create an expectation for their message frequency.
    {
        let mut stmt = match conn.prepare(
            "SELECT person_id, name, preferred_channel, avg_response_hours,
                    total_communications, last_contact_at
             FROM person_profiles
             WHERE total_communications >= 3
               AND last_contact_at IS NOT NULL"
        ) {
            Ok(s) => s,
            Err(_) => return,
        };

        let rows: Vec<(String, String, String, f64, i64, f64)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?,
                    row.get(3)?, row.get(4)?, row.get(5)?))
            })
            .ok()
            .map(|r| r.flatten().collect())
            .unwrap_or_default();

        for (person_id, _name, channel, avg_response_hrs, total_comms, last_contact) in rows {
            let entity_id = format!("contact:{person_id}");

            // Estimate expected interval from communication frequency
            // If 30 communications over 90 days → ~3 day interval
            let age_days = (now - last_contact).max(1.0) / 86400.0 + 30.0;
            let estimated_interval = (age_days * 86400.0) / total_comms.max(1) as f64;

            // Only create if interval is reasonable (1 hour to 30 days)
            if estimated_interval < 3600.0 || estimated_interval > 2592000.0 {
                continue;
            }

            // Confidence based on number of communications
            let confidence = (1.0 - (-0.1 * total_comms as f64).exp()).min(0.8);

            conn.execute(
                "INSERT OR IGNORE INTO entity_expectations
                 (entity_id, expectation_kind, expected_interval_sec, confidence,
                  last_seen_at, miss_count, n, mean_interval, m2_interval, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?3, 0.0, ?7, ?7)",
                rusqlite::params![
                    entity_id, channel, estimated_interval, confidence,
                    last_contact, total_comms.min(5), now
                ],
            )
            .ok();

            // Also seed response time baseline if available
            if avg_response_hrs > 0.0 {
                // Insert a synthetic baseline with some assumed variance
                conn.execute(
                    "INSERT OR IGNORE INTO entity_metric_baselines
                     (entity_id, metric_name, window_kind, n, mean, m2, last_value, last_seen_ts, updated_at)
                     VALUES (?1, 'response_hours', 'rolling', ?2, ?3, ?4, ?3, ?5, ?5)",
                    rusqlite::params![
                        entity_id,
                        total_comms.min(20), // cap at 20 for reasonable baseline
                        avg_response_hrs,
                        avg_response_hrs * avg_response_hrs * 0.5, // assume ~70% CV
                        now,
                    ],
                )
                .ok();
            }

            seeded += 1;
        }
    }

    // 2. Seed routine expectations from wm_routines
    {
        let mut stmt = match conn.prepare(
            "SELECT id, description, trigger_data, observation_count, confidence, last_observed_at
             FROM wm_routines
             WHERE active = 1 AND confidence > 0.3 AND observation_count >= 3"
        ) {
            Ok(s) => s,
            Err(_) => return,
        };

        let rows: Vec<(i64, String, String, i64, f64, f64)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?,
                    row.get(3)?, row.get(4)?, row.get(5)?))
            })
            .ok()
            .map(|r| r.flatten().collect())
            .unwrap_or_default();

        for (id, description, trigger_data, obs_count, confidence, last_observed) in rows {
            let entity_id = format!("routine:{id}");
            let kind = description.chars().take(50).collect::<String>();

            // Try to extract interval from trigger_data JSON
            let interval = extract_routine_interval(&trigger_data).unwrap_or(86400.0); // default daily

            conn.execute(
                "INSERT OR IGNORE INTO entity_expectations
                 (entity_id, expectation_kind, expected_interval_sec, confidence,
                  last_seen_at, miss_count, n, mean_interval, m2_interval, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?3, 0.0, ?7, ?7)",
                rusqlite::params![
                    entity_id, kind, interval, confidence.min(0.7),
                    last_observed, obs_count.min(10), now
                ],
            )
            .ok();

            seeded += 1;
        }
    }

    if seeded > 0 {
        tracing::info!(seeded, "Brain seeded entity expectations from existing data");
    }
}

/// Try to extract a routine interval from trigger_data JSON.
/// Looks for TimeOfDay triggers (→ daily), day-of-week patterns, etc.
fn extract_routine_interval(trigger_json: &str) -> Option<f64> {
    let v: serde_json::Value = serde_json::from_str(trigger_json).ok()?;

    // Check for time_of_day trigger → daily interval
    if v.get("TimeOfDay").is_some() || v.get("time_of_day").is_some() {
        return Some(86400.0); // daily
    }

    // Check for day_of_week → weekly interval
    if v.get("DayOfWeek").is_some() || v.get("day_of_week").is_some() {
        return Some(604800.0); // weekly
    }

    // Check for "After" trigger → context-dependent, assume 2x daily
    if v.get("After").is_some() {
        return Some(86400.0);
    }

    None
}

// ══════════════════════════════════════════════════════════════════════════════

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
