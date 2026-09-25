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
use crate::{App, BondData, UrgeCardData};

/// What the worker streams in place of an answer when the built-in panicked mid-turn.
///
/// Named so the chat wiring can tell it from an answer: a `__REPLACE__` followed by this text is
/// a turn the person was not talked to, and `wire::chat` does not count it toward the bond — the
/// same rule the harness path applies to a `Chunk::Failed`.
pub const TURN_FAILED_REPLY: &str = "Something went wrong internally. Please try again.";

/// Why an [`CompanionHandle::ask`] did not produce an answer.
///
/// `NoModel` is the case callers over RPC need told apart: when no model is reachable the
/// offline responder serves the turn with plausible-looking canned text, and an app that
/// cannot tell a fallback from an answer shows the fallback as the model's words — Documents
/// offered to replace a document with it. The canned text does not leave the shell.
#[derive(Debug)]
pub enum AskError {
    /// The shell answered, and no model did — none set up, or the one set up did not answer.
    NoModel,
    /// No answer at all: no worker, a timeout, a failure on the way, or a turn that panicked
    /// mid-stream — the reason then carries its failure text.
    Failed(String),
}

/// Commands from the UI thread to the companion worker.
pub enum CompanionCommand {
    /// Send a message and receive streaming tokens.
    SendMessage {
        text: String,
        token_tx: Sender<String>,
        /// The board ticket this belongs to, when it was submitted rather than blocked on.
        ///
        /// Carried on the command rather than tracked by the submitter, because only the worker
        /// knows when a job actually *starts* — everything before that is queueing, and a caller
        /// told "running" while its request sits in a channel has been told the one thing it
        /// most needs to be right.
        job: Option<String>,
        /// Where to say whether a model produced the answer, for callers that must not show a
        /// fallback as one.
        ///
        /// Only [`CompanionHandle::ask`] passes one. The chat UI shows the offline notice from
        /// its own wiring, and a submitted job's subscriber is the board.
        model: Option<Sender<bool>>,
    },
    /// Count a conversation turn — one that began with the person's words and was answered.
    ///
    /// Whichever mind answered. The `SendMessage` arm scores nothing: it also carries the
    /// startup brief, EXECUTE urges and the companion's own reflection prompts, and a prompt
    /// the machine sent itself is not a conversation. `wire::chat::dispatch` sends this when a
    /// turn it started ends answered, and this is how it reaches the bond store, which lives on
    /// this thread.
    ScoreConversationTurn { text: String },
    /// Reload the LLM backend from a new provider config.
    /// Used when user adds/edits a provider in settings.
    ReloadLLM {
        provider_type: String,
        base_url: String,
        api_key: Option<String>,
        model: String,
    },
    /// Request bond state for the bond screen.
    GetBondState {
        reply_tx: Sender<BondSnapshot>,
    },
    /// Request personality/evolution data.
    GetEvolution {
        reply_tx: Sender<EvolutionSnapshot>,
    },
    /// Run one tool by name, without a model in the loop.
    RunTool {
        name: String,
        args: serde_json::Value,
        reply_tx: Sender<String>,
        job: Option<String>,
    },
    /// List the tools an outside caller may run.
    ListTools {
        reply_tx: Sender<serde_json::Value>,
    },
    /// Protect, open, re-wrap or read the credential vault.
    ///
    /// Separate from `RunTool` on purpose. `RunTool` takes a name and a `serde_json::Value`, and
    /// everything that reaches it — every mind, every caller on the shell's socket — can compose
    /// one. This variant takes a typed `vault_unlock::Op` that nothing deserialises, so the only
    /// way a passphrase gets onto this channel is Rust code the shell compiled, and there are two
    /// such places: the login wiring, and the callback behind the shell's own unlock prompt.
    Vault {
        op: crate::vault_unlock::Op,
        reply_tx: Sender<crate::vault_unlock::Reply>,
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
    /// Read the recipes again and publish them to `crate::recipes`, which the Recipes screen,
    /// `describe shell` and the mind panel read without waiting on this thread.
    RefreshRecipes,
    /// A person's answer, pause, resume or cancel for one recipe. The outcome is published to
    /// `crate::recipes` with the recipes themselves.
    Recipe {
        recipe_id: String,
        op: yantrik_companion::recipe_view::RecipeOp,
    },
    /// Start a run of a recipe with its inputs (`recipe_templates::start`). `leave`: the person's
    /// leave for its agents — from the Recipes screen's Start or the shell's `run_recipe`, which
    /// asked them — without which a formation is refused. The outcome goes to `reply_tx` when a
    /// caller waits on it, and to `crate::recipes` as the screen's notice either way.
    StartRecipe {
        recipe: String,
        variables: serde_json::Map<String, serde_json::Value>,
        leave: Option<yantrik_companion::recipe::Leave>,
        reply_tx: Option<Sender<Result<StartedRecipe, String>>>,
    },
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
    board: crate::jobs::Board,
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

/// A companion you can use from another thread.
///
/// `CompanionBridge` is built around the UI: it owns the worker's join handle and is held in the
/// AppContext. This is the part that travels — a channel to the worker and the online flag —
/// so the RPC server can answer other processes without touching the UI at all.
#[derive(Clone)]
pub struct CompanionHandle {
    cmd_tx: Sender<CompanionCommand>,
    online: Arc<AtomicBool>,
    board: crate::jobs::Board,
}

impl CompanionHandle {
    /// Ask the companion something and wait for the finished answer.
    ///
    /// The worker streams tokens for the chat UI; a caller over RPC wants one reply, so the
    /// stream is drained here. `__REPLACE__` means the next token supersedes everything so far,
    /// which is how the worker reports an error mid-stream.
    ///
    /// A turn the offline responder served comes back as [`AskError::NoModel`], not as its
    /// canned text: the caller would show that text as the answer, and apps acted on it. A
    /// turn that panicked comes back as [`AskError::Failed`] carrying the failure text, for
    /// the same reason: `Ok` from here is always a model's words.
    pub fn ask(&self, prompt: String, timeout: std::time::Duration) -> Result<String, AskError> {
        let (token_tx, token_rx) = crossbeam_channel::unbounded();
        // The worker sends whether a model produced the turn just before the end-of-turn
        // sentinel, so by the time the stream above is over this signal has arrived.
        let (model_tx, model_rx) = crossbeam_channel::bounded(1);
        self.cmd_tx
            .send(CompanionCommand::SendMessage {
                text: prompt,
                token_tx,
                job: None,
                model: Some(model_tx),
            })
            .map_err(|_| AskError::Failed("companion worker is not running".to_string()))?;

        let deadline = std::time::Instant::now() + timeout;
        let mut answer = String::new();
        let mut replace_next = false;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err(AskError::Failed("companion timed out".to_string()));
            }
            match token_rx.recv_timeout(left) {
                Ok(token) if token == "__DONE__" => break,
                Ok(token) if token == "__REPLACE__" => replace_next = true,
                Ok(token) => {
                    if replace_next {
                        answer = token;
                        replace_next = false;
                    } else {
                        answer.push_str(&token);
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    return Err(AskError::Failed("companion timed out".to_string()))
                }
                // The sender went away: whatever arrived is all there is.
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
        match model_rx.try_recv() {
            Ok(true) => Ok(answer),
            Ok(false) => Err(AskError::NoModel),
            // No signal: the turn never reached the point where the worker knows — it panicked
            // mid-stream, and the text above is TURN_FAILED_REPLY. That is a report about the
            // turn, not the model's words, and an `Ok` would put it in a document.
            Err(_) => Err(AskError::Failed(answer)),
        }
    }

    /// Search the companion's memory.
    pub fn recall(
        &self,
        query: String,
        timeout: std::time::Duration,
    ) -> Result<Vec<MemoryResult>, String> {
        let (reply_tx, reply_rx) = crossbeam_channel::unbounded();
        self.cmd_tx
            .send(CompanionCommand::RecallMemories { query, reply_tx })
            .map_err(|_| "companion worker is not running".to_string())?;
        reply_rx
            .recv_timeout(timeout)
            .map_err(|_| "companion timed out".to_string())
    }

    /// Run one tool by name and get its result.
    ///
    /// The 178 tools were reachable only by persuading a language model to choose one. This is the
    /// same registry, called by name, with the same permission ceiling and the same audit trail —
    /// and it works when the model does not.
    ///
    /// The worker is single-threaded, so this queues behind whatever it is already doing: an
    /// answer in progress, a think cycle. That is why the timeout is the caller's to choose.
    pub fn tool(
        &self,
        name: String,
        args: serde_json::Value,
        timeout: std::time::Duration,
    ) -> Result<String, String> {
        let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
        self.cmd_tx
            .send(CompanionCommand::RunTool { name, args, reply_tx, job: None })
            .map_err(|_| "companion worker is not running".to_string())?;
        reply_rx
            .recv_timeout(timeout)
            .map_err(|_| "the companion did not finish the tool in time".to_string())
    }

    /// The tools an outside caller may run, within the configured permission ceiling.
    pub fn tools(&self, timeout: std::time::Duration) -> Result<serde_json::Value, String> {
        let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
        self.cmd_tx
            .send(CompanionCommand::ListTools { reply_tx })
            .map_err(|_| "companion worker is not running".to_string())?;
        reply_rx
            .recv_timeout(timeout)
            .map_err(|_| "the companion did not answer in time".to_string())
    }

    /// Ask for something and get a ticket, not an answer.
    ///
    /// Returns in about a millisecond whatever the companion is doing, along with the two facts a
    /// caller can act on: how many jobs are in front of it, and how busy the lane is. The
    /// alternative — blocking — told the caller nothing for as long as fifty seconds and then
    /// gave it no way to have chosen differently.
    pub fn submit_ask(&self, text: String) -> Result<crate::jobs::Receipt, String> {
        let receipt = self.board.submit("model", "ask");
        // Tokens go nowhere: the board is the subscriber for a submitted job, and the chat UI is
        // not watching this one.
        let (token_tx, _token_rx) = crossbeam_channel::unbounded();
        self.cmd_tx
            .send(CompanionCommand::SendMessage {
                text,
                token_tx,
                job: Some(receipt.ticket.clone()),
                model: None,
            })
            .map_err(|_| "companion worker is not running".to_string())?;
        Ok(receipt)
    }

    /// The same, for a tool.
    pub fn submit_tool(
        &self,
        name: String,
        args: serde_json::Value,
    ) -> Result<crate::jobs::Receipt, String> {
        let receipt = self.board.submit("model", &format!("tool:{name}"));
        let (reply_tx, _reply_rx) = crossbeam_channel::unbounded();
        self.cmd_tx
            .send(CompanionCommand::RunTool {
                name,
                args,
                reply_tx,
                job: Some(receipt.ticket.clone()),
            })
            .map_err(|_| "companion worker is not running".to_string())?;
        Ok(receipt)
    }

    /// Ask the worker to publish the recipes again. Returns at once; see `crate::recipes`.
    pub fn refresh_recipes(&self) -> Result<(), String> {
        self.cmd_tx
            .send(CompanionCommand::RefreshRecipes)
            .map_err(|_| "companion worker is not running".to_string())
    }

    /// Start a run of a recipe, with its inputs and — for a formation — the person's leave for its
    /// agents. Returns once it is queued; the outcome lands in `crate::recipes` as a notice. What
    /// the Recipes screen's Start does, on the UI thread.
    pub fn start_recipe(
        &self,
        recipe: String,
        variables: serde_json::Map<String, serde_json::Value>,
        leave: Option<yantrik_companion::recipe::Leave>,
    ) -> Result<(), String> {
        self.cmd_tx
            .send(CompanionCommand::StartRecipe { recipe, variables, leave, reply_tx: None })
            .map_err(|_| "companion worker is not running".to_string())
    }

    /// The same, waiting up to `timeout` for the worker to say what it started. Never on the UI
    /// thread: the worker may be forty seconds into a generation. `shell.run_recipe` waits here,
    /// off the UI thread (`control::answer_later`).
    pub fn start_recipe_and_wait(
        &self,
        recipe: String,
        variables: serde_json::Map<String, serde_json::Value>,
        leave: Option<yantrik_companion::recipe::Leave>,
        timeout: std::time::Duration,
    ) -> Result<StartedRecipe, String> {
        let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
        self.cmd_tx
            .send(CompanionCommand::StartRecipe { recipe, variables, leave, reply_tx: Some(reply_tx) })
            .map_err(|_| "companion worker is not running".to_string())?;
        reply_rx.recv_timeout(timeout).map_err(|_| {
            format!(
                "the companion did not take the recipe within {} s — it may be in the middle of an answer; \
                 `describe shell` → `recipes` shows whether it started",
                timeout.as_secs()
            )
        })?
    }

    /// Answer, pause, resume or cancel one recipe. Returns once it is queued: the worker may be
    /// in the middle of a generation, and nothing on the UI thread waits for it. The outcome
    /// lands in `crate::recipes`.
    pub fn recipe(&self, recipe_id: String, op: yantrik_companion::recipe_view::RecipeOp) -> Result<(), String> {
        self.cmd_tx
            .send(CompanionCommand::Recipe { recipe_id, op })
            .map_err(|_| "companion worker is not running".to_string())
    }

    /// The board, for status and cancellation.
    pub fn board(&self) -> &crate::jobs::Board {
        &self.board
    }

    /// Whether the LLM backend answered on the last call.
    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }
}

impl CompanionBridge {
    /// Protect, open, re-wrap or read the credential vault.
    ///
    /// On `CompanionBridge` and deliberately not on [`CompanionHandle`]. The handle is the part
    /// that travels: `companion_rpc` holds one and answers other processes with it, so every
    /// method on it is reachable, eventually, by something on a socket. This one is reachable
    /// only from code holding the UI's own bridge — the login wiring and the callback behind the
    /// shell's unlock prompt — which is the whole claim this feature makes about who can supply a
    /// passphrase.
    ///
    /// Blocking, because every caller is a person who has just pressed Enter and is waiting to
    /// find out. The worker holds the only connection to the memory database.
    pub fn vault(
        &self,
        op: crate::vault_unlock::Op,
        timeout: std::time::Duration,
    ) -> Result<crate::vault_unlock::Reply, String> {
        let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
        self.cmd_tx
            .send(CompanionCommand::Vault { op, reply_tx })
            .map_err(|_| "companion worker is not running".to_string())?;
        reply_rx
            .recv_timeout(timeout)
            .map_err(|_| "the vault did not answer in time".to_string())
    }

    /// A handle that other threads can hold. See [`CompanionHandle`].
    pub fn handle(&self) -> CompanionHandle {
        CompanionHandle {
            cmd_tx: self.cmd_tx.clone(),
            online: self.online.clone(),
            board: self.board.clone(),
        }
    }

    /// Start the companion worker thread.
    pub fn start(
        config: CompanionConfig,
        ui_weak: slint::Weak<App>,
        event_bus: yantrik_os::EventBus,
        board: crate::jobs::Board,
    ) -> Self {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let online = Arc::new(AtomicBool::new(true));
        let online_w = online.clone();
        let cached_bond_level = Arc::new(std::sync::atomic::AtomicU8::new(1));
        let bond_w = cached_bond_level.clone();
        let ambient = AmbientState::new();
        let ambient_w = ambient.clone();
        let bus_w = event_bus.clone();

        let self_tx = cmd_tx.clone();
        let board_w = board.clone();
        let worker_handle = std::thread::spawn(move || {
            worker_loop(config, cmd_rx, self_tx, ui_weak, online_w, bond_w, ambient_w, bus_w, board_w);
        });

        Self {
            board,
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
        if self
            .cmd_tx
            .send(CompanionCommand::SendMessage {
                text,
                token_tx: token_tx.clone(),
                job: None,
                model: None,
            })
            .is_err()
        {
            tracing::error!("Companion worker thread is dead — cannot send message");
            let _ = token_tx.send("__REPLACE__".to_string());
            let _ = token_tx.send("Companion service crashed. Please restart the application.".to_string());
            let _ = token_tx.send("__DONE__".to_string());
        }
        token_rx
    }

    /// Count an answered turn that began with the person's words toward the bond.
    pub fn score_conversation_turn(&self, text: String) {
        let _ = self.cmd_tx.send(CompanionCommand::ScoreConversationTurn { text });
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

    /// Reload the LLM backend with a new provider config.
    /// Called when user adds/edits a provider in settings.
    pub fn reload_llm(&self, provider_type: String, base_url: String, api_key: Option<String>, model: String) {
        // Mark online optimistically so UI shows correct status while reload happens
        self.online.store(true, Ordering::Relaxed);
        let _ = self.cmd_tx.send(CompanionCommand::ReloadLLM {
            provider_type,
            base_url,
            api_key,
            model,
        });
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

/// A recipe run the worker started: the run's id and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedRecipe {
    pub run: String,
    pub name: String,
    /// It hands work to agents from the catalog.
    pub formation: bool,
}

/// Start a run on the worker's own connection: `recipe_templates::start`, and what to say of it.
fn start_recipe_run(
    conn: &rusqlite::Connection,
    recipe: &str,
    variables: &serde_json::Map<String, serde_json::Value>,
    leave: Option<&yantrik_companion::recipe::Leave>,
) -> Result<StartedRecipe, String> {
    // What the person agrees to: each role the run names, as the catalog defines it now. A role
    // whose definition changes before its step is not started on this leave.
    let leave = match leave {
        Some(leave) => Some(agreed_roles(conn, recipe, variables, leave)?),
        None => None,
    };
    let (template, run) = yantrik_companion::recipe_templates::start(conn, recipe, Some(variables), leave.as_ref())?;
    let steps: Vec<yantrik_companion::recipe::RecipeStep> =
        yantrik_companion::recipe::RecipeStore::get_steps(conn, &run).into_iter().map(|s| s.step).collect();
    Ok(StartedRecipe { run, name: template.name, formation: yantrik_companion::recipe::hands_off(&steps) })
}

/// `leave`, with the digest of each role the run will hand work to, as the catalog has it now. A
/// role the catalog does not have refuses the start, naming it.
fn agreed_roles(
    conn: &rusqlite::Connection,
    recipe: &str,
    variables: &serde_json::Map<String, serde_json::Value>,
    leave: &yantrik_companion::recipe::Leave,
) -> Result<yantrik_companion::recipe::Leave, String> {
    let roles = yantrik_companion::recipe_templates::roles_for(conn, recipe, Some(variables))?;
    let catalog = crate::agents::catalog::Catalog::load();
    let mut agreed = leave.clone();
    for named in roles {
        let role = catalog.find(&named).ok_or_else(|| {
            format!("'{recipe}' hands work to `{named}`, and the catalog has no such role; it has {}.", catalog.listing())
        })?;
        agreed.roles.insert(named, role.digest());
    }
    Ok(agreed)
}

/// Signal a recipe's next step — once. A recipe with a signal already queued is not signalled
/// again: each signal runs a step and sends the next, so a second chain would double the steps
/// queued ahead of a person's message, and the clock would start one every tick.
fn signal_recipe(cmd_tx: &Sender<CompanionCommand>, queued: &mut std::collections::HashSet<String>, recipe_id: String) {
    if queued.insert(recipe_id.clone()) {
        let _ = cmd_tx.send(CompanionCommand::ProcessRecipeStep { recipe_id });
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
    board: crate::jobs::Board,
) {
    // Save config services before moving config into build_companion
    let config_services = config.enabled_services.clone();

    // Build companion on this thread (owns SQLite connection)
    //
    // If it cannot be built, the thread does NOT exit. A dead worker makes every send() fail
    // with "companion worker is not running", which is true and useless: it says the postman
    // is missing, not that there is no address. Staying up to answer with the real reason is
    // the difference between an OS that tells you the embedder is absent and one that looks
    // broken in sixteen places at once.
    let mut companion = match build_companion(config) {
        Ok(c) => c,
        Err(why) => {
            tracing::error!(reason = %why, "Companion unavailable — answering every request with this");
            online.store(false, Ordering::Relaxed);
            // #30: say so on screen, not only in the log. This path has no companion to
            // push_state from, so it sets the status-bar notice itself.
            let weak = ui_weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_companion_online(false);
                    ui.set_assistant_offline_notice(
                        assistant_offline_notice(false).unwrap_or_default().into(),
                    );
                }
            });
            while let Ok(cmd) = cmd_rx.recv() {
                if let CompanionCommand::SendMessage { token_tx, model, .. } = cmd {
                    let _ = token_tx.send(format!("__REPLACE__{why}"));
                    // No companion at all is the no-model case, not an answer: a caller blocked
                    // in `ask` must not carry this text away as one.
                    if let Some(tx) = model {
                        let _ = tx.send(false);
                    }
                }
            }
            return;
        }
    };

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

    // A recipe's Agent steps hand work to the agent catalog through the shell's own hand_off
    // (design/desk-and-mind-2026-09-23.md, section 6). Installed before the recipes a restart
    // left running are resumed below, so a recipe waiting on an agent hears it, or hears that it
    // is gone.
    companion.set_agent_hook(Box::new(crate::control_agents::RecipeHands::default()));

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

    // "Suppressing EXECUTE urges — LLM offline" is logged once per outage, not once per
    // think cycle: a backend that was down all day filled the log with these (#30).
    // Reset whenever the LLM answers again.
    let mut execute_suppression_logged = false;

    // The "when did a person last say something" clock is not here any more. It lived in this
    // worker and was bumped only in the `SendMessage` arm below, which only messages bound for
    // the BUILT-IN companion ever reach — so with a harness mind answering, the Synthesis Gate
    // saw an idle user through a live conversation. It is one clock for every mind now, in
    // `wire::notifications`, bumped at the dispatch both entry points go through.

    // Push initial state to UI
    //
    // These three steps are traced because the worker once stopped somewhere between here and
    // the command loop, with no log to say where: every RPC call then timed out and the desktop
    // looked merely slow. A silent gap before a loop that must never stop is worth four lines.
    tracing::debug!("Worker startup: pushing initial state");
    push_state(&companion, &ui_weak, online.load(Ordering::Relaxed));

    // Sync initial bond level
    tracing::debug!("Worker startup: syncing bond level");
    cached_bond.store(companion.bond_level().as_u8(), Ordering::Relaxed);
    // And the bond itself. `describe shell` and the desktop's machine rail read the `bond_data`
    // property, and the only thing that ever wrote it was opening the Bond screen — so a shell
    // nobody had opened that screen on answered "Stranger, 0.0" over a store that said
    // Partner-in-Crime, 176 interactions. The worker owns the store; it keeps the property.
    push_bond(&companion, &ui_weak);



    // Recipes with a step signal queued — see `signal_recipe`.
    let mut recipe_signals: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Resume the recipes running before shutdown, and any whose wait ran out while it was down
    tracing::debug!("Worker startup: checking for resumable recipes");
    let due = yantrik_companion::recipe_executor::due(&companion.db.conn());
    for rid in due {
        tracing::info!(recipe_id = %rid, "Resuming recipe from previous session");
        signal_recipe(&cmd_tx, &mut recipe_signals, rid);
    }

    tracing::info!("Companion worker ready for commands");

    // Set by every command that can change a recipe; the recipes are published once it is done,
    // before the next command is taken. See `crate::recipes`.
    let mut recipes_dirty = true;

    // The recipes' clock. A timed wait was resumed only by the sweep after a chat message, so a
    // recipe waiting fifteen minutes on an idle desktop waited until somebody talked to the
    // companion (#176). Waiting for the next command is bounded by the clock instead: every few
    // seconds — or after a command, once that much time has gone — what is due is signalled.
    let recipe_tick = std::time::Duration::from_secs(yantrik_companion::recipe_executor::CLOCK_SECS);
    let mut recipe_clock = std::time::Instant::now();

    loop {
        // The recipes, read here because this thread owns the store's connection (a second one
        // from this process is what the engine refuses to write beside), and before the wait, so
        // what is shown is the state the last command left. The Recipes screen, `describe shell`
        // and the mind panel's recipes in flight all read this one copy (`crate::recipes`).
        if std::mem::take(&mut recipes_dirty) {
            crate::recipes::publish(yantrik_companion::recipe_view::list(&companion.db.conn()));
        }
        // The mind panel: the worker has reached its loop, so the memory count it pushes is a count.
        crate::mind_panel::worker_up();
        if recipe_clock.elapsed() >= recipe_tick {
            recipe_clock = std::time::Instant::now();
            let due = yantrik_companion::recipe_executor::due(&companion.db.conn());
            for rid in due {
                signal_recipe(&cmd_tx, &mut recipe_signals, rid);
            }
        }
        let received = cmd_rx.recv_timeout(recipe_tick.saturating_sub(recipe_clock.elapsed()));
        if matches!(received, Err(crossbeam_channel::RecvTimeoutError::Timeout)) {
            continue;
        }
        match received {
            Ok(CompanionCommand::RefreshRecipes) => recipes_dirty = true,
            Ok(CompanionCommand::StartRecipe { recipe, variables, leave, reply_tx }) => {
                let outcome = start_recipe_run(&companion.db.conn(), &recipe, &variables, leave.as_ref());
                match &outcome {
                    Ok(started) => {
                        tracing::info!(recipe = %recipe, run = %started.run, formation = started.formation, "Recipe started");
                        signal_recipe(&cmd_tx, &mut recipe_signals, started.run.clone());
                        crate::recipes::record(
                            &started.run,
                            Ok(if started.formation {
                                format!("Started '{}': its agents are at work — each has a row on the Agents screen.", started.name)
                            } else {
                                format!("Started '{}'.", started.name)
                            }),
                        );
                    }
                    Err(why) => {
                        tracing::info!(recipe = %recipe, why = %why, "Recipe: not started");
                        crate::recipes::record(&recipe, Err(why.clone()));
                    }
                }
                if let Some(tx) = reply_tx {
                    let _ = tx.send(outcome);
                }
                recipes_dirty = true;
            }
            Ok(CompanionCommand::Recipe { recipe_id, op }) => {
                let outcome = yantrik_companion::recipe_view::apply(&companion.db.conn(), &recipe_id, &op);
                match &outcome {
                    Ok(applied) => {
                        tracing::info!(recipe_id = %recipe_id, op = op.verb(), "Recipe: {}", applied.message);
                        // Running again — after an answer, or resumed: the executor takes it now,
                        // the way a chat turn's sweep would.
                        if applied.run_now {
                            signal_recipe(&cmd_tx, &mut recipe_signals, recipe_id.clone());
                        }
                    }
                    Err(why) => tracing::info!(recipe_id = %recipe_id, op = op.verb(), why = %why, "Recipe: refused"),
                }
                crate::recipes::record(&recipe_id, outcome.map(|a| a.message));
                recipes_dirty = true;
            }
            Ok(CompanionCommand::SendMessage { text, token_tx, job, model }) => {
                // A turn can create, run or change a recipe through its tools.
                recipes_dirty = true;
                // Work that arrived without a ticket gets one here, and that is not bookkeeping:
                // the startup brief and every message typed into the chat box come through this
                // arm, and without an entry the board reported "0 active" while the worker was
                // busy for forty seconds. A caller reading that would conclude the lane was free.
                // The board must account for everything the lane does, or it is worse than no
                // board at all.
                let job = job.or_else(|| {
                    let kind = if text.contains("You just started up") { "brief" } else { "ask" };
                    Some(board.submit("model", kind).ticket)
                });

                // The moment this arm runs is the moment the job is no longer waiting. Anything
                // earlier would be a guess; anything later would report a generation as queued
                // while it is already producing tokens.
                if let Some(id) = &job {
                    board.start(id);
                }
                // A system-generated prompt is not a person talking, and must not make the
                // Synthesis Gate think somebody is. `wire::chat::dispatch` is bumped by a
                // person typing; this arm also carries the startup brief, EXECUTE urges and
                // the companion's own reflection prompts, so it bumps nothing.
                let is_system_generated = text.contains("You just started up")
                    || text.contains("EXECUTE ")
                    || text.starts_with("Reflect naturally")
                    || text.starts_with("Recall shared references");
                tracing::trace!(is_system_generated, "companion SendMessage");

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
                                &companion.db.conn(), c,
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
                    let job_ref = job.as_ref();
                    let board_ref = &board;
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        companion.handle_message_streaming(&text, |token| {
                            if *token_count_ref == 0 {
                                tracing::info!(
                                    elapsed_ms = start_ref.elapsed().as_millis(),
                                    "First token generated"
                                );
                            }
                            *token_count_ref += 1;
                            // The board sees the same stream the chat does, so a caller watching
                            // a ticket can show the answer arriving rather than a spinner.
                            if let Some(id) = job_ref {
                                board_ref.progress(id, token, token == "__REPLACE__");
                            }
                            let _ = token_tx_ref.send(token.to_string());
                        })
                    }))
                };

                match result {
                    Ok(response) => {
                        // V22: Use response.offline_mode to track LLM status
                        let ok = !response.offline_mode;
                        online.store(ok, Ordering::Relaxed);
                        // Tell a caller blocked in `ask` whether a model answered this turn,
                        // while the fact is still the one being stored: it must not take the
                        // offline responder's canned text away as the model's words.
                        if let Some(tx) = &model {
                            let _ = tx.send(ok);
                        }
                        if ok {
                            execute_suppression_logged = false;
                        }
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
                        let _ = token_tx.send(TURN_FAILED_REPLY.to_string());
                    }
                }

                // Sentinel to indicate generation is done (sent in all cases)
                if let Some(id) = &job {
                    // Settled from what the board already holds: the partial text *is* the
                    // answer once the stream ends, so there is no second copy to keep in step.
                    let text = board
                        .wait(id, std::time::Duration::ZERO)
                        .and_then(|s| s["partial"].as_str().map(str::to_string))
                        .unwrap_or_default();
                    board.finish(id, Ok(text));
                }
                let _ = token_tx.send("__DONE__".to_string());

                // Push updated state (includes online status)
                push_state(&companion, &ui_weak, online.load(Ordering::Relaxed));
                // V15: Update cached bond level for UI features
                cached_bond.store(companion.bond_level().as_u8(), Ordering::Relaxed);
                push_bond(&companion, &ui_weak);

                // If tasks are pending (maybe user just queued one), signal the task processor
                if yantrik_companion::task_queue::TaskQueue::active_count(&companion.db.conn()) > 0 {
                    let _ = cmd_tx.send(CompanionCommand::ProcessNextTask);
                }
                // A turn can start a recipe (`run_recipe`): signal what is due now rather than at
                // the clock's next tick.
                let due = yantrik_companion::recipe_executor::due(&companion.db.conn());
                for rid in due {
                    signal_recipe(&cmd_tx, &mut recipe_signals, rid);
                }
            }
            Ok(CompanionCommand::GetBondState { reply_tx }) => {
                let _ = reply_tx.send(bond_snapshot(&companion));
            }
            Ok(CompanionCommand::GetEvolution { reply_tx }) => {
                let style = Evolution::get_style(&companion.db.conn());
                let opinions = Evolution::get_opinions(&companion.db.conn(), 20);
                let refs = Evolution::get_shared_references(&companion.db.conn(), 20);

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
            Ok(CompanionCommand::RunTool { name, args, reply_tx, job }) => {
                tracing::info!(tool = %name, "Running tool for an outside caller");
                // As above: a blocking caller's tool still occupies the lane, so it still belongs
                // on the board.
                let job = job.or_else(|| Some(board.submit("model", &format!("tool:{name}")).ticket));
                if let Some(id) = &job {
                    board.start(id);
                }
                let output = companion.run_tool(&name, &args);
                if let Some(id) = &job {
                    // A tool that reports a permission denial or a bad argument has still *run*;
                    // it is `Done` with that answer, not `Failed`. Failure here is reserved for
                    // the companion being unable to try at all, which is a different thing for a
                    // caller to react to.
                    board.finish(id, Ok(output.clone()));
                }
                let _ = reply_tx.send(output);
                // `run_recipe` from an outside caller: start it as a chat turn's sweep would.
                if name == "run_recipe" {
                    let due = yantrik_companion::recipe_executor::due(&companion.db.conn());
                    for rid in due {
                        signal_recipe(&cmd_tx, &mut recipe_signals, rid);
                    }
                }
                recipes_dirty = true;
            }
            Ok(CompanionCommand::ListTools { reply_tx }) => {
                let _ = reply_tx.send(companion.tool_catalog());
            }
            Ok(CompanionCommand::Vault { op, reply_tx }) => {
                // Not put on the job board, and not logged. A board ticket carries a label into
                // `describe shell` and a log line carries a timestamp into a file, and neither is
                // a thing anybody needs to know about the moment a person typed their password.
                // An Argon2id unwrap is a fraction of a second, which is why it can sit in the
                // worker's queue like anything else instead of blocking the UI thread.
                let _ = reply_tx.send(crate::vault_unlock::run(&companion.db.conn(), op));
            }
            Ok(CompanionCommand::GetBondLevel { reply_tx }) => {
                let _ = reply_tx.send(companion.bond_level());
            }
            Ok(CompanionCommand::GetPendingUrges { reply_tx }) => {
                let urges = companion.urge_queue.get_pending(&companion.db.conn(), 10);
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
            Ok(CompanionCommand::ScoreConversationTurn { text }) => {
                companion.score_conversation_turn(&text);
                cached_bond.store(companion.bond_level().as_u8(), Ordering::Relaxed);
                push_bond(&companion, &ui_weak);
            }
            Ok(CompanionCommand::ReloadLLM { provider_type, base_url, api_key, model }) => {
                tracing::info!(provider = %provider_type, model = %model, "Reloading LLM backend");
                let new_llm: std::sync::Arc<dyn yantrik_ml::LLMBackend> = std::sync::Arc::new(
                    yantrik_ml::ApiLLM::new(base_url.clone(), api_key.clone(), &model)
                );
                companion.swap_llm(new_llm);
                // Update config in memory so it persists for next restart
                companion.config.llm.backend = "api".into();
                companion.config.llm.api_base_url = Some(base_url);
                companion.config.llm.api_model = Some(model);
                companion.config.llm.api_key = api_key;
                // Save config to disk
                companion.save_config();
                online.store(true, Ordering::Relaxed);
                execute_suppression_logged = false;
                tracing::info!("LLM reloaded successfully");
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
                let event_data = serde_json::json!({
                    "text": safe_text,
                    "importance": safe_importance,
                });
                companion.push_event(&safe_domain, event_data.clone());

                // Recipes whose Event trigger names this event start here, where the event is
                // born — the stored triggers never fired (#187). The worker's clock picks the
                // started runs up on its next tick, like any recipe set running.
                let started = {
                    let at = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs_f64();
                    yantrik_companion::recipe::RecipeStore::fire_event_triggers(
                        &companion.db.conn(),
                        &safe_domain,
                        &event_data,
                        at,
                    )
                };
                for run in started {
                    tracing::info!(recipe = %run, event = %safe_domain, "Recipe event trigger fired");
                }
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
                    // A finished task is a result somebody is waiting on, so it is never held —
                    // but it is still only written into the transcript when the built-in
                    // companion is the mind the person is talking to. Otherwise it is a
                    // notification, which is where news belongs when the conversation is
                    // somebody else's. See `wire::notifications::route_result`.
                    crate::wire::notifications::deliver_result(notif, move |_lens_was_closed| {
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
                                // Through the notifications service, and only when the Lens is
                                // shut: with it open the answer is already in the conversation
                                // a few pixels away, and a toast over it says the same thing
                                // twice. It used to be a private toast either way, so a
                                // background task that finished while the person was elsewhere
                                // left no trace.
                                crate::wire::notifications::companion_said(
                                    &ui, "Task complete", &notif_text,
                                );
                            }
                        });
                    });
                    // Forward to Telegram
                    if companion.config.telegram.enabled && companion.config.telegram.forward_proactive {
                        let _ = yantrik_companion::telegram::send_message(
                            &companion.config.telegram, notif,
                        );
                    }
                }
                let task_summary = companion.active_tasks_summary();
                let sched_summary = yantrik_companion::scheduler::Scheduler::format_summary(&companion.db.conn());
                let mut full_ctx = context;
                if !task_summary.is_empty() {
                    full_ctx.push('\n');
                    full_ctx.push_str(&task_summary);
                }
                if !sched_summary.is_empty() {
                    full_ctx.push('\n');
                    full_ctx.push_str(&sched_summary);
                }
                let auto_summary = yantrik_companion::automation::AutomationStore::format_summary(&companion.db.conn());
                if !auto_summary.is_empty() {
                    full_ctx.push('\n');
                    full_ctx.push_str(&auto_summary);
                }
                // Task queue summary — shows pending/in-progress persistent tasks
                let tq_summary = yantrik_companion::task_queue::TaskQueue::format_active_summary(&companion.db.conn());
                if !tq_summary.is_empty() {
                    full_ctx.push('\n');
                    full_ctx.push_str(&tq_summary);
                }
                // Recipe summary — active recipes + failure learnings
                let recipe_summary = yantrik_companion::recipe::RecipeStore::format_summary(&companion.db.conn());
                if !recipe_summary.is_empty() {
                    full_ctx.push('\n');
                    full_ctx.push_str(&recipe_summary);
                }
                let recipe_learnings = yantrik_companion::recipe::RecipeStore::get_failure_learnings(&companion.db.conn(), 5);
                if !recipe_learnings.is_empty() {
                    full_ctx.push_str("\nRecipe learnings (past failures to avoid):\n");
                    for l in &recipe_learnings {
                        full_ctx.push_str("  - ");
                        full_ctx.push_str(l);
                        full_ctx.push('\n');
                    }
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
                    cortex.update_focus(&companion.db.conn(), &window_title, &process_name, idle_seconds);
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
                        .get_conflicts(Some("open"), None, None, None, None, 100)
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
                    let due_tasks = yantrik_companion::scheduler::Scheduler::get_due(&companion.db.conn());
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
                        yantrik_companion::scheduler::Scheduler::advance(&companion.db.conn(), &task.task_id);
                    }
                    if !due_tasks.is_empty() {
                        tracing::info!(count = due_tasks.len(), "Scheduler: advanced due tasks");
                    }

                    // V24: Open Loops Monitor — scan commitments + attention items
                    {
                        let monitor_config = yantrik_companion::open_loops_monitor::MonitorConfig::default();
                        let scan_result = yantrik_companion::open_loops_monitor::scan(
                            &companion.db.conn(),
                            &monitor_config,
                        );

                        // Inject commitment triggers for instinct evaluation
                        let overdue = yantrik_companion::world_model::WorldModel::overdue_commitments(&companion.db.conn());
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

                        let approaching = yantrik_companion::world_model::WorldModel::approaching_deadlines(&companion.db.conn(), 24.0);
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
                    &companion.db.conn(),
                    &yantrik_companion::trust_model::TrustEvent::DailyInteraction,
                );
                yantrik_companion::trust_model::TrustModel::apply_daily_decay(&companion.db.conn());

                let state = companion.build_state();
                let mut urge_specs = companion.evaluate_instincts(&state);
                let now_ts = state.current_ts;

                // Context Cortex think step — run rules + baselines + patterns
                if let Some(ref mut cortex) = companion.cortex {
                    let attention_items = cortex.think(&companion.db.conn());
                    // Global cortex cooldown: suppress ALL cortex urges within 1 hour
                    // of the last cortex message. Prevents similar-but-different patterns
                    // from spamming the user every think cycle.
                    let cortex_globally_cooled = now_ts - last_cortex_fire_ts < CORTEX_GLOBAL_COOLDOWN_SECS;

                    if !attention_items.is_empty() && !cortex_globally_cooled {
                        let focus = cortex.current_focus();
                        let briefing = cortex.build_briefing(&companion.db.conn(), focus.as_ref(), &attention_items);
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
                    if let Some(reflection_prompt) = cortex.maybe_deep_reflection(&companion.db.conn()) {
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
                    // PlaybookState borrows the connection, so evaluate inside a
                    // scope that drops the guard before the action loop below
                    // re-enters `&mut companion`.
                    let pb_actions = {
                        let conn = companion.db.conn();
                        let pb_state = yantrik_companion::cortex::playbook::PlaybookState {
                            attention_items: &pb_attention,
                            current_focus: cortex_focus.as_ref(),
                            now_ts: pb_now,
                            user_hour: (pb_now as i64 % 86400 / 3600) as u32,
                            conn: &conn,
                            bond_level: companion.bond_level() as u8,
                        };
                        let user_receptivity = companion.user_model.engagement();
                        companion.playbook_engine.evaluate(&pb_state, user_receptivity)
                    };

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
                                    &companion.db.conn(),
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
                        companion.playbook_engine.save(&companion.db.conn());
                    }
                }

                // An unset bond clock (#156) means the person was never here —
                // the log says so instead of reporting the decades since 1970.
                let idle_hrs = match state.absence_seconds() {
                    Some(secs) => format!("{:.1}", secs / 3600.0),
                    None => "never".to_string(),
                };

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
                    idle_hours = %idle_hrs,
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
                        companion.urge_queue.push(&companion.db.conn(), spec);
                    }
                }

                // Process EXECUTE urges through tier-based log-linear softmax selector.
                let mut execute_produced_message = false;

                // v4: Suppress EXECUTE urges when LLM is offline.
                if !execute_urges.is_empty() && !online.load(Ordering::Relaxed) {
                    // Once per outage (#30) — the status bar carries the visible side.
                    if !execute_suppression_logged {
                        tracing::info!(
                            count = execute_urges.len(),
                            "Suppressing EXECUTE urges — LLM offline"
                        );
                        execute_suppression_logged = true;
                    }
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
                    if yantrik_companion::task_queue::TaskQueue::active_count(&companion.db.conn()) > 0 {
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

                    // Synthesis Gate: check similarity, budget, conversation state.
                    //
                    // This clock used to be bumped in this worker's own `SendMessage` arm,
                    // which only messages bound for the BUILT-IN companion ever reach. With a
                    // harness answering, a live conversation looked idle from here and the gate
                    // let messages through mid-answer — the fault that put "you once said: User
                    // is interested in: technology" into the middle of somebody asking Hermes
                    // for this machine's IP address. `wire::notifications` keeps the same clock
                    // for every mind, bumped at the one dispatch both entry points go through.
                    let user_idle_secs = crate::wire::notifications::seconds_since_user_message()
                        .unwrap_or(f64::MAX);
                    let conversation_active = crate::wire::notifications::conversation_active();
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

                    // Machinery talking, a raw tool call, or idle thinking that found nothing —
                    // not the companion. Checked before every other gate because the others all
                    // ask WHEN this should be said, and this one says it must not be said at all.
                    // An EXECUTE urge reaches here without passing through `ProactiveEngine::check`,
                    // which is why the same rule is applied in both places — and that is the path
                    // notification 68 took. Small talk is not refused here: it may still reach the
                    // Lens, and `deliver_proactive` keeps it out of the notification store.
                    if let Some(why) = yantrik_companion::proactive::must_not_be_said(&msg.text) {
                        tracing::warn!(
                            reason = why,
                            text = msg.text,
                            urges = ?msg.urge_ids,
                            "Refused a proactive message: it is not something to say"
                        );
                        companion.record_suppressed_urge(&delivery_key, why);
                        event_bus.emit(
                            yantrik_os::EventKind::ProactiveSuppressed {
                                reason: why.into(),
                                urge_ids: msg.urge_ids.clone(),
                            },
                            yantrik_os::EventSource::ProactiveEngine,
                        );
                    } else if on_cooldown {
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
                    } else if matches!(
                        crate::wire::notifications::route_proactive(
                            crate::wire::notifications::situation_now()
                        ),
                        crate::wire::notifications::ProactiveDelivery::Hold
                    ) {
                        // A mind is mid-answer. Checked here, before anything is recorded as
                        // sent: an urge that was held has not been said, and marking it sent
                        // would make the anti-repetition tracker suppress it the next time it
                        // is genuinely due.
                        tracing::info!(
                            text = msg.text,
                            "Holding proactive message — a mind is answering right now"
                        );
                        companion.record_suppressed_urge(&delivery_key, "a mind is mid-answer");
                        event_bus.emit(
                            yantrik_os::EventKind::ProactiveSuppressed {
                                reason: "a mind is mid-answer".into(),
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
                        // Where this goes depends on who is answering. The transcript belongs
                        // to the mind the person is talking to; when that is not this one, an
                        // unprompted line in it reads as the answer to whatever they just
                        // asked. See the section in `wire::notifications` — that is a fault
                        // somebody watched happen.
                        let route = crate::wire::notifications::deliver_proactive(
                            &msg.text,
                            move |_lens_was_closed| {
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
                                        // Raised only when the Lens is closed — re-checked here on
                                        // the UI thread, which is the one place that reading is
                                        // exact — and only when the thought is actionable and
                                        // today's cap is not reached. `companion_thought` is the
                                        // gated sibling of `companion_said`; the message is already
                                        // in the transcript above either way, so small talk still
                                        // reaches the Lens (issue #216).
                                        crate::wire::notifications::companion_thought(
                                            &ui,
                                            &notif_text,
                                        );
                                    }
                                });
                            },
                        );

                        // Forward proactive messages to Telegram
                        if companion.config.telegram.enabled && companion.config.telegram.forward_proactive {
                            if let Err(e) = yantrik_companion::telegram::send_message(
                                &companion.config.telegram, &msg.text,
                            ) {
                                tracing::warn!(error = %e, "Failed to forward proactive to Telegram");
                            }
                        }

                        // Emit proactive delivered event.
                        //
                        // The channel is read off the route rather than hardcoded to "chat":
                        // it was "chat" even when the message never reached a chat, which made
                        // the event log agree with the bug rather than describe it.
                        event_bus.emit(
                            yantrik_os::EventKind::ProactiveDelivered {
                                urge_ids: msg.urge_ids.clone(),
                                text_preview: msg.text.chars().take(100).collect(),
                                delivery_channel: match route {
                                    crate::wire::notifications::ProactiveDelivery::NotifyOnly => {
                                        "notification".into()
                                    }
                                    _ => "chat".into(),
                                },
                            },
                            yantrik_os::EventSource::ProactiveEngine,
                        );

                        // Record per-key cooldown after delivery. Recipe messages key on their
                        // own delivery (#187), so keys pile up that will never be looked up
                        // again; one whose cooldown has long expired makes way for the new one,
                        // which keeps this map from growing without limit.
                        if !delivery_key.is_empty() {
                            delivered_cooldowns.retain(|_, ts| now_ts - *ts < 2.0 * DELIVERED_COOLDOWN_SECS);
                            delivered_cooldowns.insert(delivery_key, now_ts);
                        }
                    }
                } else {
                    let pending = companion.urge_queue.count_pending(&companion.db.conn());
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
                companion.user_model.save(&companion.db.conn());

                // Push updated urges to Home screen
                push_urges(&companion, &ui_weak);

                push_state(&companion, &ui_weak, online.load(Ordering::Relaxed));
            }
            Ok(CompanionCommand::ProcessNextTask) => {
                recipes_dirty = true;
                // Event-driven task processing: process one step, then self-signal
                // to continue. User messages have priority — they arrive via the same
                // channel and will be processed before queued ProcessNextTask signals.

                // Check budget first
                let now_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64();
                let over_budget = companion.is_over_daily_budget();

                // Fetch in a scoped block so the connection guard is released
                // before the body: since v0.10 `conn()` returns a MutexGuard,
                // and an `if let` scrutinee holds its temporary for the whole
                // body — which calls back into `&mut companion` and touches the
                // database again.
                let next_task = if over_budget {
                    None
                } else {
                    let conn = companion.db.conn();
                    yantrik_companion::task_queue::TaskQueue::next_task(&conn)
                };

                if over_budget {
                    tracing::debug!("Task processing skipped — over daily budget");
                } else if let Some(task) = next_task {
                    // Guard: auto-fail tasks stuck after too many steps
                    const MAX_TASK_STEPS: i32 = 10;
                    if task.steps_completed >= MAX_TASK_STEPS {
                        tracing::warn!(
                            task_id = %task.task_id,
                            title = %task.title,
                            steps = task.steps_completed,
                            "Task exceeded max steps ({}) — auto-failing",
                            MAX_TASK_STEPS
                        );
                        yantrik_companion::task_queue::TaskQueue::fail(
                            &companion.db.conn(),
                            &task.task_id,
                            &format!("Auto-failed after {} steps without completion", MAX_TASK_STEPS),
                        );
                        // Check for more tasks
                        if yantrik_companion::task_queue::TaskQueue::active_count(&companion.db.conn()) > 0 {
                            let _ = cmd_tx.send(CompanionCommand::ProcessNextTask);
                        }
                        continue;
                    }

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

                    // Increment step counter and mark as in_progress
                    let new_steps = task.steps_completed + 1;
                    yantrik_companion::task_queue::TaskQueue::update_progress(
                        &companion.db.conn(), &task.task_id,
                        &if task.progress.is_empty() { "Starting...".to_string() } else { task.progress.clone() },
                        new_steps,
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
                    if !was_completed || yantrik_companion::task_queue::TaskQueue::active_count(&companion.db.conn()) > 0 {
                        let _ = cmd_tx.send(CompanionCommand::ProcessNextTask);
                    }
                } else {
                    tracing::debug!("No tasks in queue");
                }
            }
            Ok(CompanionCommand::ProcessRecipeStep { recipe_id }) => {
                recipes_dirty = true;
                recipe_signals.remove(&recipe_id);
                // One step of the companion's executor — the only one there is: the step the
                // recipe stands at, or the next inside the Branch it stands in. This arm used to
                // be an executor of its own, which passed ThinkCited, Validate, Render and the
                // data steps through and marked a Branch taken without running either side (#176).
                // It says whether there is more to run now; a wait or a question ends the chain,
                // and the clock or the answer starts it again. The signal goes to the back of the
                // queue, so a person's message is taken between any two steps.
                if yantrik_companion::recipe_executor::step(&mut companion, &recipe_id)
                    == yantrik_companion::recipe_executor::Advance::Next
                {
                    signal_recipe(&cmd_tx, &mut recipe_signals, recipe_id);
                }
            }
            Ok(CompanionCommand::GetMorningBrief { reply_tx }) => {
                // Build structured brief from active day context sections
                let user_name = &companion.config.user_name;
                // The clock decides, not the name of the feature.
                //
                // This was hardcoded to "Good morning", so a machine booted at 20:29 greeted
                // its user with a card saying good morning directly beside a hero saying good
                // evening. Both were on screen at once; one of them was wrong every day after
                // lunch. The hero already had the answer -- there is one function for this and
                // this call site was not using it.
                let greeting = format!(
                    "{}, {}!",
                    crate::app_context::time_of_day_greeting(),
                    user_name
                );
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

/// What the status bar says when the assistant has no model to answer with (#30).
///
/// A desktop whose backend was down all day wrote 450+ log lines and showed nothing
/// on screen; the failure belongs on the screen. Pure state: online is `None` (the
/// chip hides itself), offline is one persistent line naming the way out — Settings → AI.
///
/// It says what is known and no more: the built-in assistant's model did not answer. On the
/// machine that reported this a model WAS set up (Ollama on the host) and simply was not
/// running, so "set one up" would have sent the person to fix the wrong thing; and an attached
/// mind such as Hermes may be answering the chat meanwhile, so the line names which assistant
/// it is about.
pub fn assistant_offline_notice(companion_online: bool) -> Option<&'static str> {
    if companion_online {
        None
    } else {
        Some("The built-in assistant's model is not answering. Check Settings → AI")
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
            ui.set_assistant_offline_notice(
                assistant_offline_notice(companion_online).unwrap_or_default().into(),
            );
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

/// The bond as the store has it right now.
fn bond_snapshot(companion: &CompanionService) -> BondSnapshot {
    let bond = BondTracker::get_state(&companion.db.conn());
    let humor_rate = if bond.humor_attempts > 0 {
        bond.humor_successes as f64 / bond.humor_attempts as f64
    } else {
        0.0
    };
    BondSnapshot {
        bond_score: bond.bond_score,
        bond_level: bond.bond_level.name().to_string(),
        total_interactions: bond.total_interactions,
        days_together: bond.days_together as i64,
        current_streak: bond.current_streak_days,
        humor_rate,
        vulnerability_events: bond.vulnerability_events,
        shared_references: bond.shared_references,
    }
}

/// Push the bond to the Slint UI thread.
///
/// Called whenever the store moves and once at startup, so `bond_data` says what the store
/// says rather than what the Bond screen last fetched. Everything that shows the bond without
/// opening that screen — `describe shell`, the machine rail — reads this property.
fn push_bond(companion: &CompanionService, ui_weak: &slint::Weak<App>) {
    let bond = bond_snapshot(companion);
    let weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_bond_data(BondData {
                // The one place this becomes true: the store has been read, on the thread that
                // owns it. Until then the property is the Slint default, and the rail and
                // `describe shell` say so instead of showing that default as a level.
                loaded: true,
                bond_score: bond.bond_score as f32,
                bond_level: bond.bond_level.into(),
                total_interactions: bond.total_interactions as i32,
                days_together: bond.days_together as i32,
                current_streak: bond.current_streak as i32,
                humor_rate: bond.humor_rate as f32,
                vulnerability_events: bond.vulnerability_events as i32,
                shared_references: bond.shared_references as i32,
            });
        }
    });
}

/// Push pending urges to the Slint UI thread.
fn push_urges(companion: &CompanionService, ui_weak: &slint::Weak<App>) {
    let urges = companion.urge_queue.get_pending(&companion.db.conn(), 10);
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

/// The embedder every machine gets when it has not been given one of its own.
///
/// MiniLM-L6-v2, fetched once and cached by the hub client. Semantic memory is the OS's own
/// faculty rather than a harness's -- a harness brings its own LLM, which is a different
/// thing -- so this is the one model the OS will go and get for itself.
fn default_embedder() -> Result<CandleEmbedder, String> {
    tracing::info!("Loading the default MiniLM embedder");
    CandleEmbedder::from_hub("sentence-transformers/all-MiniLM-L6-v2", None)
        .map_err(|e| format!("the default embedder could not be fetched: {e}"))
}

/// Build a CompanionService (same logic as crates/yantrik/src/main.rs).
///
/// Returns `Err` instead of panicking, because the two `.expect()`s this used to have were
/// killing the companion worker thread at boot on any machine without the embedder files --
/// and taking every agentic feature in the OS down with it, silently.
///
/// What that looked like: `companion.sock` listening and accepting connections, the desktop
/// reporting `companion_online: true`, and every AI action in every app answering
/// "companion worker is not running". Fifty-five buttons, one root cause, and nothing on
/// screen said which. The log had it, once, at boot:
///
///     panicked at bridge.rs: failed to load embedder from directory:
///       config.json not found in /opt/yantrik/models/embedder
///     ERROR Companion worker thread is dead — cannot send message
///
/// The OS does not ship models -- a harness brings its own -- so "the embedder is not here"
/// is a normal state on a fresh machine, not a fault. It has to degrade and say so.
fn build_companion(config: CompanionConfig) -> Result<CompanionService, String> {
    // Load embedder
    // Identity travels with the embedder so yantrikdb can distinguish a
    // same-model reattach from a different-model-same-dim swap, which would
    // silently corrupt an already-indexed database.
    let embedder_identity: Option<String> = config
        .yantrikdb
        .embedder_model_dir
        .as_ref()
        .map(|d| format!("candle:dir:{d}"));
    let embedder = if let Some(ref dir) = config.yantrikdb.embedder_model_dir {
        tracing::info!(dir, "Loading embedder from directory");
        // A configured directory is a PREFERENCE, not a requirement.
        //
        // The default path /opt/yantrik/models/embedder is written into the config on every
        // machine, and the release does not ship models -- so on a fresh install this branch
        // was always taken, always failed, and took the whole companion down with it. There
        // was a perfectly good default two lines below that could never be reached, because
        // having a directory CONFIGURED is not the same as having one PRESENT.
        match CandleEmbedder::from_dir(std::path::Path::new(dir)) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(dir, error = %e, "No embedder there — falling back to the default");
                default_embedder()?
            }
        }
    } else {
        default_embedder()?
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
            .ok_or_else(|| "api_base_url required for API backend (set it or use a named provider like 'ollama')".to_string())?;
        let model = config.llm.api_model.as_deref()
            .ok_or_else(|| "api_model required for API backend".to_string())?;
        tracing::info!(
            backend = config.llm.backend,
            base_url = %base_url,
            model,
            "Using API LLM backend"
        );
        std::sync::Arc::new(ApiLLM::new(base_url, config.llm.api_key.clone(), model))
    } else if config.llm.backend == "llamacpp" {
        let gguf = config.llm.gguf_path.as_deref()
            .ok_or_else(|| "gguf_path required for llamacpp backend".to_string())?;
        let gpu_layers = config.llm.fallback.as_ref()
            .map(|f| f.n_gpu_layers).unwrap_or(99);
        let ctx_size = config.llm.max_context_tokens as u32;
        tracing::info!(
            gguf, gpu_layers, ctx_size,
            raw_max_ctx = config.llm.max_context_tokens,
            "Using llama.cpp backend"
        );
        #[cfg(feature = "llamacpp")]
        { std::sync::Arc::new(LlamaCppLLM::from_gguf(
            std::path::Path::new(gguf), gpu_layers, ctx_size,
        ).map_err(|e| format!("failed to load llama.cpp model: {e}"))?) }
        #[cfg(not(feature = "llamacpp"))]
        { panic!("llamacpp feature not enabled at compile time") }
    } else if let Some(ref dir) = config.llm.model_dir {
        tracing::info!(dir, "Loading Candle LLM from directory");
        std::sync::Arc::new(CandleLLM::from_dir(std::path::Path::new(dir))
            .map_err(|e| format!("failed to load LLM from directory: {e}"))?)
    } else if let (Some(ref gguf), Some(ref tok)) =
        (&config.llm.gguf_path, &config.llm.tokenizer_path)
    {
        tracing::info!(gguf, tok, "Loading Candle LLM from explicit paths");
        std::sync::Arc::new(CandleLLM::from_gguf(std::path::Path::new(gguf), std::path::Path::new(tok))
            .map_err(|e| format!("failed to load LLM: {e}"))?)
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
        .map_err(|e| format!("failed to download LLM: {e}"))?;
        std::sync::Arc::new(CandleLLM::from_gguf(&files.gguf, &files.tokenizer).map_err(|e| format!("failed to load LLM: {e}"))?)
    };

    // Wrap with fallback if configured
    let llm: std::sync::Arc<dyn LLMBackend> = if let Some(ref fb_config) = config.llm.fallback {
        let fallback_cfg = match fb_config.backend.as_str() {
            "llamacpp" => {
                let path = fb_config.model_path.as_deref()
                    .ok_or_else(|| "model_path required for llamacpp fallback".to_string())?;
                tracing::info!(model = path, gpu_layers = fb_config.n_gpu_layers, "Fallback: llama.cpp");
                FallbackConfig::LlamaCpp {
                    model_path: std::path::PathBuf::from(path),
                    n_gpu_layers: fb_config.n_gpu_layers,
                    context_size: fb_config.context_size,
                }
            }
            _ => {
                let url = fb_config.api_base_url.as_deref()
                    .ok_or_else(|| "api_base_url required for API fallback".to_string())?;
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
    //
    // Its directory is made first. /opt/yantrik/data does not exist on a fresh install and
    // nothing was creating it, so the open failed with "unable to open database file" and the
    // worker died — which is the same single point of failure as the embedder, one line down.
    if let Some(parent) = std::path::Path::new(&config.yantrikdb.db_path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let mut db =
        yantrikdb_core::YantrikDB::new(&config.yantrikdb.db_path, config.yantrikdb.embedding_dim)
            .map_err(|e| format!("failed to create YantrikDB: {e}"))?;
    db.set_embedder(match embedder_identity {
        Some(id) => Box::new(yantrik_companion::embedder_bridge::EmbedderBridge::with_identity(embedder, id)),
        None => Box::new(yantrik_companion::embedder_bridge::EmbedderBridge::new(embedder)),
    });

    tracing::info!(
        db_path = config.yantrikdb.db_path,
        user = config.user_name,
        "Companion initialized"
    );

    Ok(CompanionService::new(db, llm, config))
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

#[cfg(test)]
mod bond_property_tests {
    use std::path::Path;

    /// This file above the tests, as written.
    ///
    /// The worker needs a companion, a model and a Slint event loop to run, so what is pinned
    /// here is the property that matters and that no type could check: that the bond reaches
    /// the UI property from the thread that owns the store, and not only when a screen asks.
    fn worker() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge.rs");
        let whole = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        whole.split("#[cfg(test)]").next().unwrap_or_default().to_string()
    }

    fn between<'a>(src: &'a str, from: &str, to: &str) -> &'a str {
        let start = src.find(from).unwrap_or_else(|| panic!("`{from}` is no longer in bridge.rs"));
        let rest = &src[start..];
        &rest[..rest.find(to).unwrap_or(rest.len())]
    }

    /// One arm of the worker's command loop: from its pattern to the next arm's.
    fn arm<'a>(src: &'a str, variant: &str) -> &'a str {
        let head = format!("Ok(CompanionCommand::{variant}");
        let start = src.find(&head).unwrap_or_else(|| panic!("the worker no longer handles `{variant}`"));
        let rest = &src[start + head.len()..];
        &rest[..rest.find("Ok(CompanionCommand::").unwrap_or(rest.len())]
    }

    /// `describe shell` and the machine rail read `bond_data`. It used to be written by one
    /// thing — opening the Bond screen — so a shell on which nobody had reported "Stranger, 0.0"
    /// for forty minutes over a store that said Partner-in-Crime, 176 interactions.
    #[test]
    fn the_worker_keeps_the_bond_property_current() {
        let src = worker();
        let startup = between(&src, "Worker startup: syncing bond level", "Companion worker ready for commands");
        assert!(
            startup.contains("push_bond("),
            "the worker must push the bond once the store is open, or every describe before \
             somebody opens the Bond screen answers with the Slint default. Startup as written:\n{startup}"
        );
        let builtin_turn = arm(&src, "SendMessage");
        assert!(
            builtin_turn.contains("push_bond("),
            "a turn the built-in answered moves the store; the property has to follow"
        );
        let harness_turn = arm(&src, "ScoreConversationTurn");
        assert!(
            harness_turn.contains("score_conversation_turn(&text)") && harness_turn.contains("push_bond("),
            "a turn a harness answered is scored on this thread and the property follows. Arm as written:\n{harness_turn}"
        );
    }

    /// A formation's Agent steps reach the catalog through the shell's hook, installed before the
    /// worker resumes the recipes a restart left running — so one waiting on an agent hears its
    /// answer, or that it is gone, rather than failing for want of a hook. And Start goes through
    /// `recipe_templates::start`, the one door that gives a run its leave for agents.
    #[test]
    fn the_worker_hands_agent_steps_to_the_shell_before_it_resumes_recipes() {
        let src = worker();
        let hook = src.find("companion.set_agent_hook(Box::new(crate::control_agents::RecipeHands::default()))").expect("the hook is installed");
        let resume = src.find("Resume the recipes running before shutdown").expect("the resume");
        assert!(hook < resume, "installed before the recipes resume");
        let start = arm(&src, "StartRecipe");
        assert!(start.contains("start_recipe_run(&companion.db.conn()"), "{start}");
        let run = between(&src, "fn start_recipe_run(", "fn agreed_roles(");
        assert!(run.find("agreed_roles(").unwrap() < run.find("recipe_templates::start(").unwrap(), "the roles' definitions are recorded with the leave it starts on");
        assert!(src.contains("yantrik_companion::recipe_templates::start(conn, recipe, Some(variables), leave.as_ref())"));
    }

    /// The shell runs the companion's one recipe executor, and the recipes keep time on the
    /// worker's own clock (#176). The worker had an executor of its own in this arm — the
    /// companion's fuller one never ran — and it marked a Branch taken without running either
    /// side; and a timed wait was resumed only by the sweep after a chat message, so a recipe
    /// waiting fifteen minutes waited until somebody talked to the companion.
    #[test]
    fn the_worker_runs_the_companion_s_executor_on_its_own_clock() {
        let src = worker();
        let step = arm(&src, "ProcessRecipeStep");
        assert!(
            step.contains("recipe_executor::step("),
            "a recipe's step is the companion's executor's to run. Arm as written:\n{step}"
        );
        assert!(
            !step.contains("RecipeStep::"),
            "the worker runs no step kind of its own: a second executor is how Branch came to do nothing. Arm as written:\n{step}"
        );
        let waiting = between(&src, "Companion worker ready for commands", "Ok(CompanionCommand::RefreshRecipes)");
        assert!(
            waiting.contains("recv_timeout(") && waiting.contains("recipe_executor::due("),
            "the worker's wait for its next command is also the recipes' clock, so a timer fires on an idle desktop. Loop head as written:\n{waiting}"
        );
    }

    /// An event the shell records also reaches the recipe triggers waiting on it (#187). The
    /// worker's clock picks up cron and completion triggers, but an event only exists at the
    /// moment it is pushed, so this arm is the one place an Event trigger can fire.
    #[test]
    fn an_event_the_shell_records_reaches_the_recipe_triggers_waiting_on_it() {
        let src = worker();
        let record = arm(&src, "RecordSystemEvent");
        assert!(
            record.contains("push_event("),
            "the event still reaches the companion. Arm as written:\n{record}"
        );
        assert!(
            record.contains("fire_event_triggers("),
            "a recorded event must also fire the recipe triggers naming it, or Event triggers \
             are stored and never happen. Arm as written:\n{record}"
        );
    }
}

#[cfg(test)]
mod ask_tests {
    use super::*;

    /// A real `CompanionHandle` around a stub worker: it serves one message and stops, doing
    /// what the real worker does at the end of a turn — text on the token channel, the model
    /// signal, then the sentinel.
    ///
    /// The real worker needs a companion, a model and a Slint event loop, so what is pinned
    /// here is the contract `ask` has with it, which is the part that changed.
    fn stub(
        served: impl Fn(&Sender<String>, &Option<Sender<bool>>) + Send + 'static,
    ) -> CompanionHandle {
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded::<CompanionCommand>();
        std::thread::spawn(move || {
            while let Ok(cmd) = cmd_rx.recv() {
                if let CompanionCommand::SendMessage { token_tx, model, .. } = cmd {
                    served(&token_tx, &model);
                }
            }
        });
        CompanionHandle {
            cmd_tx,
            online: Arc::new(AtomicBool::new(true)),
            board: crate::jobs::Board::new(),
        }
    }

    fn done(token_tx: &Sender<String>, model: &Option<Sender<bool>>, answered: bool) {
        if let Some(tx) = model {
            let _ = tx.send(answered);
        }
        let _ = token_tx.send("__DONE__".to_string());
    }

    /// The defect this pins: a turn the offline responder served used to come back as `Ok`
    /// holding its canned text, and apps showed that text as the model's answer — Documents
    /// offered to replace the document with it.
    #[test]
    fn a_turn_the_offline_responder_served_is_an_error_not_an_answer() {
        let handle = stub(|token_tx, model| {
            let _ = token_tx.send("Here is a canned reply that reads plausibly.".to_string());
            done(token_tx, model, false);
        });
        match handle.ask("anything".to_string(), std::time::Duration::from_secs(10)) {
            Err(AskError::NoModel) => {}
            other => panic!("a fallback turn must not come back as an answer; got {other:?}"),
        }
    }

    #[test]
    fn a_turn_a_model_served_still_comes_back_as_its_text() {
        let handle = stub(|token_tx, model| {
            let _ = token_tx.send("The model's ".to_string());
            let _ = token_tx.send("answer.".to_string());
            done(token_tx, model, true);
        });
        let got = handle
            .ask("anything".to_string(), std::time::Duration::from_secs(10))
            .expect("a model's answer is an answer");
        assert_eq!(got, "The model's answer.");
    }

    /// A turn that panicked mid-stream never reaches the point where the worker sends the
    /// model signal, and its text is `TURN_FAILED_REPLY` — a report about the turn, not the
    /// model's words. `Ok` would hand apps that sentence to show, or insert, as an answer:
    /// the canned-text defect wearing a different hat.
    #[test]
    fn a_turn_that_panicked_is_an_error_carrying_the_failure_text() {
        let handle = stub(|token_tx, _model| {
            let _ = token_tx.send("__REPLACE__".to_string());
            let _ = token_tx.send(TURN_FAILED_REPLY.to_string());
            let _ = token_tx.send("__DONE__".to_string());
        });
        match handle.ask("anything".to_string(), std::time::Duration::from_secs(10)) {
            Err(AskError::Failed(reason)) => assert_eq!(reason, TURN_FAILED_REPLY),
            other => panic!("a turn with no model signal must not come back as an answer; got {other:?}"),
        }
    }
}

#[cfg(test)]
mod assistant_offline_notice_tests {
    use std::path::Path;

    /// This file above the tests, as written (the same slice `bond_property_tests` reads).
    fn worker() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bridge.rs");
        let whole = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        whole.split("#[cfg(test)]").next().unwrap_or_default().to_string()
    }

    #[test]
    fn the_shell_says_when_there_is_no_model_and_hushes_when_it_answers() {
        // #30: the built-in companion had no model all day and the desktop never said
        // so — 450+ log lines and nothing on screen. The status item is pure state:
        // visible whenever the backend is offline, cleared the moment a model answers.
        let notice = super::assistant_offline_notice(false);
        assert!(notice.is_some(), "a backend that does not answer is visible on the shell");
        let text = notice.unwrap();
        assert!(text.contains("not answering"), "the line says what is wrong: {text}");
        assert!(
            text.contains("built-in"),
            "and which assistant, since an attached mind may be answering the chat: {text}"
        );
        assert!(text.contains("Settings → AI"), "and says where to fix it: {text}");
        assert_eq!(
            super::assistant_offline_notice(true),
            None,
            "the notice clears itself when the model answers again"
        );
    }

    #[test]
    fn the_notice_reaches_the_status_bar_from_every_offline_path() {
        let src = worker();
        let push = &src[src.find("fn push_state(").expect("push_state is in bridge.rs")..];
        assert!(
            push.contains("set_assistant_offline_notice("),
            "the per-cycle state push carries the notice, so it appears and clears with the backend"
        );
        let from = "Companion unavailable — answering every request with this";
        let start = src.find(from).expect("the build-failure path is in bridge.rs");
        let build_fail = &src[start..start + 1200];
        assert!(
            build_fail.contains("set_assistant_offline_notice("),
            "a worker whose companion could not even be built has no push_state — it says so itself. \
             Path as written:\n{build_fail}"
        );
        assert!(
            src.contains("if !execute_suppression_logged"),
            "the EXECUTE suppression line is gated: once per outage, not once per think cycle (#30)"
        );
    }
}
