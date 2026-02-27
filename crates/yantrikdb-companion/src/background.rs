//! Background cognition — periodic think cycles, instinct evaluation,
//! proactive message generation, narrative updates, evolution ticks, and urge expiry.
//!
//! Designed to run on a dedicated thread with a simple sleep loop
//! (no async runtime needed — all operations are synchronous).

use yantrikdb_core::types::ThinkConfig;
use yantrikdb_ml::{ChatMessage, GenerationConfig};

use crate::bond::BondTracker;
use crate::companion::CompanionService;
use crate::evolution::Evolution;
use crate::narrative::Narrative;

/// Run a single background cognition cycle.
///
/// Call this periodically from a timer/thread. It:
/// 1. Runs db.think() to process memory maintenance
/// 2. Caches triggers, patterns, conflicts
/// 3. Evaluates all instincts → pushes urges
/// 4. Generates a proactive message if urgency is high enough
pub fn run_think_cycle(service: &mut CompanionService) {
    // Clone threshold to avoid holding immutable borrow across mutable calls
    let proactive_threshold = service.config.cognition.proactive_urgency_threshold;

    // 1. Run think
    let think_config = ThinkConfig::default();
    let think_result = match service.db.think(&think_config) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Think cycle failed: {e}");
            return;
        }
    };

    // 2. Extract triggers as JSON values
    let triggers: Vec<serde_json::Value> = think_result
        .triggers
        .iter()
        .map(|t| {
            serde_json::json!({
                "trigger_type": t.trigger_type,
                "reason": t.reason,
                "urgency": t.urgency,
                "context": t.context,
            })
        })
        .collect();

    // 3. Fetch patterns
    let patterns: Vec<serde_json::Value> = service
        .db
        .get_patterns(None, Some("active"), 10)
        .unwrap_or_default()
        .iter()
        .map(|p| {
            serde_json::json!({
                "pattern_type": p.pattern_type,
                "description": p.description,
                "confidence": p.confidence,
            })
        })
        .collect();

    // 4. Count open conflicts
    let conflicts_count = service
        .db
        .get_conflicts(Some("open"), None, None, None, 100)
        .map(|c| c.len())
        .unwrap_or(0);

    // 5. Extract valence trend
    let valence_avg = triggers
        .iter()
        .find(|t| {
            t.get("trigger_type").and_then(|v| v.as_str()) == Some("valence_trend")
        })
        .and_then(|t| {
            t.get("context")
                .and_then(|c| c.get("current_avg"))
                .and_then(|v| v.as_f64())
        });

    // Update cached state
    service.update_cognition_cache(
        triggers,
        patterns.clone(),
        conflicts_count,
        valence_avg,
    );

    // 6. Evaluate instincts
    let state = service.build_state();
    let mut high_urgency_urges = Vec::new();
    let instinct_urges = service.evaluate_instincts(&state);

    for spec in &instinct_urges {
        if spec.urgency >= proactive_threshold {
            high_urgency_urges.push(spec.clone());
        }
        service.urge_queue.push(service.db.conn(), spec);
    }

    // 7. Generate proactive message if urgency is high
    if !high_urgency_urges.is_empty() {
        generate_proactive_message(service, &high_urgency_urges);
    }

    // 8. Background narrative update (if needed and bond is enabled)
    if service.config.bond.enabled {
        let needs_narrative = Narrative::tick_interaction(
            service.db.conn(),
            service.config.narrative.update_interval_interactions,
        );
        if needs_narrative {
            update_narrative_background(service);
        }

        // Tick evolution (formality shift) based on current bond level
        let bond_state = BondTracker::get_state(service.db.conn());
        Evolution::tick(
            service.db.conn(),
            bond_state.bond_level,
            service.config.evolution.formality_alpha,
        );
    }

    tracing::debug!(
        triggers = state.pending_triggers.len(),
        patterns = patterns.len(),
        conflicts = conflicts_count,
        bond = service.bond_level().name(),
        "Think cycle complete"
    );
}

/// Run urge expiry (call periodically, e.g., every hour).
pub fn expire_urges(service: &mut CompanionService) {
    let expired = service.urge_queue.expire_old(service.db.conn());
    if expired > 0 {
        tracing::debug!(expired, "Expired old urges");
    }
}

/// Get the adaptive think interval based on idle time.
pub fn get_think_interval(service: &CompanionService) -> u64 {
    let config = &service.config.cognition;
    let idle = service.idle_seconds();

    if idle < config.think_interval_active_minutes as f64 * 60.0 {
        config.think_interval_active_minutes * 60
    } else if idle < 3600.0 {
        config.think_interval_minutes * 60
    } else {
        config.idle_think_interval_minutes * 60
    }
}

fn generate_proactive_message(
    service: &mut CompanionService,
    urges: &[crate::types::UrgeSpec],
) {
    let urge_reasons: Vec<&str> = urges.iter().map(|u| u.reason.as_str()).collect();
    let urge_ids: Vec<String> = urges
        .iter()
        .map(|u| u.cooldown_key.clone())
        .collect();

    let messages = vec![
        ChatMessage::system(format!(
            "You are {}, a thoughtful companion. Generate a brief, natural message \
             based on what's on your mind. Be warm but concise (1-2 sentences).",
            service.config.personality.name,
        )),
        ChatMessage::user(format!(
            "Things on your mind:\n{}",
            urge_reasons
                .iter()
                .map(|r| format!("- {r}"))
                .collect::<Vec<_>>()
                .join("\n")
        )),
    ];

    let gen_config = GenerationConfig {
        max_tokens: 150,
        temperature: 0.7,
        top_p: Some(0.9),
        ..Default::default()
    };

    match service.llm.chat(&messages, &gen_config) {
        Ok(response) => {
            let msg = crate::types::ProactiveMessage {
                text: response.text,
                urge_ids,
                generated_at: now_ts(),
            };
            service.set_proactive_message(msg);
        }
        Err(e) => {
            tracing::warn!("Failed to generate proactive message: {e}");
        }
    }
}

/// Background narrative generation — gathers self-reflections and updates the diary.
fn update_narrative_background(service: &mut CompanionService) {
    // Gather recent self-reflections
    let self_memories = service
        .db
        .recall_text("self-reflection companion observations", 10)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| {
            r.source == "self" || r.domain == "self-reflection"
        })
        .take(5)
        .map(|r| r.text)
        .collect::<Vec<_>>();

    let bond_state = BondTracker::get_state(service.db.conn());

    Narrative::update(
        service.db.conn(),
        &*service.llm,
        &service.config.user_name,
        bond_state.bond_level,
        bond_state.bond_score,
        &self_memories,
        service.config.narrative.max_tokens,
    );
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}
