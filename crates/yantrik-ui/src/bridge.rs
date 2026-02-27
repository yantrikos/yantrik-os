//! Companion bridge — worker thread that owns CompanionService.
//!
//! The Slint UI thread sends commands via crossbeam channel.
//! The worker thread processes them sequentially and pushes
//! state updates back via slint::invoke_from_event_loop().

use crossbeam_channel::{Receiver, Sender};
use yantrikdb_companion::{CompanionConfig, CompanionService};
use yantrikdb_companion::bond::{BondLevel, BondTracker};
use yantrikdb_companion::evolution::Evolution;
use yantrikdb_ml::{CandleEmbedder, CandleLLM, GGUFFiles};

use slint::{ModelRc, VecModel};

use crate::{App, UrgeCardData};

/// Commands from the UI thread to the companion worker.
pub enum CompanionCommand {
    /// Send a message and receive streaming tokens.
    SendMessage {
        text: String,
        token_tx: Sender<String>,
    },
    /// Request a state snapshot.
    GetState {
        reply_tx: Sender<StateSnapshot>,
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
    /// Run a background think cycle.
    Think,
    /// Shut down the worker.
    Shutdown,
}

/// Serializable state snapshot for the UI.
#[derive(Debug, Clone)]
pub struct StateSnapshot {
    pub memory_count: i64,
    pub bond_level: String,
    pub bond_score: f64,
    pub session_active: bool,
    pub conversation_turns: usize,
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
}

impl CompanionBridge {
    /// Start the companion worker thread.
    pub fn start(config: CompanionConfig, ui_weak: slint::Weak<App>) -> Self {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();

        let worker_handle = std::thread::spawn(move || {
            worker_loop(config, cmd_rx, ui_weak);
        });

        Self {
            cmd_tx,
            worker_handle: Some(worker_handle),
        }
    }

    /// Send a message and get a channel to receive streaming tokens.
    pub fn send_message(&self, text: String) -> Receiver<String> {
        let (token_tx, token_rx) = crossbeam_channel::unbounded();
        let _ = self.cmd_tx.send(CompanionCommand::SendMessage { text, token_tx });
        token_rx
    }

    /// Request a state snapshot.
    pub fn request_state(&self) -> Receiver<StateSnapshot> {
        let (reply_tx, reply_rx) = crossbeam_channel::unbounded();
        let _ = self.cmd_tx.send(CompanionCommand::GetState { reply_tx });
        reply_rx
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

    /// Trigger a think cycle.
    pub fn think(&self) {
        let _ = self.cmd_tx.send(CompanionCommand::Think);
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
) {
    // Build companion on this thread (owns SQLite connection)
    let mut companion = build_companion(config);
    tracing::info!("Companion worker started");

    // Push initial state to UI
    push_state(&companion, &ui_weak);

    loop {
        match cmd_rx.recv() {
            Ok(CompanionCommand::SendMessage { text, token_tx }) => {
                tracing::info!(text = %text, "Processing message");
                let start = std::time::Instant::now();
                let mut token_count = 0u32;
                let _response = companion.handle_message_streaming(&text, |token| {
                    if token_count == 0 {
                        tracing::info!(
                            elapsed_ms = start.elapsed().as_millis(),
                            "First token generated"
                        );
                    }
                    token_count += 1;
                    let _ = token_tx.send(token.to_string());
                });
                tracing::info!(
                    elapsed_ms = start.elapsed().as_millis(),
                    tokens = token_count,
                    "Generation complete"
                );
                // Sentinel to indicate generation is done
                let _ = token_tx.send("__DONE__".to_string());

                // Push updated state
                push_state(&companion, &ui_weak);
            }
            Ok(CompanionCommand::GetState { reply_tx }) => {
                let state = build_snapshot(&companion);
                let _ = reply_tx.send(state);
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
            Ok(CompanionCommand::Think) => {
                let db = &companion.db;
                let config = yantrikdb_core::types::ThinkConfig::default();
                if let Ok(result) = db.think(&config) {
                    tracing::debug!(
                        triggers = result.triggers.len(),
                        "Think cycle complete"
                    );
                }

                // Check for proactive message
                if let Some(msg) = companion.take_proactive_message() {
                    let text = msg.text.clone();
                    let weak = ui_weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_notification_text(text.into());
                            ui.set_show_notification(true);
                        }
                    });
                }

                // Push updated urges to Home screen
                push_urges(&companion, &ui_weak);

                push_state(&companion, &ui_weak);
            }
            Ok(CompanionCommand::Shutdown) | Err(_) => {
                tracing::info!("Companion worker shutting down");
                break;
            }
        }
    }
}

/// Push current state to the Slint UI thread.
fn push_state(companion: &CompanionService, ui_weak: &slint::Weak<App>) {
    let snapshot = build_snapshot(companion);
    let weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_memory_count(snapshot.memory_count as i32);
            ui.set_is_thinking(false);
            ui.set_has_urges(snapshot.has_pending_urges);
        }
    });
}

/// Build a state snapshot from the companion.
fn build_snapshot(companion: &CompanionService) -> StateSnapshot {
    let state = companion.build_state();
    StateSnapshot {
        memory_count: state.memory_count,
        bond_level: format!("{:?}", state.bond_level),
        bond_score: state.bond_score,
        session_active: state.session_active,
        conversation_turns: state.conversation_turn_count,
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
fn format_time_ago(seconds: f64) -> String {
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

    // Load LLM
    let llm = if let Some(ref dir) = config.llm.model_dir {
        tracing::info!(dir, "Loading LLM from directory");
        CandleLLM::from_dir(std::path::Path::new(dir))
            .expect("failed to load LLM from directory")
    } else if let (Some(ref gguf), Some(ref tok)) =
        (&config.llm.gguf_path, &config.llm.tokenizer_path)
    {
        tracing::info!(gguf, tok, "Loading LLM from explicit paths");
        CandleLLM::from_gguf(std::path::Path::new(gguf), std::path::Path::new(tok))
            .expect("failed to load LLM")
    } else {
        tracing::info!("Downloading Qwen2.5-0.5B from HuggingFace Hub");
        let files = GGUFFiles::from_hub(
            "Qwen/Qwen2.5-0.5B-Instruct-GGUF",
            "qwen2.5-0.5b-instruct-q4_k_m.gguf",
            "Qwen/Qwen2.5-0.5B-Instruct",
        )
        .expect("failed to download LLM");
        CandleLLM::from_gguf(&files.gguf, &files.tokenizer).expect("failed to load LLM")
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

    CompanionService::new(db, Box::new(llm), config)
}
