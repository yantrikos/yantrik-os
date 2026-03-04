//! Background cognition — periodic think cycles, instinct evaluation,
//! proactive message generation, narrative updates, evolution ticks, and urge expiry.
//!
//! Designed to run on a dedicated thread with a simple sleep loop
//! (no async runtime needed — all operations are synchronous).

use yantrikdb_core::types::ThinkConfig;
use yantrikdb_ml::{ChatMessage, GenerationConfig, parse_tool_calls};

use crate::automation::AutomationStore;
use crate::bond::BondTracker;
use crate::companion::CompanionService;
use crate::evolution::Evolution;
use crate::narrative::Narrative;
use crate::tools::{PermissionLevel, ToolContext, parse_permission};

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

    // 5b. Check event-triggered automations
    let events = service.drain_events();
    let mut automation_triggers: Vec<serde_json::Value> = Vec::new();
    for (event_type, event_data) in &events {
        let automations = AutomationStore::get_event_automations(service.db.conn(), event_type);
        for automation in automations {
            if !AutomationStore::event_matches(&automation.trigger_config, event_data) {
                continue;
            }
            tracing::info!(
                automation = %automation.name,
                event = %event_type,
                "Event automation triggered"
            );
            AutomationStore::record_run(service.db.conn(), &automation.automation_id);
            automation_triggers.push(serde_json::json!({
                "trigger_type": "automation",
                "automation_id": automation.automation_id,
                "name": automation.name,
                "steps": automation.steps,
                "condition": automation.condition,
                "urgency": 0.85,
            }));
        }
    }

    // 5c. Check schedule-triggered automations (action field starts with "automation:")
    for trigger in &triggers {
        if trigger.get("trigger_type").and_then(|v| v.as_str()) == Some("scheduled_task") {
            if let Some(action) = trigger.get("action").and_then(|v| v.as_str()) {
                if let Some(auto_id) = action.strip_prefix("automation:") {
                    if let Some(automation) = AutomationStore::get(service.db.conn(), auto_id) {
                        if automation.enabled {
                            AutomationStore::record_run(service.db.conn(), auto_id);
                            automation_triggers.push(serde_json::json!({
                                "trigger_type": "automation",
                                "automation_id": automation.automation_id,
                                "name": automation.name,
                                "steps": automation.steps,
                                "condition": automation.condition,
                                "urgency": 0.85,
                            }));
                        }
                    }
                }
            }
        }
    }

    // Merge automation triggers into the main triggers list
    let mut all_triggers = triggers;
    all_triggers.extend(automation_triggers);

    // Update cached state
    service.update_cognition_cache(
        all_triggers,
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

    // 9. Memory evolution — background maintenance (V23)
    {
        use crate::memory_evolution;
        let conn = service.db.conn();
        let cfg = &service.config.memory_evolution;

        // Gap 3: Semantic drift correction
        if cfg.consolidation_enabled && memory_evolution::should_consolidate(conn, cfg) {
            memory_evolution::run_consolidation(&service.db, &*service.llm, cfg);
        }

        // Gap 5: Memory pruning
        if cfg.variable_halflife_enabled && memory_evolution::should_prune(conn, cfg) {
            memory_evolution::run_pruning(&service.db, &*service.llm, cfg);
        }

        // Gap 4: Reference freshness decay + auto-detection
        if cfg.reference_freshness_enabled {
            memory_evolution::decay_reference_freshness(conn, cfg);
            memory_evolution::detect_emerging_references(&service.db, conn, &*service.llm);
        }

        // Gap 1: Expire stale conversation context
        memory_evolution::expire_stale_context(conn);

        // Memory graph weaving — proactive idle-time linking
        if cfg.weaving_enabled && memory_evolution::should_weave(conn, cfg) {
            // Only weave when system is idle (>15 min since last interaction)
            let idle = service.idle_seconds();
            if idle > cfg.weaving_interval_hours.min(0.25) * 3600.0 {
                memory_evolution::run_weaving_cycle(&service.db, &*service.llm, cfg);
            }
        }
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

    // Check if any urge has executable actions (EXECUTE prefix)
    let has_actions = urges.iter().any(|u| u.reason.starts_with("EXECUTE "));

    let system_prompt = if has_actions {
        format!(
            "You are {}, a proactive companion. Some items below are EXECUTE instructions \u{2014} \
             carry them out using your available tools. For non-EXECUTE items, generate a brief, \
             natural message. Be warm but concise.",
            service.config.personality.name,
        )
    } else {
        format!(
            "You are {}, a thoughtful companion. Generate a brief, natural message \
             based on what's on your mind. Be warm but concise (1-2 sentences).",
            service.config.personality.name,
        )
    };

    let messages = vec![
        ChatMessage::system(system_prompt),
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
        max_tokens: if has_actions { 500 } else { 150 },
        temperature: 0.7,
        top_p: Some(0.9),
        ..Default::default()
    };

    // If actions present, provide tools for autonomous execution
    let max_perm = parse_permission(&service.config.tools.max_permission);
    let tools: Option<Vec<serde_json::Value>> = if has_actions && service.config.tools.enabled {
        Some(service.registry.definitions(max_perm))
    } else {
        None
    };
    let tools_ref: Option<&[serde_json::Value]> = tools.as_deref();

    match service.llm.chat(&messages, &gen_config, tools_ref) {
        Ok(response) => {
            // Execute any tool calls from the response
            let mut response_text = response.text.clone();
            let tool_calls = if !response.tool_calls.is_empty() {
                response.tool_calls.clone()
            } else {
                parse_tool_calls(&response.text)
            };

            if !tool_calls.is_empty() {
                let ctx = ToolContext {
                    db: &service.db,
                    max_permission: max_perm,
                    registry_metadata: None,
                    task_manager: Some(&service.task_manager),
                };
                for tc in &tool_calls {
                    tracing::info!(tool = %tc.name, "Auto-executing tool from automation");
                    let result = service.registry.execute(&ctx, &tc.name, &tc.arguments);
                    tracing::debug!(tool = %tc.name, result_len = result.len(), "Tool result");
                }

                // Strip tool call XML from the text if present
                if let Some(pos) = response_text.find("<tool_call>") {
                    response_text = response_text[..pos].trim().to_string();
                }
                if response_text.is_empty() {
                    response_text = format!(
                        "I ran {} automation action(s) in the background.",
                        tool_calls.len()
                    );
                }
            }

            let msg = crate::types::ProactiveMessage {
                text: response_text,
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
