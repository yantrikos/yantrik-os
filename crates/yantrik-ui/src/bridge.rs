//! Companion bridge — worker thread that owns CompanionService.
//!
//! The Slint UI thread sends commands via crossbeam channel.
//! The worker thread processes them sequentially and pushes
//! state updates back via slint::invoke_from_event_loop().

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::{Receiver, Sender};
use yantrikdb_companion::{CompanionConfig, CompanionService};
use yantrikdb_companion::bond::{BondLevel, BondTracker};
use yantrikdb_companion::evolution::Evolution;
use yantrikdb_ml::{CandleEmbedder, CandleLLM, GGUFFiles, LLMBackend};
use yantrikdb_ml::ApiLLM;

use slint::{Model, ModelRc, SharedString, VecModel};

use crate::{App, UrgeCardData};

/// Commands from the UI thread to the companion worker.
pub enum CompanionCommand {
    /// Send a message and receive streaming tokens.
    SendMessage {
        text: String,
        token_tx: Sender<String>,
    },
    /// Request bond state for the bond screen.
    GetBondState {
        reply_tx: Sender<BondSnapshot>,
    },
    /// Request personality/evolution data.
    GetEvolution {
        reply_tx: Sender<EvolutionSnapshot>,
    },
    /// Search memories.
    RecallMemories {
        query: String,
        reply_tx: Sender<Vec<MemoryResult>>,
    },
    /// Get current bond level (for voice profile adaptation).
    GetBondLevel {
        reply_tx: Sender<BondLevel>,
    },
    /// Request pending urges for display on Home screen.
    GetPendingUrges {
        reply_tx: Sender<Vec<UrgeSnapshot>>,
    },
    /// Record a system event in the companion's memory.
    RecordSystemEvent {
        text: String,
        domain: String,
        importance: f64,
    },
    /// Update the system context string for LLM prompt injection.
    SetSystemContext {
        context: String,
    },
    /// Store a periodic hourly system snapshot digest.
    RecordSnapshot {
        text: String,
    },
    /// Store a detected system issue as a persistent memory.
    RecordIssue {
        text: String,
        importance: f64,
        /// Decay constant in seconds. 0.0 = permanent.
        decay: f64,
    },
    /// Run a background think cycle.
    Think {
        /// Current interruptibility from FocusFlow (0.0 = deep work, 1.0 = normal).
        interruptibility: f32,
    },
    /// Shut down the worker.
    Shutdown,
}

/// Serializable state snapshot for the UI.
#[derive(Debug, Clone)]
pub struct StateSnapshot {
    pub memory_count: i64,
    pub has_pending_urges: bool,
}

/// Bond screen data.
#[derive(Debug, Clone)]
pub struct BondSnapshot {
    pub bond_score: f64,
    pub bond_level: String,
    pub total_interactions: i64,
    pub days_together: i64,
    pub current_streak: i64,
    pub humor_rate: f64,
    pub vulnerability_events: i64,
    pub shared_references: i64,
}

/// Personality/evolution screen data.
#[derive(Debug, Clone)]
pub struct EvolutionSnapshot {
    pub formality: f64,
    pub humor_ratio: f64,
    pub opinion_strength: f64,
    pub question_ratio: f64,
    pub opinions: Vec<OpinionItem>,
    pub shared_refs: Vec<SharedRefItem>,
}

#[derive(Debug, Clone)]
pub struct OpinionItem {
    pub topic: String,
    pub stance: String,
    pub confidence: f64,
}

#[derive(Debug, Clone)]
pub struct SharedRefItem {
    pub text: String,
    pub times_used: i64,
}

/// Memory search result for the UI.
#[derive(Debug, Clone)]
pub struct MemoryResult {
    pub rid: String,
    pub text: String,
    pub memory_type: String,
    pub importance: f64,
    pub valence: f64,
    pub score: f64,
    pub created_at: f64,
}

/// Urge data for the UI.
#[derive(Debug, Clone)]
pub struct UrgeSnapshot {
    pub urge_id: String,
    pub instinct_name: String,
    pub reason: String,
    pub urgency: f64,
    pub suggested_message: String,
    pub created_at: f64,
}

/// The bridge between Slint UI and the companion worker thread.
pub struct CompanionBridge {
    cmd_tx: Sender<CompanionCommand>,
    worker_handle: Option<std::thread::JoinHandle<()>>,
    /// Whether the LLM backend responded successfully on the last call.
    online: Arc<AtomicBool>,
    /// Cached bond level (1-5) — updated by worker thread, read by UI features.
    cached_bond_level: Arc<std::sync::atomic::AtomicU8>,
}

impl CompanionBridge {
    /// Start the companion worker thread.
    pub fn start(config: CompanionConfig, ui_weak: slint::Weak<App>) -> Self {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let online = Arc::new(AtomicBool::new(true));
        let online_w = online.clone();
        let cached_bond_level = Arc::new(std::sync::atomic::AtomicU8::new(1));
        let bond_w = cached_bond_level.clone();

        let worker_handle = std::thread::spawn(move || {
            worker_loop(config, cmd_rx, ui_weak, online_w, bond_w);
        });

        Self {
            cmd_tx,
            worker_handle: Some(worker_handle),
            online,
            cached_bond_level,
        }
    }

    /// Whether the LLM backend is reachable.
    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    /// Current bond level as u8 (1-5). Updated by worker thread, safe to read from UI thread.
    pub fn bond_level_cached(&self) -> u8 {
        self.cached_bond_level.load(Ordering::Relaxed)
    }

    /// Send a message and get a channel to receive streaming tokens.
    pub fn send_message(&self, text: String) -> Receiver<String> {
        let (token_tx, token_rx) = crossbeam_channel::unbounded();
        let _ = self.cmd_tx.send(CompanionCommand::SendMessage { text, token_tx });
        token_rx
    }

    /// Request bond data.
    pub fn request_bond(&self) -> Receiver<BondSnapshot> {
        let (reply_tx, reply_rx) = crossbeam_channel::unbounded();
        let _ = self.cmd_tx.send(CompanionCommand::GetBondState { reply_tx });
        reply_rx
    }

    /// Request personality/evolution data.
    pub fn request_evolution(&self) -> Receiver<EvolutionSnapshot> {
        let (reply_tx, reply_rx) = crossbeam_channel::unbounded();
        let _ = self.cmd_tx.send(CompanionCommand::GetEvolution { reply_tx });
        reply_rx
    }

    /// Search memories.
    pub fn recall_memories(&self, query: String) -> Receiver<Vec<MemoryResult>> {
        let (reply_tx, reply_rx) = crossbeam_channel::unbounded();
        let _ = self.cmd_tx.send(CompanionCommand::RecallMemories { query, reply_tx });
        reply_rx
    }

    /// Request current bond level.
    pub fn request_bond_level(&self) -> Receiver<BondLevel> {
        let (reply_tx, reply_rx) = crossbeam_channel::unbounded();
        let _ = self.cmd_tx.send(CompanionCommand::GetBondLevel { reply_tx });
        reply_rx
    }

    /// Request pending urges.
    pub fn request_pending_urges(&self) -> Receiver<Vec<UrgeSnapshot>> {
        let (reply_tx, reply_rx) = crossbeam_channel::unbounded();
        let _ = self.cmd_tx.send(CompanionCommand::GetPendingUrges { reply_tx });
        reply_rx
    }

    /// Record a system event in the companion's memory.
    pub fn record_system_event(&self, text: String, domain: String, importance: f64) {
        let _ = self.cmd_tx.send(CompanionCommand::RecordSystemEvent {
            text,
            domain,
            importance,
        });
    }

    /// Update the system context for LLM prompt injection.
    pub fn set_system_context(&self, context: String) {
        let _ = self.cmd_tx.send(CompanionCommand::SetSystemContext { context });
    }

    /// Store a periodic system snapshot digest.
    pub fn record_snapshot(&self, text: String) {
        let _ = self.cmd_tx.send(CompanionCommand::RecordSnapshot { text });
    }

    /// Store a detected system issue as a persistent memory.
    pub fn record_issue(&self, text: String, importance: f64, decay: f64) {
        let _ = self.cmd_tx.send(CompanionCommand::RecordIssue {
            text,
            importance,
            decay,
        });
    }

    /// Trigger a think cycle with current focus interruptibility.
    pub fn think(&self, interruptibility: f32) {
        let _ = self.cmd_tx.send(CompanionCommand::Think { interruptibility });
    }

    /// Shut down the worker thread.
    pub fn shutdown(&mut self) {
        let _ = self.cmd_tx.send(CompanionCommand::Shutdown);
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for CompanionBridge {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The worker thread's main loop.
fn worker_loop(
    config: CompanionConfig,
    cmd_rx: Receiver<CompanionCommand>,
    ui_weak: slint::Weak<App>,
    online: Arc<AtomicBool>,
    cached_bond: Arc<std::sync::atomic::AtomicU8>,
) {
    // Build companion on this thread (owns SQLite connection)
    let mut companion = build_companion(config);
    tracing::info!("Companion worker started");

    // Push initial state to UI
    push_state(&companion, &ui_weak, online.load(Ordering::Relaxed));

    // Sync initial bond level
    cached_bond.store(companion.bond_level().as_u8(), Ordering::Relaxed);

    loop {
        match cmd_rx.recv() {
            Ok(CompanionCommand::SendMessage { text, token_tx }) => {
                tracing::info!(text = %text, "Processing message");
                let start = std::time::Instant::now();
                let mut token_count = 0u32;
                let response = companion.handle_message_streaming(&text, |token| {
                    if token_count == 0 {
                        tracing::info!(
                            elapsed_ms = start.elapsed().as_millis(),
                            "First token generated"
                        );
                    }
                    token_count += 1;
                    let _ = token_tx.send(token.to_string());
                });

                // V22: Use response.offline_mode to track LLM status
                let ok = !response.offline_mode;
                online.store(ok, Ordering::Relaxed);
                if response.offline_mode {
                    tracing::info!("Response served by offline responder");
                }

                tracing::info!(
                    elapsed_ms = start.elapsed().as_millis(),
                    tokens = token_count,
                    online = ok,
                    offline_mode = response.offline_mode,
                    "Generation complete"
                );
                // Sentinel to indicate generation is done
                let _ = token_tx.send("__DONE__".to_string());

                // Push updated state (includes online status)
                push_state(&companion, &ui_weak, ok);
                // V15: Update cached bond level for UI features
                cached_bond.store(companion.bond_level().as_u8(), Ordering::Relaxed);
            }
            Ok(CompanionCommand::GetBondState { reply_tx }) => {
                let bond = BondTracker::get_state(companion.db.conn());
                let humor_rate = if bond.humor_attempts > 0 {
                    bond.humor_successes as f64 / bond.humor_attempts as f64
                } else {
                    0.0
                };
                let _ = reply_tx.send(BondSnapshot {
                    bond_score: bond.bond_score,
                    bond_level: bond.bond_level.name().to_string(),
                    total_interactions: bond.total_interactions,
                    days_together: bond.days_together as i64,
                    current_streak: bond.current_streak_days,
                    humor_rate,
                    vulnerability_events: bond.vulnerability_events,
                    shared_references: bond.shared_references,
                });
            }
            Ok(CompanionCommand::GetEvolution { reply_tx }) => {
                let style = Evolution::get_style(companion.db.conn());
                let opinions = Evolution::get_opinions(companion.db.conn(), 20);
                let refs = Evolution::get_shared_references(companion.db.conn(), 20);

                let _ = reply_tx.send(EvolutionSnapshot {
                    formality: style.formality,
                    humor_ratio: style.humor_ratio,
                    opinion_strength: style.opinion_strength,
                    question_ratio: style.question_ratio,
                    opinions: opinions
                        .into_iter()
                        .map(|o| OpinionItem {
                            topic: o.topic,
                            stance: o.stance,
                            confidence: o.confidence,
                        })
                        .collect(),
                    shared_refs: refs
                        .into_iter()
                        .map(|r| SharedRefItem {
                            text: r.reference_text,
                            times_used: r.times_used,
                        })
                        .collect(),
                });
            }
            Ok(CompanionCommand::RecallMemories { query, reply_tx }) => {
                match companion.db.recall_text(&query, 20) {
                    Ok(results) => {
                        let items: Vec<MemoryResult> = results
                            .into_iter()
                            .map(|r| MemoryResult {
                                rid: r.rid,
                                text: r.text,
                                memory_type: r.memory_type,
                                importance: r.importance,
                                valence: r.valence,
                                score: r.score,
                                created_at: r.created_at,
                            })
                            .collect();
                        let _ = reply_tx.send(items);
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Memory recall failed");
                        let _ = reply_tx.send(vec![]);
                    }
                }
            }
            Ok(CompanionCommand::GetBondLevel { reply_tx }) => {
                let _ = reply_tx.send(companion.bond_level());
            }
            Ok(CompanionCommand::GetPendingUrges { reply_tx }) => {
                let urges = companion.urge_queue.get_pending(companion.db.conn(), 10);
                let snapshots: Vec<UrgeSnapshot> = urges
                    .into_iter()
                    .map(|u| UrgeSnapshot {
                        urge_id: u.urge_id,
                        instinct_name: u.instinct_name,
                        reason: u.reason,
                        urgency: u.urgency,
                        suggested_message: u.suggested_message,
                        created_at: u.created_at,
                    })
                    .collect();
                let _ = reply_tx.send(snapshots);
            }
            Ok(CompanionCommand::RecordSystemEvent { text, domain, importance }) => {
                // Sanitize system event data before storing as memory.
                // System events come from D-Bus/inotify — external input.
                let safe_text: String = text.chars()
                    .filter(|c| !c.is_control() || *c == '\n')
                    .take(500)
                    .collect();
                let safe_importance = importance.clamp(0.0, 1.0);
                // Validate domain — only allow known prefixes
                let safe_domain = if domain.starts_with("system/") {
                    domain
                } else {
                    "system/general".to_string()
                };

                if let Err(e) = companion.db.record_text(
                    &safe_text,
                    "episodic",
                    safe_importance,
                    0.0,
                    604800.0,
                    &serde_json::json!({}),
                    "default",
                    0.9,
                    &safe_domain,
                    "system",
                    None,
                ) {
                    tracing::warn!(error = %e, "Failed to record system event");
                }

                // Buffer event for automation matching in think cycle
                companion.push_event(&safe_domain, serde_json::json!({
                    "text": safe_text,
                    "importance": safe_importance,
                }));
            }
            Ok(CompanionCommand::SetSystemContext { context }) => {
                // Poll background tasks and append summary to system context
                companion.poll_background_tasks();
                let task_summary = companion.active_tasks_summary();
                let sched_summary = yantrikdb_companion::scheduler::Scheduler::format_summary(companion.db.conn());
                let mut full_ctx = context;
                if !task_summary.is_empty() {
                    full_ctx.push('\n');
                    full_ctx.push_str(&task_summary);
                }
                if !sched_summary.is_empty() {
                    full_ctx.push('\n');
                    full_ctx.push_str(&sched_summary);
                }
                let auto_summary = yantrikdb_companion::automation::AutomationStore::format_summary(companion.db.conn());
                if !auto_summary.is_empty() {
                    full_ctx.push('\n');
                    full_ctx.push_str(&auto_summary);
                }
                companion.set_system_context(full_ctx);
            }
            Ok(CompanionCommand::RecordSnapshot { text }) => {
                let safe: String = text
                    .chars()
                    .filter(|c| !c.is_control() || *c == '\n')
                    .take(2000)
                    .collect();
                if let Err(e) = companion.db.record_text(
                    &safe,
                    "episodic",
                    0.6,
                    0.0,
                    604800.0, // 7-day decay
                    &serde_json::json!({}),
                    "default",
                    0.95,
                    "system/snapshot",
                    "system",
                    None,
                ) {
                    tracing::warn!(error = %e, "Failed to record system snapshot");
                } else {
                    tracing::debug!("Hourly system snapshot stored");
                }
            }
            Ok(CompanionCommand::RecordIssue {
                text,
                importance,
                decay,
            }) => {
                let safe: String = text
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(500)
                    .collect();
                let safe_importance = importance.clamp(0.0, 1.0);
                if let Err(e) = companion.db.record_text(
                    &safe,
                    "episodic",
                    safe_importance,
                    -0.3, // negative valence — it's a problem
                    decay,
                    &serde_json::json!({"issue": true}),
                    "default",
                    1.0, // max consolidation — never prune issues
                    "system/issue",
                    "system",
                    None,
                ) {
                    tracing::warn!(error = %e, "Failed to record system issue");
                } else {
                    tracing::info!(text = %safe, "System issue recorded to memory");
                }
            }
            Ok(CompanionCommand::Think { interruptibility }) => {
                let db = &companion.db;
                // Disable consolidation: yantrikdb-core's consolidate()
                // hardcodes domain="general",source="user" on merged records,
                // which destroys system/* domain info and creates compound
                // blobs. Keep triggers, patterns, and personality running.
                let config = yantrikdb_core::types::ThinkConfig {
                    run_consolidation: false,
                    ..Default::default()
                };
                if let Ok(result) = db.think(&config) {
                    // Convert triggers to JSON for instinct evaluation
                    let mut triggers: Vec<serde_json::Value> = result
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

                    // Fetch active patterns
                    let patterns: Vec<serde_json::Value> = db
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

                    // Count open conflicts
                    let conflicts_count = db
                        .get_conflicts(Some("open"), None, None, None, 100)
                        .map(|c| c.len())
                        .unwrap_or(0);

                    // Extract valence trend
                    let valence_avg = triggers
                        .iter()
                        .find(|t| {
                            t.get("trigger_type").and_then(|v| v.as_str())
                                == Some("valence_trend")
                        })
                        .and_then(|t| {
                            t.get("context")
                                .and_then(|c| c.get("current_avg"))
                                .and_then(|v| v.as_f64())
                        });

                    // Check scheduler for due tasks — inject as triggers
                    let due_tasks = yantrikdb_companion::scheduler::Scheduler::get_due(companion.db.conn());
                    for task in &due_tasks {
                        triggers.push(serde_json::json!({
                            "trigger_type": "scheduled_task",
                            "task_id": task.task_id,
                            "label": task.label,
                            "description": task.description,
                            "urgency": task.urgency,
                            "schedule_type": task.schedule_type,
                            "action": task.action,
                        }));
                        yantrikdb_companion::scheduler::Scheduler::advance(companion.db.conn(), &task.task_id);
                    }
                    if !due_tasks.is_empty() {
                        tracing::info!(count = due_tasks.len(), "Scheduler: advanced due tasks");
                    }

                    // V15: Serendipity — surface a random older memory as a connection trigger
                    if companion.bond_level() >= yantrikdb_companion::types::BondLevel::Friend {
                        if let Some(memory) = pick_serendipity_memory(&companion.db) {
                            triggers.push(serde_json::json!({
                                "trigger_type": "serendipity",
                                "memory_text": memory,
                            }));
                        }
                    }

                    tracing::info!(
                        triggers = triggers.len(),
                        patterns = patterns.len(),
                        conflicts = conflicts_count,
                        "Think cycle — caching cognition results"
                    );

                    // CRITICAL: Cache results so instincts can see them
                    companion.update_cognition_cache(
                        triggers,
                        patterns,
                        conflicts_count,
                        valence_avg,
                    );
                }

                // V22: Evaluate instincts + check proactive engine for messages
                let state = companion.build_state();
                let urge_specs = companion.evaluate_instincts(&state);

                let idle_hrs = (state.current_ts - state.last_interaction_ts) / 3600.0;
                tracing::info!(
                    urge_count = urge_specs.len(),
                    triggers = state.pending_triggers.len(),
                    patterns = state.active_patterns.len(),
                    idle_hours = %format!("{idle_hrs:.1}"),
                    "Instinct evaluation complete"
                );

                for spec in &urge_specs {
                    tracing::info!(
                        instinct = %spec.instinct_name,
                        urgency = spec.urgency,
                        reason = %spec.reason,
                        "Urge generated"
                    );
                    companion.urge_queue.push(companion.db.conn(), spec);
                }
                companion.check_proactive();

                // Deliver proactive message — gated on focus state
                if let Some(msg) = companion.take_proactive_message() {
                    if interruptibility < 0.5 {
                        // Deep work mode — suppress non-critical proactive messages
                        tracing::info!(
                            interruptibility,
                            text = msg.text,
                            "Suppressing proactive message (deep work mode)"
                        );
                    } else {
                        tracing::info!(
                            text = msg.text,
                            urges = ?msg.urge_ids,
                            "Delivering proactive message to UI"
                        );
                        let text = msg.text.clone();
                        let notif_text = msg.text.clone();
                        let weak = ui_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = weak.upgrade() {
                                // Add as assistant message in chat
                                let messages = ui.get_messages();
                                let model = messages
                                    .as_any()
                                    .downcast_ref::<VecModel<crate::MessageData>>()
                                    .unwrap();
                                model.push(crate::MessageData {
                                    role: SharedString::from("assistant"),
                                    content: SharedString::from(&text),
                                    is_streaming: false,
                                    blocks: ModelRc::default(),
                                });
                                // Also show notification
                                ui.set_notification_text(notif_text.into());
                                ui.set_show_notification(true);
                            }
                        });

                        // Forward proactive messages to Telegram
                        if companion.config.telegram.enabled && companion.config.telegram.forward_proactive {
                            if let Err(e) = yantrikdb_companion::telegram::send_message(
                                &companion.config.telegram, &msg.text,
                            ) {
                                tracing::warn!(error = %e, "Failed to forward proactive to Telegram");
                            }
                        }
                    }
                } else {
                    let pending = companion.urge_queue.count_pending(companion.db.conn());
                    tracing::debug!(
                        pending_urges = pending,
                        "No proactive message this cycle"
                    );
                }

                // Push updated urges to Home screen
                push_urges(&companion, &ui_weak);

                push_state(&companion, &ui_weak, online.load(Ordering::Relaxed));
            }
            Ok(CompanionCommand::Shutdown) | Err(_) => {
                tracing::info!("Companion worker shutting down");
                break;
            }
        }
    }
}

/// Push current state to the Slint UI thread.
fn push_state(companion: &CompanionService, ui_weak: &slint::Weak<App>, companion_online: bool) {
    let snapshot = build_snapshot(companion);
    let weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_memory_count(snapshot.memory_count as i32);
            ui.set_has_urges(snapshot.has_pending_urges);
            ui.set_companion_online(companion_online);
        }
    });
}

/// Build a state snapshot from the companion.
fn build_snapshot(companion: &CompanionService) -> StateSnapshot {
    let state = companion.build_state();
    StateSnapshot {
        memory_count: state.memory_count,
        has_pending_urges: !state.pending_triggers.is_empty(),
    }
}

/// Push pending urges to the Slint UI thread.
fn push_urges(companion: &CompanionService, ui_weak: &slint::Weak<App>) {
    let urges = companion.urge_queue.get_pending(companion.db.conn(), 10);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();

    let cards: Vec<UrgeCardData> = urges
        .iter()
        .map(|u| UrgeCardData {
            urge_id: u.urge_id.clone().into(),
            instinct_name: u.instinct_name.clone().into(),
            reason: u.reason.clone().into(),
            urgency: u.urgency as f32,
            suggested_message: u.suggested_message.clone().into(),
            time_ago: format_time_ago(now - u.created_at).into(),
            border_color: instinct_color(&u.instinct_name),
        })
        .collect();

    let weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_urges(ModelRc::new(VecModel::from(cards)));
        }
    });
}

/// Map instinct names to Firelight theme colors.
pub fn instinct_color(name: &str) -> slint::Color {
    match name {
        "check_in" => slint::Color::from_rgb_u8(0xD4, 0xA0, 0x3C),            // amber
        "emotional_awareness" => slint::Color::from_rgb_u8(0xE8, 0x6B, 0x6B),  // warm red
        "follow_up" => slint::Color::from_rgb_u8(0x6B, 0xB8, 0xE8),            // soft blue
        "reminder" => slint::Color::from_rgb_u8(0x8B, 0xE8, 0x6B),             // soft green
        "pattern_surfacing" => slint::Color::from_rgb_u8(0xC4, 0x8B, 0xE8),    // lavender
        "conflict_alerting" => slint::Color::from_rgb_u8(0xE8, 0xA0, 0x6B),    // orange
        "bond_milestone" => slint::Color::from_rgb_u8(0xE8, 0xD4, 0x6B),       // gold
        "self_awareness" => slint::Color::from_rgb_u8(0x6B, 0xE8, 0xC4),       // teal
        "humor" => slint::Color::from_rgb_u8(0xE8, 0x6B, 0xC4),                // pink
        _ => slint::Color::from_rgb_u8(0xD4, 0xA0, 0x3C),                      // default amber
    }
}

/// Format seconds-ago into a human-readable string.
pub fn format_time_ago(seconds: f64) -> String {
    if seconds < 60.0 {
        "just now".to_string()
    } else if seconds < 3600.0 {
        format!("{}m ago", (seconds / 60.0) as i64)
    } else if seconds < 86400.0 {
        format!("{}h ago", (seconds / 3600.0) as i64)
    } else {
        format!("{}d ago", (seconds / 86400.0) as i64)
    }
}

/// Build a CompanionService (same logic as crates/yantrik/src/main.rs).
fn build_companion(config: CompanionConfig) -> CompanionService {
    // Load embedder
    let embedder = if let Some(ref dir) = config.yantrikdb.embedder_model_dir {
        tracing::info!(dir, "Loading embedder from directory");
        CandleEmbedder::from_dir(std::path::Path::new(dir))
            .expect("failed to load embedder from directory")
    } else {
        tracing::info!("Downloading MiniLM embedder from HuggingFace Hub");
        CandleEmbedder::from_hub("sentence-transformers/all-MiniLM-L6-v2", None)
            .expect("failed to load embedder from hub")
    };

    // Load LLM — select backend based on config
    let llm: Box<dyn LLMBackend> = if config.llm.is_api_backend() {
        // API backend (Ollama, OpenAI, DeepSeek, vLLM, etc.)
        let base_url = config.llm.resolve_api_base_url()
            .expect("api_base_url required for API backend (set it or use a named provider like 'ollama')");
        let model = config.llm.api_model.as_deref()
            .expect("api_model required for API backend");
        tracing::info!(
            backend = config.llm.backend,
            base_url = %base_url,
            model,
            "Using API LLM backend"
        );
        Box::new(ApiLLM::new(base_url, config.llm.api_key.clone(), model))
    } else if let Some(ref dir) = config.llm.model_dir {
        tracing::info!(dir, "Loading Candle LLM from directory");
        Box::new(CandleLLM::from_dir(std::path::Path::new(dir))
            .expect("failed to load LLM from directory"))
    } else if let (Some(ref gguf), Some(ref tok)) =
        (&config.llm.gguf_path, &config.llm.tokenizer_path)
    {
        tracing::info!(gguf, tok, "Loading Candle LLM from explicit paths");
        Box::new(CandleLLM::from_gguf(std::path::Path::new(gguf), std::path::Path::new(tok))
            .expect("failed to load LLM"))
    } else {
        tracing::info!(
            repo = config.llm.hub_repo,
            gguf = config.llm.hub_gguf,
            "Downloading LLM from HuggingFace Hub"
        );
        let files = GGUFFiles::from_hub(
            &config.llm.hub_repo,
            &config.llm.hub_gguf,
            &config.llm.hub_tokenizer,
        )
        .expect("failed to download LLM");
        Box::new(CandleLLM::from_gguf(&files.gguf, &files.tokenizer).expect("failed to load LLM"))
    };

    // Create YantrikDB
    let mut db =
        yantrikdb_core::YantrikDB::new(&config.yantrikdb.db_path, config.yantrikdb.embedding_dim)
            .expect("failed to create YantrikDB");
    db.set_embedder(Box::new(embedder));

    tracing::info!(
        db_path = config.yantrikdb.db_path,
        user = config.user_name,
        "Companion initialized"
    );

    CompanionService::new(db, llm, config)
}

/// V15: Pick a random older memory for serendipity connections.
///
/// Uses semantic recall with a broad query to find diverse user memories,
/// then filters for older ones worth surfacing. Only fires ~10% of think
/// cycles to avoid spam.
fn pick_serendipity_memory(db: &yantrikdb_core::YantrikDB) -> Option<String> {
    // Only fire ~10% of think cycles to avoid spam
    let roll = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        % 10;
    if roll != 0 {
        return None;
    }

    // Query for older user memories (3+ days old, importance >= 0.4)
    let cutoff_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        - (3.0 * 86400.0);

    // Use a broad query to get diverse memories
    let memories = db.recall_text("things the user shared or talked about", 20).unwrap_or_default();

    // Filter to old enough and important enough
    let candidates: Vec<_> = memories
        .iter()
        .filter(|m| m.created_at < cutoff_ts && m.importance >= 0.4 && m.text.len() >= 10)
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // Pick a pseudo-random one based on subsec nanos
    let idx = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as usize
        % candidates.len();

    Some(candidates[idx].text.clone())
}
