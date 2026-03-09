//! Companion bridge — worker thread that owns CompanionService.
//!
//! The Slint UI thread sends commands via crossbeam channel.
//! The worker thread processes them sequentially and pushes
//! state updates back via slint::invoke_from_event_loop().

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::{Receiver, Sender};
use yantrik_companion::{CompanionConfig, CompanionService};
use yantrik_companion::bond::{BondLevel, BondTracker};
use yantrik_companion::evolution::Evolution;
use yantrik_ml::{CandleEmbedder, CandleLLM, GGUFFiles, LLMBackend};
use yantrik_ml::ApiLLM;
use yantrik_ml::{FallbackLLM, FallbackConfig};
#[cfg(feature = "llamacpp")]
use yantrik_ml::LlamaCppLLM;
#[cfg(feature = "claude-cli")]
use yantrik_ml::ClaudeCliLLM;

use slint::{Model, ModelRc, SharedString, VecModel};

use crate::ambient::AmbientState;
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
    /// Toggle incognito mode (no data persistence).
    SetIncognitoMode {
        enabled: bool,
    },
    /// Run a background think cycle.
    Think {
        /// Current interruptibility from FocusFlow (0.0 = deep work, 1.0 = normal).
        interruptibility: f32,
        /// Foreground window title (for cortex focus detection).
        window_title: String,
        /// Foreground process/app name (for cortex focus detection).
        process_name: String,
        /// User idle seconds (for cortex focus detection).
        idle_seconds: u64,
    },
    /// Process the next pending task from the queue.
    /// Self-signals after each step for continuous processing.
    ProcessNextTask,
    /// Execute the next step of a recipe. Self-signals for continuous execution.
    ProcessRecipeStep { recipe_id: String },
    /// Rename the user (persists to config).
    RenameUser { name: String },
    /// Rename the companion (persists to config).
    RenameCompanion { name: String },
    /// Request a structured morning brief for the desktop card.
    GetMorningBrief {
        reply_tx: Sender<MorningBriefSnapshot>,
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

/// Structured morning brief data for the desktop card.
#[derive(Debug, Clone)]
pub struct MorningBriefSnapshot {
    pub greeting: String,
    pub sections: Vec<MorningBriefSectionData>,
}

/// A single section of the morning brief card.
#[derive(Debug, Clone)]
pub struct MorningBriefSectionData {
    pub icon: String,
    pub label: String,
    pub content: String,
    pub expanded: bool,
    pub action_id: String,
}

/// The bridge between Slint UI and the companion worker thread.
pub struct CompanionBridge {
    cmd_tx: Sender<CompanionCommand>,
    worker_handle: Option<std::thread::JoinHandle<()>>,
    /// Whether the LLM backend responded successfully on the last call.
    online: Arc<AtomicBool>,
    /// Cached bond level (1-5) — updated by worker thread, read by UI features.
    cached_bond_level: Arc<std::sync::atomic::AtomicU8>,
    /// Ambient intelligence state — sentiment, cognitive load.
    ambient: AmbientState,
    /// Cognitive event bus — shared with the entire system.
    event_bus: yantrik_os::EventBus,
}

impl CompanionBridge {
    /// Start the companion worker thread.
    pub fn start(config: CompanionConfig, ui_weak: slint::Weak<App>, event_bus: yantrik_os::EventBus) -> Self {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let online = Arc::new(AtomicBool::new(true));
        let online_w = online.clone();
        let cached_bond_level = Arc::new(std::sync::atomic::AtomicU8::new(1));
        let bond_w = cached_bond_level.clone();
        let ambient = AmbientState::new();
        let ambient_w = ambient.clone();
        let bus_w = event_bus.clone();

        let self_tx = cmd_tx.clone();
        let worker_handle = std::thread::spawn(move || {
            worker_loop(config, cmd_rx, self_tx, ui_weak, online_w, bond_w, ambient_w, bus_w);
        });

        Self {
            cmd_tx,
            worker_handle: Some(worker_handle),
            online,
            cached_bond_level,
            ambient,
            event_bus,
        }
    }

    /// Access the cognitive event bus.
    pub fn event_bus(&self) -> &yantrik_os::EventBus {
        &self.event_bus
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
        if self.cmd_tx.send(CompanionCommand::SendMessage { text, token_tx: token_tx.clone() }).is_err() {
            tracing::error!("Companion worker thread is dead — cannot send message");
            let _ = token_tx.send("__REPLACE__".to_string());
            let _ = token_tx.send("Companion service crashed. Please restart the application.".to_string());
            let _ = token_tx.send("__DONE__".to_string());
        }
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

    /// Request a structured morning brief for the desktop card.
    pub fn request_morning_brief(&self) -> Receiver<MorningBriefSnapshot> {
        let (reply_tx, reply_rx) = crossbeam_channel::unbounded();
        let _ = self.cmd_tx.send(CompanionCommand::GetMorningBrief { reply_tx });
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

    /// Toggle incognito mode (no data persistence while active).
    pub fn set_incognito(&self, enabled: bool) {
        let _ = self.cmd_tx.send(CompanionCommand::SetIncognitoMode { enabled });
    }

    /// Rename the user at runtime (updates config + companion).
    pub fn rename_user(&self, name: String) {
        let _ = self.cmd_tx.send(CompanionCommand::RenameUser { name });
    }

    /// Rename the companion at runtime (updates config + companion).
    pub fn rename_companion(&self, name: String) {
        let _ = self.cmd_tx.send(CompanionCommand::RenameCompanion { name });
    }

    /// Get ambient intelligence state (sentiment, cognitive_load).
    pub fn ambient_state(&self) -> (f32, f32) {
        (self.ambient.sentiment(), self.ambient.cognitive_load())
    }

    /// Trigger a think cycle with current focus interruptibility and focus data.
    pub fn think(&self, interruptibility: f32, window_title: String, process_name: String, idle_seconds: u64) {
        let _ = self.cmd_tx.send(CompanionCommand::Think {
            interruptibility,
            window_title,
            process_name,
            idle_seconds,
        });
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
    cmd_tx: Sender<CompanionCommand>,
    ui_weak: slint::Weak<App>,
    online: Arc<AtomicBool>,
    cached_bond: Arc<std::sync::atomic::AtomicU8>,
    ambient: AmbientState,
    event_bus: yantrik_os::EventBus,
) {
    // Save config services before moving config into build_companion
    let config_services = config.enabled_services.clone();

    // Build companion on this thread (owns SQLite connection)
    let mut companion = build_companion(config);

    // Apply Skill Store snapshot — overrides services, filters instincts,
    // extends core tools based on enabled skills.
    {
        let skills_dir = if std::path::Path::new("/opt/yantrik/skills").exists() {
            std::path::PathBuf::from("/opt/yantrik/skills")
        } else {
            std::env::current_dir().unwrap_or_default().join("skills")
        };
        let snapshot = yantrik_companion::skills::load_skill_snapshot_with_services(&skills_dir, &config_services);
        tracing::info!(
            services = snapshot.enabled_services.len(),
            instincts = snapshot.enabled_instincts.len(),
            cortex_rules = snapshot.enabled_cortex_rules.len(),
            extra_tools = snapshot.extra_core_tools.len(),
            "Loaded Skill Store snapshot"
        );
        companion.apply_skill_snapshot(&snapshot);
    }

    // Attach cognitive event bus for tool execution tracing
    companion.set_event_bus(event_bus.clone());

    tracing::info!("Companion worker started");

    // Cooldown tracker for EXECUTE urges (key → last_fired_ts)
    let mut execute_cooldowns: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    const EXECUTE_COOLDOWN_SECS: f64 = 7200.0; // 2 hours between same EXECUTE urge
    // Fairness tracker and category budgets for tier-based urge selector
    let mut fairness = yantrik_companion::urge_selector::FairnessTracker::new();
    let mut budgets = yantrik_companion::urge_selector::CategoryBudget::new();

    // Global cortex cooldown — prevents ANY cortex message within this window.
    // Individual cortex patterns use hash-based keys, but similar-but-different
    // patterns can still spam. This global cooldown is the backstop.
    let mut last_cortex_fire_ts: f64 = 0.0;
    const CORTEX_GLOBAL_COOLDOWN_SECS: f64 = 3600.0; // 1 hour between ANY cortex messages

    // Cooldown tracker for delivered proactive messages (key → last_delivered_ts)
    let mut delivered_cooldowns: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    const DELIVERED_COOLDOWN_SECS: f64 = 7200.0; // 2 hours between same proactive key

    // Track last real user message timestamp (not EXECUTE/system messages)
    // Used for synthesis gate conversation activity detection
    let mut last_user_message_ts: f64 = 0.0;

    // Push initial state to UI
    push_state(&companion, &ui_weak, online.load(Ordering::Relaxed));

    // Sync initial bond level
    cached_bond.store(companion.bond_level().as_u8(), Ordering::Relaxed);

    // Resume any running/waiting recipes from before shutdown
    for rid in yantrik_companion::recipe::RecipeStore::get_resumable(companion.db.conn()) {
        tracing::info!(recipe_id = %rid, "Resuming recipe from previous session");
        let _ = cmd_tx.send(CompanionCommand::ProcessRecipeStep { recipe_id: rid });
    }

    loop {
        match cmd_rx.recv() {
            Ok(CompanionCommand::SendMessage { text, token_tx }) => {
                // Track real user message timestamp for synthesis gate
                // Skip system-generated prompts (startup brief, etc.)
                let is_system_generated = text.contains("You just started up")
                    || text.contains("EXECUTE ")
                    || text.starts_with("Reflect naturally")
                    || text.starts_with("Recall shared references");
                if !is_system_generated {
                    last_user_message_ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs_f64();
                }

                // Update ambient sentiment from user message
                ambient.update_from_message(&text);

                // Track user message length for conversational metabolism
                companion.track_user_msg_length(text.len());

                // Proactive message threading: if user replies shortly after a proactive message,
                // prepend context so the LLM knows what the conversation is about.
                let text = if let Some(ctx) = companion.get_threading_context() {
                    format!("{}{}", ctx, text)
                } else {
                    text
                };

                tracing::info!(text = %text, "Processing message");
                // Emit user message event
                let msg_trace = if !is_system_generated {
                    event_bus.emit(
                        yantrik_os::EventKind::UserMessage {
                            text: text.chars().take(200).collect(),
                            source: "chat".into(),
                        },
                        yantrik_os::EventSource::UserInterface,
                    )
                } else {
                    yantrik_os::TraceId::new() // no-op trace for system messages
                };

                // V18: Extract commitments from user messages
                if !is_system_generated {
                    let source = yantrik_companion::world_model::CommitmentSource::Conversation {
                        turn_id: Some(msg_trace.as_u64().to_string()),
                    };
                    let extracted = yantrik_companion::commitment_extractor::extract_commitments(
                        &text,
                        &companion.config.user_name,
                        &source,
                    );
                    if !extracted.is_empty() {
                        let wm_commits = yantrik_companion::commitment_extractor::to_world_model_commitments(
                            &extracted,
                            &companion.config.user_name,
                            source,
                        );
                        for c in &wm_commits {
                            yantrik_companion::world_model::WorldModel::insert_commitment(
                                companion.db.conn(), c,
                            );
                        }
                        tracing::info!(
                            count = wm_commits.len(),
                            "Extracted commitments from user message"
                        );
                    }
                }

                // Resonance Model: record user interaction (positive quality for now)
                {
                    let now_r = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs_f64();
                    companion.resonance.record_user_interaction(now_r, 0.8);
                    // Adaptive User Model: record user message (checks for pending proactive response)
                    let had_pending = companion.user_model.inner_pending_ts().is_some();
                    companion.user_model.on_user_message(now_r);
                    // V25: If user responded to a proactive message, record as "accepted"
                    if had_pending {
                        companion.record_proactive_outcome("proactive", yantrik_companion::silence_policy::InterventionOutcome::Accepted);
                    }
                }
                let start = std::time::Instant::now();
                let mut token_count = 0u32;

                // Wrap in catch_unwind so a panic in handle_message_streaming
                // doesn't kill the worker thread (which would make ALL future
                // messages silently fail).
                let result = {
                    let token_tx_ref = &token_tx;
                    let token_count_ref = &mut token_count;
                    let start_ref = &start;
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        companion.handle_message_streaming(&text, |token| {
                            if *token_count_ref == 0 {
                                tracing::info!(
                                    elapsed_ms = start_ref.elapsed().as_millis(),
                                    "First token generated"
                                );
                            }
                            *token_count_ref += 1;
                            let _ = token_tx_ref.send(token.to_string());
                        })
                    }))
                };

                match result {
                    Ok(response) => {
                        // V22: Use response.offline_mode to track LLM status
                        let ok = !response.offline_mode;
                        online.store(ok, Ordering::Relaxed);
                        if response.offline_mode {
                            tracing::info!("Response served by offline responder");
                        }

                        // Record significant interactions as events for aftermath instinct
                        // Skip system-generated prompts (startup brief, EXECUTE instructions)
                        let is_system_prompt = text.contains("You just started up")
                            || text.contains("EXECUTE ")
                            || text.starts_with("Reflect naturally")
                            || text.starts_with("Recall shared references");
                        if !response.tool_calls_made.is_empty() && !is_system_prompt {
                            let tools_summary = response.tool_calls_made.join(", ");
                            // Use user's message (truncated) as event description
                            let user_text = text.chars().take(80).collect::<String>();
                            let event_desc = if tools_summary.contains("run_command") {
                                format!("Ran commands for: {}", user_text)
                            } else if tools_summary.contains("browse") || tools_summary.contains("browser") {
                                format!("Browser session: {}", user_text)
                            } else if tools_summary.contains("write_file") {
                                format!("File editing: {}", user_text)
                            } else {
                                format!("Helped with: {}", user_text)
                            };
                            companion.record_event(&event_desc);
                        }

                        // Emit companion response event
                        event_bus.emit_with_parent(
                            yantrik_os::EventKind::CompanionResponse {
                                text_length: response.message.len(),
                                tool_calls_count: response.tool_calls_made.len(),
                                total_ms: start.elapsed().as_millis() as u64,
                            },
                            yantrik_os::EventSource::Companion,
                            msg_trace,
                        );

                        tracing::info!(
                            elapsed_ms = start.elapsed().as_millis(),
                            tokens = token_count,
                            online = ok,
                            offline_mode = response.offline_mode,
                            "Generation complete"
                        );
                    }
                    Err(panic_info) => {
                        let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                            s.to_string()
                        } else if let Some(s) = panic_info.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "unknown panic".to_string()
                        };
                        tracing::error!(
                            panic = %msg,
                            elapsed_ms = start.elapsed().as_millis(),
                            "Companion panicked during message handling — recovering"
                        );
                        let _ = token_tx.send("__REPLACE__".to_string());
                        let _ = token_tx.send(
                            "Something went wrong internally. Please try again.".to_string()
                        );
                    }
                }

                // Sentinel to indicate generation is done (sent in all cases)
                let _ = token_tx.send("__DONE__".to_string());

                // Push updated state (includes online status)
                push_state(&companion, &ui_weak, online.load(Ordering::Relaxed));
                // V15: Update cached bond level for UI features
                cached_bond.store(companion.bond_level().as_u8(), Ordering::Relaxed);

                // If tasks are pending (maybe user just queued one), signal the task processor
                if yantrik_companion::task_queue::TaskQueue::active_count(companion.db.conn()) > 0 {
                    let _ = cmd_tx.send(CompanionCommand::ProcessNextTask);
                }
                // If recipes are pending (maybe user just called run_recipe), signal them
                for rid in yantrik_companion::recipe::RecipeStore::get_resumable(companion.db.conn()) {
                    let _ = cmd_tx.send(CompanionCommand::ProcessRecipeStep { recipe_id: rid });
                }
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
            Ok(CompanionCommand::SetIncognitoMode { enabled }) => {
                companion.set_incognito(enabled);
                tracing::info!(incognito = enabled, "Incognito mode toggled");
            }
            Ok(CompanionCommand::RecordSystemEvent { text, domain, importance }) => {
                if companion.is_incognito() {
                    tracing::debug!("Incognito: skipping RecordSystemEvent");
                } else {
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
            }
            Ok(CompanionCommand::SetSystemContext { context }) => {
                // Poll background tasks — returns notifications for completed tasks
                let bg_notifications = companion.poll_background_tasks();
                for notif in &bg_notifications {
                    tracing::info!(text = %notif, "Background task completed — sending notification");
                    let text = notif.clone();
                    let weak = ui_weak.clone();
                    let notif_text = if notif.len() > 100 {
                        format!("{}...", &notif[..notif.floor_char_boundary(97)])
                    } else {
                        notif.clone()
                    };
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak.upgrade() {
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
                            crate::wire::toast::push_toast(
                                &weak, "Task Complete", &notif_text, "", 1,
                            );
                        }
                    });
                    // Forward to Telegram
                    if companion.config.telegram.enabled && companion.config.telegram.forward_proactive {
                        let _ = yantrik_companion::telegram::send_message(
                            &companion.config.telegram, notif,
                        );
                    }
                }
                let task_summary = companion.active_tasks_summary();
                let sched_summary = yantrik_companion::scheduler::Scheduler::format_summary(companion.db.conn());
                let mut full_ctx = context;
                if !task_summary.is_empty() {
                    full_ctx.push('\n');
                    full_ctx.push_str(&task_summary);
                }
                if !sched_summary.is_empty() {
                    full_ctx.push('\n');
                    full_ctx.push_str(&sched_summary);
                }
                let auto_summary = yantrik_companion::automation::AutomationStore::format_summary(companion.db.conn());
                if !auto_summary.is_empty() {
                    full_ctx.push('\n');
                    full_ctx.push_str(&auto_summary);
                }
                // Task queue summary — shows pending/in-progress persistent tasks
                let tq_summary = yantrik_companion::task_queue::TaskQueue::format_active_summary(companion.db.conn());
                if !tq_summary.is_empty() {
                    full_ctx.push('\n');
                    full_ctx.push_str(&tq_summary);
                }
                companion.set_system_context(full_ctx);
            }
            Ok(CompanionCommand::RecordSnapshot { text }) => {
                if !companion.is_incognito() {
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
            }
            Ok(CompanionCommand::RecordIssue {
                text,
                importance,
                decay,
            }) => {
                if !companion.is_incognito() {
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
            }
            Ok(CompanionCommand::Think { interruptibility, window_title, process_name, idle_seconds }) => {
                // Update Context Cortex focus from system signals
                if let Some(ref mut cortex) = companion.cortex {
                    cortex.update_focus(companion.db.conn(), &window_title, &process_name, idle_seconds);
                }

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
                    let due_tasks = yantrik_companion::scheduler::Scheduler::get_due(companion.db.conn());
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
                        yantrik_companion::scheduler::Scheduler::advance(companion.db.conn(), &task.task_id);
                    }
                    if !due_tasks.is_empty() {
                        tracing::info!(count = due_tasks.len(), "Scheduler: advanced due tasks");
                    }

                    // V24: Open Loops Monitor — scan commitments + attention items
                    {
                        let monitor_config = yantrik_companion::open_loops_monitor::MonitorConfig::default();
                        let scan_result = yantrik_companion::open_loops_monitor::scan(
                            companion.db.conn(),
                            &monitor_config,
                        );

                        // Inject commitment triggers for instinct evaluation
                        let overdue = yantrik_companion::world_model::WorldModel::overdue_commitments(companion.db.conn());
                        for c in &overdue {
                            triggers.push(serde_json::json!({
                                "trigger_type": "commitment_overdue",
                                "commitment_id": c.id,
                                "action": c.action,
                                "promisor": c.promisor,
                                "promisee": c.promisee,
                                "urgency": 0.8,
                            }));
                            event_bus.emit(
                                yantrik_os::EventKind::CommitmentAlert {
                                    commitment_id: c.id.to_string(),
                                    description: c.action.clone(),
                                    alert_type: yantrik_os::CommitmentAlertType::Overdue,
                                },
                                yantrik_os::EventSource::Companion,
                            );
                        }

                        let approaching = yantrik_companion::world_model::WorldModel::approaching_deadlines(companion.db.conn(), 24.0);
                        for c in &approaching {
                            triggers.push(serde_json::json!({
                                "trigger_type": "commitment_approaching",
                                "commitment_id": c.id,
                                "action": c.action,
                                "promisor": c.promisor,
                                "promisee": c.promisee,
                                "deadline": c.deadline,
                                "urgency": 0.6,
                            }));
                            event_bus.emit(
                                yantrik_os::EventKind::CommitmentAlert {
                                    commitment_id: c.id.to_string(),
                                    description: c.action.clone(),
                                    alert_type: yantrik_os::CommitmentAlertType::Approaching,
                                },
                                yantrik_os::EventSource::Companion,
                            );
                        }

                        if scan_result.overdue_threads > 0 || scan_result.approaching_threads > 0
                            || scan_result.attention_threads > 0
                        {
                            tracing::info!(
                                overdue = scan_result.overdue_threads,
                                approaching = scan_result.approaching_threads,
                                attention = scan_result.attention_threads,
                                "Open loops monitor: scan complete"
                            );
                        }
                    }

                    // V15: Serendipity — surface a random older memory as a connection trigger
                    if companion.bond_level() >= yantrik_companion::types::BondLevel::Friend {
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
                // Resonance Model: tick phase dynamics each think cycle
                companion.resonance.tick_phase(companion.bond_level(), 60.0);

                // V25: Refresh trust state + apply daily interaction + daily decay
                companion.refresh_trust_state();
                yantrik_companion::trust_model::TrustModel::apply_event(
                    companion.db.conn(),
                    &yantrik_companion::trust_model::TrustEvent::DailyInteraction,
                );
                yantrik_companion::trust_model::TrustModel::apply_daily_decay(companion.db.conn());

                let state = companion.build_state();
                let mut urge_specs = companion.evaluate_instincts(&state);
                let now_ts = state.current_ts;

                // Context Cortex think step — run rules + baselines + patterns
                if let Some(ref mut cortex) = companion.cortex {
                    let attention_items = cortex.think(companion.db.conn());
                    // Global cortex cooldown: suppress ALL cortex urges within 1 hour
                    // of the last cortex message. Prevents similar-but-different patterns
                    // from spamming the user every think cycle.
                    let cortex_globally_cooled = now_ts - last_cortex_fire_ts < CORTEX_GLOBAL_COOLDOWN_SECS;

                    if !attention_items.is_empty() && !cortex_globally_cooled {
                        let focus = cortex.current_focus();
                        let briefing = cortex.build_briefing(companion.db.conn(), focus.as_ref(), &attention_items);
                        tracing::info!(
                            attention_count = attention_items.len(),
                            "Cortex attention items fired"
                        );
                        // Hash the attention item summaries into the cooldown key
                        // so the same pattern doesn't fire every think cycle.
                        let mut attention_hasher = std::collections::hash_map::DefaultHasher::new();
                        for item in &attention_items {
                            std::hash::Hash::hash(&item.summary, &mut attention_hasher);
                        }
                        let hash_val = std::hash::Hasher::finish(&attention_hasher);
                        let cortex_cooldown = format!("cortex:situation:{:x}", hash_val);

                        let cortex_urge = yantrik_companion::UrgeSpec::new(
                            "Cortex",
                            &format!("EXECUTE {}", briefing),
                            0.7,
                        )
                        .with_cooldown(&cortex_cooldown);
                        urge_specs.push(cortex_urge);
                    } else if cortex_globally_cooled && !attention_items.is_empty() {
                        tracing::debug!(
                            attention_count = attention_items.len(),
                            "Cortex attention suppressed by global cooldown"
                        );
                    }

                    // LLM Reasoner — deep reflection every ~4 hours
                    if let Some(reflection_prompt) = cortex.maybe_deep_reflection(companion.db.conn()) {
                        tracing::info!("Cortex LLM reasoner: triggering deep reflection");
                        let reasoner_urge = yantrik_companion::UrgeSpec::new(
                            "CortexReasoner",
                            &reflection_prompt,
                            0.6,
                        )
                        .with_cooldown("cortex:deep_reflection");
                        urge_specs.push(reasoner_urge);
                    }
                }

                // ── Playbook Engine: deterministic anticipatory actions ──
                // Playbooks bypass the urge queue — they fire directly as
                // notifications when conviction + evidence thresholds are met.
                {
                    let cortex_focus = companion.cortex.as_ref()
                        .and_then(|c| c.current_focus());
                    // Gather attention items for playbook state (re-run is cheap)
                    let pb_attention = companion.cortex.as_ref()
                        .map(|_| {
                            // Use empty attention — playbooks query DB directly
                            Vec::new()
                        })
                        .unwrap_or_default();

                    let pb_now = state.current_ts;
                    let pb_state = yantrik_companion::cortex::playbook::PlaybookState {
                        attention_items: &pb_attention,
                        current_focus: cortex_focus.as_ref(),
                        now_ts: pb_now,
                        user_hour: (pb_now as i64 % 86400 / 3600) as u32,
                        conn: companion.db.conn(),
                        bond_level: companion.bond_level() as u8,
                    };

                    let user_receptivity = companion.user_model.engagement();
                    let pb_actions = companion.playbook_engine.evaluate(&pb_state, user_receptivity);

                    // Check timeouts for pending playbook outcomes
                    companion.playbook_engine.check_timeouts(pb_now);

                    for action in &pb_actions {
                        match action {
                            yantrik_companion::cortex::playbook::CortexAction::Notify {
                                title, body, explanation, playbook_id,
                            } => {
                                tracing::info!(
                                    playbook = %playbook_id,
                                    title = %title,
                                    "Playbook Notify action"
                                );
                                // Deliver as proactive message
                                let text = format!("{}\n{}", title, body);
                                let msg = yantrik_companion::types::ProactiveMessage {
                                    text,
                                    urge_ids: vec![format!("playbook:{}", playbook_id)],
                                    generated_at: pb_now,
                                };
                                companion.set_proactive_message(msg);
                            }
                            yantrik_companion::cortex::playbook::CortexAction::QueueTask {
                                description, playbook_id,
                            } => {
                                tracing::info!(
                                    playbook = %playbook_id,
                                    description = %description,
                                    "Playbook QueueTask action"
                                );
                                yantrik_companion::task_queue::TaskQueue::enqueue(
                                    companion.db.conn(),
                                    &format!("Playbook: {}", playbook_id),
                                    description,
                                    0, // normal priority
                                    "playbook_engine",
                                );
                            }
                            yantrik_companion::cortex::playbook::CortexAction::SuggestTool {
                                tool_name, explanation, playbook_id, ..
                            } => {
                                tracing::info!(
                                    playbook = %playbook_id,
                                    tool = %tool_name,
                                    "Playbook SuggestTool action"
                                );
                                let text = format!("Suggestion: {}", explanation);
                                let msg = yantrik_companion::types::ProactiveMessage {
                                    text,
                                    urge_ids: vec![format!("playbook:{}", playbook_id)],
                                    generated_at: pb_now,
                                };
                                companion.set_proactive_message(msg);
                            }
                        }
                    }

                    // Save playbook conviction state periodically (every 10 think cycles)
                    if companion.playbook_engine.diagnostic_summary().contains("fires=") {
                        companion.playbook_engine.save(companion.db.conn());
                    }
                }

                let idle_hrs = (state.current_ts - state.last_interaction_ts) / 3600.0;

                // Emit instinct evaluation events for significant firings
                for spec in &urge_specs {
                    if spec.urgency >= 0.5 {
                        event_bus.emit(
                            yantrik_os::EventKind::InstinctFired {
                                instinct_name: spec.instinct_name.clone(),
                                urge_count: 1,
                                max_urgency: spec.urgency,
                            },
                            yantrik_os::EventSource::ProactiveEngine,
                        );
                    }
                }

                tracing::info!(
                    urge_count = urge_specs.len(),
                    triggers = state.pending_triggers.len(),
                    patterns = state.active_patterns.len(),
                    idle_hours = %format!("{idle_hrs:.1}"),
                    "Instinct evaluation complete"
                );

                // Separate EXECUTE urges (need LLM processing) from regular urges
                let mut execute_urges = Vec::new();
                let now_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64();

                // Daily budget check — skip all urges if over budget
                let over_budget = companion.is_over_daily_budget();
                if over_budget {
                    tracing::info!(
                        count = companion.daily_proactive_count,
                        budget = companion.daily_message_budget(),
                        "Daily proactive budget exceeded, suppressing all urges"
                    );
                }

                for spec in &urge_specs {
                    tracing::info!(
                        instinct = %spec.instinct_name,
                        urgency = spec.urgency,
                        reason = %spec.reason,
                        "Urge generated"
                    );
                    if over_budget {
                        companion.record_suppressed_urge(&spec.cooldown_key, "daily budget exceeded");
                        continue;
                    }
                    if spec.reason.starts_with("EXECUTE ") {
                        // Check cooldown with jitter
                        let last = execute_cooldowns.get(&spec.cooldown_key).copied().unwrap_or(0.0);
                        let jittered_cooldown = yantrik_companion::synthesis_gate::jitter_cooldown(EXECUTE_COOLDOWN_SECS);
                        if now_ts - last < jittered_cooldown {
                            tracing::debug!(
                                key = %spec.cooldown_key,
                                "EXECUTE urge on cooldown, skipping"
                            );
                            continue;
                        }
                        execute_urges.push(spec.clone());
                    } else {
                        companion.urge_queue.push(companion.db.conn(), spec);
                    }
                }

                // Process EXECUTE urges through tier-based log-linear softmax selector.
                let mut execute_produced_message = false;

                // v4: Suppress EXECUTE urges when LLM is offline.
                if !execute_urges.is_empty() && !online.load(Ordering::Relaxed) {
                    tracing::info!(
                        count = execute_urges.len(),
                        "Suppressing EXECUTE urges — LLM offline"
                    );
                    execute_urges.clear();
                }

                if !execute_urges.is_empty() {
                    let recent_msgs = companion.last_sent_messages(10).to_vec();
                    let bond = companion.bond_level();

                    if let Some((exec_urge, serendipity)) = yantrik_companion::urge_selector::process_execute_urges(
                        &execute_urges,
                        &mut fairness,
                        &mut budgets,
                        &companion.resonance,
                        &companion.user_model,
                        &recent_msgs,
                        bond,
                        now_ts,
                    ) {
                        // Cooldown the picked urge
                        execute_cooldowns.insert(exec_urge.cooldown_key.clone(), now_ts);

                        tracing::info!(
                            count = execute_urges.len(),
                            picked = %exec_urge.instinct_name,
                            urgency = exec_urge.urgency,
                            serendipity = serendipity,
                            tier = exec_urge.time_sensitivity.tier(),
                            category = ?exec_urge.category,
                            "Processing EXECUTE urge (tier-softmax selector)"
                        );

                        let mut instruction = exec_urge.reason.trim_start_matches("EXECUTE ").to_string();
                        let anti_rep = yantrik_companion::synthesis_gate::anti_repetition_instruction(
                            companion.last_sent_messages(5),
                            state.avg_user_msg_length,
                        );
                        instruction.push_str(&anti_rep);

                        let instinct_name = exec_urge.instinct_name.clone();
                        if instinct_name == "Cortex" || instinct_name == "CortexReasoner" {
                            last_cortex_fire_ts = now_ts;
                        }
                        tracing::info!(
                            instinct = %instinct_name,
                            instruction = %instruction,
                            "Executing research urge"
                        );
                        let resp = companion.handle_message(&instruction);
                        let response_text = resp.message.trim().to_string();
                        if !response_text.is_empty() && !is_nothing_response(&response_text) {
                            let msg = yantrik_companion::types::ProactiveMessage {
                                text: response_text,
                                urge_ids: vec![exec_urge.cooldown_key.clone()],
                                generated_at: now_ts,
                            };
                            companion.set_proactive_message(msg);
                            execute_produced_message = true;
                            tracing::info!(
                                instinct = %instinct_name,
                                "EXECUTE urge produced proactive message"
                            );
                        } else {
                            tracing::info!(
                                instinct = %instinct_name,
                                "EXECUTE urge returned no actionable content"
                            );
                        }
                    } else {
                        tracing::info!("All EXECUTE urges filtered by tier-softmax selector");
                    }
                }

                // ── Task Queue Signal ──
                // If no EXECUTE urges fired and there are pending tasks,
                // signal the event-driven task processor to start working.
                if !execute_produced_message && !over_budget {
                    if yantrik_companion::task_queue::TaskQueue::active_count(companion.db.conn()) > 0 {
                        let _ = cmd_tx.send(CompanionCommand::ProcessNextTask);
                    }
                }

                // Only run proactive engine if EXECUTE didn't already produce a message
                if !execute_produced_message {
                    companion.check_proactive();
                }

                // Deliver proactive message — Synthesis Gate + focus state + per-key cooldown
                if let Some(msg) = companion.take_proactive_message() {
                    // Per-key cooldown: don't deliver same urge key within 2 hours
                    let delivery_key = msg.urge_ids.first().cloned().unwrap_or_default();
                    let jittered_delivery_cd = yantrik_companion::synthesis_gate::jitter_cooldown(DELIVERED_COOLDOWN_SECS);
                    let on_cooldown = if !delivery_key.is_empty() {
                        let last = delivered_cooldowns.get(&delivery_key).copied().unwrap_or(0.0);
                        now_ts - last < jittered_delivery_cd
                    } else { false };

                    // Synthesis Gate: check similarity, budget, conversation state
                    // Use last_user_message_ts (real user messages only) for activity check,
                    // not session_turn_count which gets bumped by EXECUTE urges too
                    let user_idle_secs = now_ts - last_user_message_ts;
                    let conversation_active = last_user_message_ts > 0.0 && user_idle_secs < 300.0;
                    let gate_result = yantrik_companion::synthesis_gate::evaluate(
                        &msg.text,
                        companion.last_sent_messages(10),
                        companion.daily_proactive_count,
                        companion.bond_level(),
                        state.idle_seconds,
                        conversation_active,
                        0.5, // default urgency for template messages
                    );

                    let gate_suppressed = matches!(gate_result, yantrik_companion::synthesis_gate::GateDecision::Suppress { .. });
                    if let yantrik_companion::synthesis_gate::GateDecision::Suppress { ref reason } = gate_result {
                        tracing::info!(
                            reason = %reason,
                            text = msg.text,
                            "Synthesis Gate suppressed proactive message"
                        );
                        companion.record_suppressed_urge(&delivery_key, reason);
                    }

                    if on_cooldown {
                        tracing::info!(
                            key = %delivery_key,
                            "Proactive message on per-key cooldown, skipping"
                        );
                        companion.record_suppressed_urge(&delivery_key, "per-key cooldown");
                        event_bus.emit(
                            yantrik_os::EventKind::ProactiveSuppressed {
                                reason: "per-key cooldown".into(),
                                urge_ids: msg.urge_ids.clone(),
                            },
                            yantrik_os::EventSource::ProactiveEngine,
                        );
                    } else if gate_suppressed {
                        // Already logged above
                        event_bus.emit(
                            yantrik_os::EventKind::ProactiveSuppressed {
                                reason: "synthesis gate".into(),
                                urge_ids: msg.urge_ids.clone(),
                            },
                            yantrik_os::EventSource::ProactiveEngine,
                        );
                    } else if interruptibility < 0.5 {
                        tracing::info!(
                            interruptibility,
                            text = msg.text,
                            "Suppressing proactive message (deep work mode)"
                        );
                        companion.record_suppressed_urge(&delivery_key, "deep work mode");
                        // V25: Record suppression as "ignored" for silence policy learning
                        companion.record_proactive_outcome(&delivery_key, yantrik_companion::silence_policy::InterventionOutcome::Ignored);
                        event_bus.emit(
                            yantrik_os::EventKind::ProactiveSuppressed {
                                reason: "deep work mode".into(),
                                urge_ids: msg.urge_ids.clone(),
                            },
                            yantrik_os::EventSource::ProactiveEngine,
                        );
                    } else {
                        tracing::info!(
                            text = msg.text,
                            urges = ?msg.urge_ids,
                            daily_count = companion.daily_proactive_count,
                            "Delivering proactive message to UI"
                        );

                        // Record sent message for anti-repetition tracking
                        companion.record_sent_message(&msg.text);
                        companion.record_proactive_context(&msg.text, msg.urge_ids.clone());

                        // Resonance Model: record sent for fatigue/phase tracking
                        let instinct_name = msg.urge_ids.first()
                            .map(|s| s.split(':').next().unwrap_or("unknown").to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        companion.resonance.record_sent(&instinct_name, now_ts);

                        // Adaptive User Model: record proactive send (starts pending response tracking)
                        let category = yantrik_companion::resonance::InstinctCategory::from_instinct(&instinct_name);
                        companion.user_model.on_proactive_sent(&instinct_name, &format!("{:?}", category), now_ts);

                        // Update global cortex cooldown if this was a cortex-originated message
                        if instinct_name == "Cortex" || instinct_name == "CortexReasoner" {
                            last_cortex_fire_ts = now_ts;
                        }

                        let text = msg.text.clone();
                        let notif_text = msg.text.clone();
                        let weak = ui_weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = weak.upgrade() {
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
                                crate::wire::toast::push_toast(
                                    &weak,
                                    "Companion",
                                    &notif_text,
                                    "",
                                    1,
                                );
                            }
                        });

                        // Forward proactive messages to Telegram
                        if companion.config.telegram.enabled && companion.config.telegram.forward_proactive {
                            if let Err(e) = yantrik_companion::telegram::send_message(
                                &companion.config.telegram, &msg.text,
                            ) {
                                tracing::warn!(error = %e, "Failed to forward proactive to Telegram");
                            }
                        }

                        // Emit proactive delivered event
                        event_bus.emit(
                            yantrik_os::EventKind::ProactiveDelivered {
                                urge_ids: msg.urge_ids.clone(),
                                text_preview: msg.text.chars().take(100).collect(),
                                delivery_channel: "chat".into(),
                            },
                            yantrik_os::EventSource::ProactiveEngine,
                        );

                        // Record per-key cooldown after delivery
                        if !delivery_key.is_empty() {
                            delivered_cooldowns.insert(delivery_key, now_ts);
                        }
                    }
                } else {
                    let pending = companion.urge_queue.count_pending(companion.db.conn());
                    tracing::debug!(
                        pending_urges = pending,
                        "No proactive message this cycle"
                    );
                }

                // Adaptive User Model: check for ignored proactive messages (2h timeout)
                {
                    let m = companion.user_model.inner_pending_ts();
                    if let Some(sent_ts) = m {
                        if now_ts - sent_ts > 2.0 * 3600.0 {
                            companion.user_model.on_proactive_ignored(now_ts);
                            // V25: Record as "ignored" in silence policy
                            companion.record_proactive_outcome("proactive", yantrik_companion::silence_policy::InterventionOutcome::Ignored);
                            tracing::info!("UserModel: proactive message expired (2h without response)");
                        }
                    }
                }

                // Periodic save of adaptive user model (every think cycle is fine — it's cheap)
                companion.user_model.save(companion.db.conn());

                // Push updated urges to Home screen
                push_urges(&companion, &ui_weak);

                push_state(&companion, &ui_weak, online.load(Ordering::Relaxed));
            }
            Ok(CompanionCommand::ProcessNextTask) => {
                // Event-driven task processing: process one step, then self-signal
                // to continue. User messages have priority — they arrive via the same
                // channel and will be processed before queued ProcessNextTask signals.

                // Check budget first
                let now_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64();
                let over_budget = companion.is_over_daily_budget();

                if over_budget {
                    tracing::debug!("Task processing skipped — over daily budget");
                } else if let Some(task) = yantrik_companion::task_queue::TaskQueue::next_task(companion.db.conn()) {
                    tracing::info!(
                        task_id = %task.task_id,
                        title = %task.title,
                        status = %task.status.as_str(),
                        steps = task.steps_completed,
                        "Processing queued task"
                    );

                    // Build a prompt with task context
                    let task_prompt = if task.progress.is_empty() {
                        format!(
                            "You have a queued task to work on.\n\
                             Task ID: {}\n\
                             Title: {}\n\
                             Description: {}\n\n\
                             Start working on this task NOW. Use tools to do the actual work. \
                             When you make progress, call update_task with a summary. \
                             When completely done, call complete_task with the result.",
                            task.task_id, task.title, task.description
                        )
                    } else {
                        format!(
                            "Continue working on your queued task.\n\
                             Task ID: {}\n\
                             Title: {}\n\
                             Progress so far: {}\n\
                             Steps completed: {}\n\n\
                             Continue where you left off. Use tools to do the actual work. \
                             Call update_task when you make more progress, or complete_task when done.",
                            task.task_id, task.title, task.progress, task.steps_completed
                        )
                    };

                    // Mark as in_progress
                    yantrik_companion::task_queue::TaskQueue::update_progress(
                        companion.db.conn(), &task.task_id,
                        &if task.progress.is_empty() { "Starting...".to_string() } else { task.progress.clone() },
                        task.steps_completed,
                    );

                    let resp = companion.handle_message(&task_prompt);
                    let response_text = resp.message.trim().to_string();

                    // Check if task was completed
                    let was_completed = resp.tool_calls_made.iter().any(|t| t == "complete_task");

                    if was_completed {
                        let notify_text = format!("Task completed: {}\n\n{}", task.title, response_text);
                        let msg = yantrik_companion::types::ProactiveMessage {
                            text: notify_text,
                            urge_ids: vec![format!("task_queue:{}", task.task_id)],
                            generated_at: now_ts,
                        };
                        companion.set_proactive_message(msg);
                        tracing::info!(
                            task_id = %task.task_id,
                            tools = %resp.tool_calls_made.join(", "),
                            "Queued task completed — notifying user"
                        );
                    } else if !resp.tool_calls_made.is_empty() {
                        tracing::info!(
                            task_id = %task.task_id,
                            tools = %resp.tool_calls_made.join(", "),
                            "Queued task progress — {} tools called",
                            resp.tool_calls_made.len()
                        );
                    }

                    // Self-signal: if there are more tasks or this one isn't done yet,
                    // continue processing. The signal goes to the back of the channel
                    // queue, so any pending user messages get processed first.
                    if !was_completed || yantrik_companion::task_queue::TaskQueue::active_count(companion.db.conn()) > 0 {
                        let _ = cmd_tx.send(CompanionCommand::ProcessNextTask);
                    }
                } else {
                    tracing::debug!("No tasks in queue");
                }
            }
            Ok(CompanionCommand::ProcessRecipeStep { recipe_id }) => {
                use yantrik_companion::recipe::*;

                let recipe = match RecipeStore::get(companion.db.conn(), &recipe_id) {
                    Some(r) => r,
                    None => {
                        tracing::warn!(recipe_id = %recipe_id, "Recipe not found");
                        continue;
                    }
                };

                // Skip if recipe is done/failed
                if recipe.status == RecipeStatus::Done || recipe.status == RecipeStatus::Failed {
                    continue;
                }

                let steps = RecipeStore::get_steps(companion.db.conn(), &recipe_id);
                let step_count = steps.len();

                // Check if we've finished all steps
                if recipe.current_step >= step_count {
                    RecipeStore::update_status(companion.db.conn(), &recipe_id, &RecipeStatus::Done, recipe.current_step);
                    tracing::info!(recipe_id = %recipe_id, name = %recipe.name, "Recipe completed");
                    // Notify user
                    let msg = yantrik_companion::types::ProactiveMessage {
                        text: format!("Recipe completed: {}", recipe.name),
                        urge_ids: vec![format!("recipe:{}", recipe_id)],
                        generated_at: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64(),
                    };
                    companion.set_proactive_message(msg);
                    continue;
                }

                // Mark as running
                RecipeStore::update_status(companion.db.conn(), &recipe_id, &RecipeStatus::Running, recipe.current_step);

                let stored_step = &steps[recipe.current_step];
                let mut vars = RecipeStore::get_vars(companion.db.conn(), &recipe_id);
                let step_idx = recipe.current_step;

                tracing::info!(
                    recipe_id = %recipe_id,
                    step = step_idx,
                    step_type = ?std::mem::discriminant(&stored_step.step),
                    "Executing recipe step"
                );

                match &stored_step.step {
                    RecipeStep::Tool { tool_name, args, store_as, on_error } => {
                        // Direct tool execution — NO LLM CALL
                        let resolved_args = resolve_vars_in_json(args, &vars);
                        let result = companion.execute_tool_direct(tool_name, &resolved_args);

                        let is_error = result.starts_with("Unknown tool:")
                            || result.starts_with("Permission denied:")
                            || result.starts_with("Failed")
                            || result.starts_with("Error");

                        if is_error {
                            tracing::warn!(
                                recipe_id = %recipe_id, step = step_idx,
                                tool = %tool_name, error = %result,
                                "Recipe tool step failed"
                            );
                            match on_error {
                                ErrorAction::Fail => {
                                    RecipeStore::fail_step(companion.db.conn(), &recipe_id, step_idx, &result);
                                    RecipeStore::set_error(companion.db.conn(), &recipe_id, &result);
                                    continue; // Don't self-signal
                                }
                                ErrorAction::Skip => {
                                    RecipeStore::skip_step(companion.db.conn(), &recipe_id, step_idx);
                                    RecipeStore::update_status(companion.db.conn(), &recipe_id, &RecipeStatus::Running, step_idx + 1);
                                }
                                ErrorAction::Retry { max } => {
                                    // Simple retry — re-send same step
                                    let retry_key = format!("_retry_{}", step_idx);
                                    let retries = vars.get(&retry_key)
                                        .and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                                    if retries < *max {
                                        RecipeStore::set_var(companion.db.conn(), &recipe_id, &retry_key,
                                            &serde_json::Value::Number((retries + 1).into()));
                                        // Don't advance step — retry
                                    } else {
                                        RecipeStore::fail_step(companion.db.conn(), &recipe_id, step_idx, &result);
                                        RecipeStore::set_error(companion.db.conn(), &recipe_id,
                                            &format!("Failed after {} retries: {}", max, result));
                                        continue;
                                    }
                                }
                                ErrorAction::JumpTo { step } => {
                                    RecipeStore::fail_step(companion.db.conn(), &recipe_id, step_idx, &result);
                                    RecipeStore::update_status(companion.db.conn(), &recipe_id, &RecipeStatus::Running, *step);
                                }
                            }
                        } else {
                            // Success — store result and advance
                            let result_json = serde_json::Value::String(result.clone());
                            RecipeStore::set_var(companion.db.conn(), &recipe_id, store_as, &result_json);
                            RecipeStore::complete_step(companion.db.conn(), &recipe_id, step_idx, &result);
                            RecipeStore::update_status(companion.db.conn(), &recipe_id, &RecipeStatus::Running, step_idx + 1);
                        }

                        // Self-signal to continue
                        let _ = cmd_tx.send(CompanionCommand::ProcessRecipeStep { recipe_id: recipe_id.clone() });
                    }

                    RecipeStep::Think { prompt, store_as } => {
                        // LLM call — resolve variables in prompt
                        let resolved_prompt = resolve_vars(prompt, &vars);
                        let resp = companion.handle_message(&resolved_prompt);
                        let response_text = resp.message.trim().to_string();

                        let result_json = serde_json::Value::String(response_text.clone());
                        RecipeStore::set_var(companion.db.conn(), &recipe_id, store_as, &result_json);
                        RecipeStore::complete_step(companion.db.conn(), &recipe_id, step_idx, &response_text);
                        RecipeStore::update_status(companion.db.conn(), &recipe_id, &RecipeStatus::Running, step_idx + 1);

                        // Self-signal to continue
                        let _ = cmd_tx.send(CompanionCommand::ProcessRecipeStep { recipe_id: recipe_id.clone() });
                    }

                    RecipeStep::JumpIf { condition, target_step } => {
                        // Pure Rust evaluation — NO LLM
                        if condition.evaluate(&vars) {
                            tracing::info!(recipe_id = %recipe_id, step = step_idx, target = target_step, "JumpIf: jumping");
                            RecipeStore::complete_step(companion.db.conn(), &recipe_id, step_idx, "jumped");
                            RecipeStore::update_status(companion.db.conn(), &recipe_id, &RecipeStatus::Running, *target_step);
                        } else {
                            RecipeStore::complete_step(companion.db.conn(), &recipe_id, step_idx, "continued");
                            RecipeStore::update_status(companion.db.conn(), &recipe_id, &RecipeStatus::Running, step_idx + 1);
                        }

                        // Self-signal to continue
                        let _ = cmd_tx.send(CompanionCommand::ProcessRecipeStep { recipe_id: recipe_id.clone() });
                    }

                    RecipeStep::WaitFor { condition, timeout_secs } => {
                        // Pause recipe — will be resumed by trigger check
                        RecipeStore::complete_step(companion.db.conn(), &recipe_id, step_idx, "waiting");
                        RecipeStore::update_status(companion.db.conn(), &recipe_id, &RecipeStatus::Waiting, step_idx + 1);
                        tracing::info!(recipe_id = %recipe_id, step = step_idx, "Recipe waiting");
                        // Don't self-signal — will be resumed by Think cycle trigger check
                    }

                    RecipeStep::Notify { message } => {
                        let resolved_msg = resolve_vars(message, &vars);
                        let msg = yantrik_companion::types::ProactiveMessage {
                            text: resolved_msg,
                            urge_ids: vec![format!("recipe:{}", recipe_id)],
                            generated_at: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64(),
                        };
                        companion.set_proactive_message(msg);
                        RecipeStore::complete_step(companion.db.conn(), &recipe_id, step_idx, "notified");
                        RecipeStore::update_status(companion.db.conn(), &recipe_id, &RecipeStatus::Running, step_idx + 1);

                        // Self-signal to continue
                        let _ = cmd_tx.send(CompanionCommand::ProcessRecipeStep { recipe_id: recipe_id.clone() });
                    }
                }
            }
            Ok(CompanionCommand::GetMorningBrief { reply_tx }) => {
                // Build structured brief from active day context sections
                let user_name = &companion.config.user_name;
                let greeting = format!("Good morning, {}!", user_name);
                let sections: Vec<MorningBriefSectionData> = companion
                    .active_context
                    .sections_by_priority()
                    .into_iter()
                    .filter(|s| s.id != "time") // Skip raw time — not useful in card
                    .map(|s| {
                        let icon = match s.id.as_str() {
                            "weather" => "🌤",
                            "calendar" | "next_event" => "📅",
                            "email" => "📧",
                            "alert" => "⚠",
                            "people" | "relationship" => "💬",
                            "finance" => "💰",
                            "news" => "📰",
                            "health" => "🫀",
                            _ => "✦",
                        };
                        let expanded = matches!(
                            s.priority,
                            yantrik_companion::active_context::ContextPriority::Critical
                            | yantrik_companion::active_context::ContextPriority::High
                        );
                        let action_id = match s.id.as_str() {
                            "weather" => "navigate:weather".to_string(),
                            "calendar" | "next_event" => "navigate:calendar".to_string(),
                            "email" => "navigate:email".to_string(),
                            _ => String::new(),
                        };
                        MorningBriefSectionData {
                            icon: icon.to_string(),
                            label: s.label.clone(),
                            content: s.content.clone(),
                            expanded,
                            action_id,
                        }
                    })
                    .collect();

                let snapshot = if sections.is_empty() {
                    MorningBriefSnapshot {
                        greeting,
                        sections: vec![MorningBriefSectionData {
                            icon: "☀".into(),
                            label: "Today".into(),
                            content: "Looks like a quiet day ahead. Enjoy it!".into(),
                            expanded: true,
                            action_id: String::new(),
                        }],
                    }
                } else {
                    MorningBriefSnapshot { greeting, sections }
                };

                let _ = reply_tx.send(snapshot);
            }
            Ok(CompanionCommand::RenameUser { name }) => {
                companion.config.user_name = name.clone();
                tracing::info!(user_name = %name, "User renamed at runtime");
            }
            Ok(CompanionCommand::RenameCompanion { name }) => {
                companion.config.personality.name = name.clone();
                tracing::info!(companion_name = %name, "Companion renamed at runtime");
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
/// Check if an EXECUTE urge response is a "nothing found" message that should be suppressed.
fn is_nothing_response(text: &str) -> bool {
    let lower = text.to_lowercase();
    let patterns = [
        "no major news",
        "nothing significant",
        "nothing to share",
        "nothing to report",
        "nothing interesting",
        "nothing noteworthy",
        "no need for a full briefing",
        "no breaking news",
        "nothing earth-shattering",
        "nothing genuinely",
        "nothing stands out",
        "no trending",
        "nothing worth",
        "no developments",
    ];
    patterns.iter().any(|p| lower.contains(p))
}

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
    let primary_llm: std::sync::Arc<dyn LLMBackend> = if config.llm.is_claude_cli_backend() {
        // Claude Code CLI backend — uses `claude -p` for inference
        let model = config.llm.api_model.clone();
        let max_tokens = config.llm.max_tokens;
        tracing::info!(
            model = ?model,
            "Using Claude Code CLI backend"
        );
        #[cfg(feature = "claude-cli")]
        { std::sync::Arc::new(ClaudeCliLLM::new(model, max_tokens)) }
        #[cfg(not(feature = "claude-cli"))]
        { panic!("claude-cli feature not enabled at compile time") }
    } else if config.llm.is_api_backend() {
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
        std::sync::Arc::new(ApiLLM::new(base_url, config.llm.api_key.clone(), model))
    } else if config.llm.backend == "llamacpp" {
        let gguf = config.llm.gguf_path.as_deref()
            .expect("gguf_path required for llamacpp backend");
        let gpu_layers = config.llm.fallback.as_ref()
            .map(|f| f.n_gpu_layers).unwrap_or(99);
        let ctx_size = config.llm.fallback.as_ref()
            .map(|f| f.context_size).unwrap_or(4096);
        tracing::info!(gguf, gpu_layers, ctx_size, "Using llama.cpp backend");
        #[cfg(feature = "llamacpp")]
        { std::sync::Arc::new(LlamaCppLLM::from_gguf(
            std::path::Path::new(gguf), gpu_layers, ctx_size,
        ).expect("failed to load llama.cpp model")) }
        #[cfg(not(feature = "llamacpp"))]
        { panic!("llamacpp feature not enabled at compile time") }
    } else if let Some(ref dir) = config.llm.model_dir {
        tracing::info!(dir, "Loading Candle LLM from directory");
        std::sync::Arc::new(CandleLLM::from_dir(std::path::Path::new(dir))
            .expect("failed to load LLM from directory"))
    } else if let (Some(ref gguf), Some(ref tok)) =
        (&config.llm.gguf_path, &config.llm.tokenizer_path)
    {
        tracing::info!(gguf, tok, "Loading Candle LLM from explicit paths");
        std::sync::Arc::new(CandleLLM::from_gguf(std::path::Path::new(gguf), std::path::Path::new(tok))
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
        std::sync::Arc::new(CandleLLM::from_gguf(&files.gguf, &files.tokenizer).expect("failed to load LLM"))
    };

    // Wrap with fallback if configured
    let llm: std::sync::Arc<dyn LLMBackend> = if let Some(ref fb_config) = config.llm.fallback {
        let fallback_cfg = match fb_config.backend.as_str() {
            "llamacpp" => {
                let path = fb_config.model_path.as_deref()
                    .expect("model_path required for llamacpp fallback");
                tracing::info!(model = path, gpu_layers = fb_config.n_gpu_layers, "Fallback: llama.cpp");
                FallbackConfig::LlamaCpp {
                    model_path: std::path::PathBuf::from(path),
                    n_gpu_layers: fb_config.n_gpu_layers,
                    context_size: fb_config.context_size,
                }
            }
            _ => {
                let url = fb_config.api_base_url.as_deref()
                    .expect("api_base_url required for API fallback");
                let model = fb_config.api_model.as_deref().unwrap_or("default");
                tracing::info!(url, model, "Fallback: API");
                FallbackConfig::Api {
                    base_url: url.to_string(),
                    model: model.to_string(),
                }
            }
        };
        std::sync::Arc::new(FallbackLLM::new(primary_llm, Some(fallback_cfg)))
    } else {
        primary_llm
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
