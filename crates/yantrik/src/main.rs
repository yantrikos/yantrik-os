//! Yantrik — single-binary personal AI companion.
//!
//! Replaces 3 separate server processes:
//! - llama-server (LLM inference) → candle in-process
//! - llama-embed (embeddings)     → candle MiniLM in-process
//! - yantrik-companion (FastAPI)  → axum in-process
//!
//! Usage:
//!   yantrik serve --config config.yaml
//!   yantrik chat --config config.yaml
//!   yantrik voice --config config.yaml
//!   yantrik think --db memory.db
//!   yantrik stats --db memory.db

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use yantrik_companion::{CompanionConfig, CompanionService};
use yantrik_ml::{CandleEmbedder, CandleLLM, GGUFFiles, LLMBackend};
use yantrik_ml::ApiLLM;
use yantrik_ml::{FallbackLLM, FallbackConfig};
#[cfg(feature = "claude-cli")]
use yantrik_ml::ClaudeCliLLM;
#[cfg(feature = "llamacpp")]
use yantrik_ml::LlamaCppLLM;

#[derive(Parser)]
#[command(name = "yantrik", about = "Personal AI companion — single binary")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the HTTP server + background cognition.
    Serve {
        /// Path to config.yaml.
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Interactive CLI chat (no HTTP server).
    Chat {
        /// Path to config.yaml.
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Voice conversation mode (microphone + speaker).
    Voice {
        /// Path to config.yaml.
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    /// Run a single think cycle on the database.
    Think {
        /// Path to memory database.
        #[arg(long, default_value = "memory.db")]
        db: PathBuf,
    },
    /// Send a single message through the full companion pipeline and exit.
    Ask {
        /// The message to send.
        message: String,
        /// Path to config.yaml.
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Output JSON metadata after response (memories, tools, bond).
        #[arg(long)]
        json: bool,
    },
    /// Show memory statistics.
    Stats {
        /// Path to memory database.
        #[arg(long, default_value = "memory.db")]
        db: PathBuf,
    },
}

fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { config } => cmd_serve(config),
        Commands::Chat { config } => cmd_chat(config),
        Commands::Ask { message, config, json } => cmd_ask(config, &message, json),
        Commands::Voice { config } => cmd_voice(config),
        Commands::Think { db } => cmd_think(db),
        Commands::Stats { db } => cmd_stats(db),
    }
}

fn load_config(path: Option<PathBuf>) -> CompanionConfig {
    match path {
        Some(p) => {
            tracing::info!(path = %p.display(), "Loading config");
            CompanionConfig::from_yaml(&p).expect("failed to load config")
        }
        None => {
            tracing::info!("Using default config");
            CompanionConfig::default()
        }
    }
}

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
        let model = config.llm.api_model.clone();
        let max_tokens = config.llm.max_tokens;
        tracing::info!(model = ?model, "Using Claude Code CLI backend");
        #[cfg(feature = "claude-cli")]
        { std::sync::Arc::new(ClaudeCliLLM::new(model, max_tokens)) }
        #[cfg(not(feature = "claude-cli"))]
        { panic!("claude-cli feature not enabled at compile time") }
    } else if config.llm.is_api_backend() {
        let base_url = config.llm.resolve_api_base_url()
            .expect("api_base_url required for API backend");
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
        let ctx_size = config.llm.max_context_tokens as u32;
        tracing::info!(gguf, gpu_layers, ctx_size, "Using llama.cpp backend");
        #[cfg(feature = "llamacpp")]
        { std::sync::Arc::new(LlamaCppLLM::from_gguf(
            std::path::Path::new(gguf), gpu_layers, ctx_size,
        ).expect("failed to load llama.cpp model")) }
        #[cfg(not(feature = "llamacpp"))]
        { panic!("llamacpp feature not enabled at compile time") }
    } else if let Some(ref dir) = config.llm.model_dir {
        tracing::info!(dir, "Loading LLM from directory");
        std::sync::Arc::new(CandleLLM::from_dir(std::path::Path::new(dir))
            .expect("failed to load LLM from directory"))
    } else if let (Some(ref gguf), Some(ref tok)) = (&config.llm.gguf_path, &config.llm.tokenizer_path) {
        tracing::info!(gguf, tok, "Loading LLM from explicit paths");
        std::sync::Arc::new(CandleLLM::from_gguf(
            std::path::Path::new(gguf),
            std::path::Path::new(tok),
        )
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
        std::sync::Arc::new(CandleLLM::from_gguf(&files.gguf, &files.tokenizer)
            .expect("failed to load LLM"))
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
    let mut db = yantrikdb_core::YantrikDB::new(&config.yantrikdb.db_path, config.yantrikdb.embedding_dim)
        .expect("failed to create YantrikDB");
    db.set_embedder(Box::new(embedder));

    tracing::info!(
        db_path = config.yantrikdb.db_path,
        user = config.user_name,
        "Companion initialized"
    );

    let config_services = config.enabled_services.clone();
    let mut companion = CompanionService::new(db, llm, config);

    // Apply Skill Store snapshot — overrides services, filters instincts,
    // extends core tools based on enabled skills.
    let skills_dir = if std::path::Path::new("/opt/yantrik/skills").exists() {
        std::path::PathBuf::from("/opt/yantrik/skills")
    } else {
        std::env::current_dir().unwrap_or_default().join("skills")
    };
    let snapshot = yantrik_companion::skills::load_skill_snapshot_with_services(&skills_dir, &config_services);
    companion.apply_skill_snapshot(&snapshot);

    companion
}

fn cmd_serve(config_path: Option<PathBuf>) {
    let config = load_config(config_path);
    let host = config.server.host.clone();
    let port = config.server.port;
    let think_interval_secs = config.cognition.think_interval_minutes * 60;

    let companion = build_companion(config);

    // Create shared state for both HTTP server and background think thread
    let start_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let state = Arc::new(yantrikdb_server::AppState {
        service: std::sync::Mutex::new(companion),
        start_time,
    });

    let router = yantrikdb_server::build_router_from_state(state.clone());

    // Spawn background cognition thread
    let think_state = state.clone();
    let think_running = Arc::new(AtomicBool::new(true));
    let think_running_clone = think_running.clone();

    let think_handle = std::thread::Builder::new()
        .name("background-cognition".into())
        .spawn(move || {
            tracing::info!(
                interval_secs = think_interval_secs,
                "Background cognition thread started"
            );

            // Cooldown trackers
            let mut execute_cooldowns: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
            let mut delivered_cooldowns: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
            let mut last_cortex_fire_ts: f64 = 0.0;
            // Fairness tracker and category budgets for the new urge selector
            let mut fairness = yantrik_companion::urge_selector::FairnessTracker::new();
            let mut budgets = yantrik_companion::urge_selector::CategoryBudget::new();
            const EXECUTE_COOLDOWN_SECS: f64 = 3600.0;
            const DELIVERED_COOLDOWN_SECS: f64 = 7200.0;
            const CORTEX_GLOBAL_COOLDOWN_SECS: f64 = 3600.0;

            while think_running_clone.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_secs(think_interval_secs));
                if !think_running_clone.load(Ordering::Relaxed) {
                    break;
                }

                let mut companion = match think_state.service.lock() {
                    Ok(guard) => guard,
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to lock companion for think cycle");
                        continue;
                    }
                };

                run_headless_think_cycle(
                    &mut companion,
                    &mut execute_cooldowns,
                    &mut delivered_cooldowns,
                    &mut last_cortex_fire_ts,
                    &mut fairness,
                    &mut budgets,
                    EXECUTE_COOLDOWN_SECS,
                    DELIVERED_COOLDOWN_SECS,
                    CORTEX_GLOBAL_COOLDOWN_SECS,
                );
            }
            tracing::info!("Background cognition thread stopped");
        })
        .expect("failed to spawn think thread");

    let addr = format!("{host}:{port}");
    tracing::info!(addr, "Starting Yantrik server with background cognition");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    rt.block_on(async {
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .expect("failed to bind");
        tracing::info!("Listening on {addr}");
        axum::serve(listener, router.into_make_service())
            .with_graceful_shutdown(shutdown_signal())
            .await
            .expect("server error");
    });

    // Clean shutdown
    think_running.store(false, Ordering::Relaxed);
    let _ = think_handle.join();
}

/// Run a single headless think cycle (no UI required).
/// Evaluates instincts, processes EXECUTE urges, delivers proactive messages via Telegram.
fn run_headless_think_cycle(
    companion: &mut CompanionService,
    execute_cooldowns: &mut std::collections::HashMap<String, f64>,
    delivered_cooldowns: &mut std::collections::HashMap<String, f64>,
    last_cortex_fire_ts: &mut f64,
    fairness: &mut yantrik_companion::urge_selector::FairnessTracker,
    budgets: &mut yantrik_companion::urge_selector::CategoryBudget,
    execute_cooldown_secs: f64,
    delivered_cooldown_secs: f64,
    cortex_global_cooldown_secs: f64,
) {
    // 1. Run db.think() — memory maintenance, triggers, patterns
    let config = yantrikdb_core::types::ThinkConfig {
        run_consolidation: false,
        ..Default::default()
    };
    if let Ok(result) = companion.db.think(&config) {
        let mut triggers: Vec<serde_json::Value> = result
            .triggers
            .iter()
            .map(|t| serde_json::json!({
                "trigger_type": t.trigger_type,
                "reason": t.reason,
                "urgency": t.urgency,
                "context": t.context,
            }))
            .collect();

        let patterns: Vec<serde_json::Value> = companion.db
            .get_patterns(None, Some("active"), 10)
            .unwrap_or_default()
            .iter()
            .map(|p| serde_json::json!({
                "pattern_type": p.pattern_type,
                "description": p.description,
                "confidence": p.confidence,
            }))
            .collect();

        let conflicts_count = companion.db
            .get_conflicts(Some("open"), None, None, None, 100)
            .map(|c| c.len())
            .unwrap_or(0);

        let valence_avg = triggers.iter()
            .find(|t| t.get("trigger_type").and_then(|v| v.as_str()) == Some("valence_trend"))
            .and_then(|t| t.get("context").and_then(|c| c.get("current_avg")).and_then(|v| v.as_f64()));

        // Check scheduler for due tasks
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

        tracing::info!(
            triggers = triggers.len(),
            patterns = patterns.len(),
            conflicts = conflicts_count,
            "Think cycle — caching cognition results"
        );

        companion.update_cognition_cache(triggers, patterns, conflicts_count, valence_avg);
    }

    // 2. Evaluate instincts
    companion.resonance.tick_phase(companion.bond_level(), 60.0);
    let state = companion.build_state();
    let mut urge_specs = companion.evaluate_instincts(&state);
    let now_ts = state.current_ts;

    // 3. Context Cortex
    if let Some(ref mut cortex) = companion.cortex {
        let attention_items = cortex.think(companion.db.conn());
        let cortex_globally_cooled = now_ts - *last_cortex_fire_ts < cortex_global_cooldown_secs;

        if !attention_items.is_empty() && !cortex_globally_cooled {
            let focus = cortex.current_focus();
            let briefing = cortex.build_briefing(companion.db.conn(), focus.as_ref(), &attention_items);
            let mut attention_hasher = std::collections::hash_map::DefaultHasher::new();
            for item in &attention_items {
                std::hash::Hash::hash(&item.summary, &mut attention_hasher);
            }
            let hash_val = std::hash::Hasher::finish(&attention_hasher);
            let cortex_cooldown = format!("cortex:situation:{:x}", hash_val);
            let cortex_urge = yantrik_companion::UrgeSpec::new("Cortex", &format!("EXECUTE {}", briefing), 0.7)
                .with_cooldown(&cortex_cooldown);
            urge_specs.push(cortex_urge);
        }

        if let Some(reflection_prompt) = cortex.maybe_deep_reflection(companion.db.conn()) {
            let reasoner_urge = yantrik_companion::UrgeSpec::new("CortexReasoner", &reflection_prompt, 0.6)
                .with_cooldown("cortex:deep_reflection");
            urge_specs.push(reasoner_urge);
        }
    }

    let idle_hrs = (now_ts - state.last_interaction_ts) / 3600.0;
    tracing::info!(
        urge_count = urge_specs.len(),
        triggers = state.pending_triggers.len(),
        patterns = state.active_patterns.len(),
        idle_hours = %format!("{idle_hrs:.1}"),
        "Instinct evaluation complete"
    );

    // 4. Separate EXECUTE urges from regular urges
    let mut execute_urges = Vec::new();
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
            let last = execute_cooldowns.get(&spec.cooldown_key).copied().unwrap_or(0.0);
            let jittered_cooldown = yantrik_companion::synthesis_gate::jitter_cooldown(execute_cooldown_secs);
            if now_ts - last < jittered_cooldown {
                tracing::debug!(key = %spec.cooldown_key, "EXECUTE urge on cooldown, skipping");
                continue;
            }
            execute_urges.push(spec.clone());
        } else {
            companion.urge_queue.push(companion.db.conn(), spec);
        }
    }

    // 5. Process EXECUTE urges through tier-based log-linear softmax selector
    let mut execute_produced_message = false;

    if !execute_urges.is_empty() {
        let recent_msgs = companion.last_sent_messages(10).to_vec();
        let bond = companion.bond_level();

        if let Some((exec_urge, serendipity)) = yantrik_companion::urge_selector::process_execute_urges(
            &execute_urges,
            fairness,
            budgets,
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
                companion.last_sent_messages(5), state.avg_user_msg_length,
            );
            instruction.push_str(&anti_rep);

            let instinct_name = exec_urge.instinct_name.clone();
            if instinct_name == "Cortex" || instinct_name == "CortexReasoner" {
                *last_cortex_fire_ts = now_ts;
            }

            tracing::info!(instinct = %instinct_name, "Executing research urge");
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
                tracing::info!(instinct = %instinct_name, "EXECUTE urge produced proactive message");
            }
        } else {
            tracing::info!("All EXECUTE urges filtered by tier-softmax selector");
        }
    }

    // 6. Check regular proactive engine
    if !execute_produced_message {
        companion.check_proactive();
    }

    // 7. Deliver proactive message via Telegram
    if let Some(msg) = companion.take_proactive_message() {
        let delivery_key = msg.urge_ids.first().cloned().unwrap_or_default();
        let jittered_delivery_cd = yantrik_companion::synthesis_gate::jitter_cooldown(delivered_cooldown_secs);
        let on_cooldown = if !delivery_key.is_empty() {
            let last = delivered_cooldowns.get(&delivery_key).copied().unwrap_or(0.0);
            now_ts - last < jittered_delivery_cd
        } else { false };

        let gate_result = yantrik_companion::synthesis_gate::evaluate(
            &msg.text, companion.last_sent_messages(10),
            companion.daily_proactive_count, companion.bond_level(),
            state.idle_seconds, false, 0.5,
        );

        let gate_suppressed = matches!(gate_result, yantrik_companion::synthesis_gate::GateDecision::Suppress { .. });
        if let yantrik_companion::synthesis_gate::GateDecision::Suppress { ref reason } = gate_result {
            tracing::info!(reason = %reason, text = msg.text, "Synthesis Gate suppressed proactive message");
            companion.record_suppressed_urge(&delivery_key, reason);
        }

        if on_cooldown {
            tracing::info!(key = %delivery_key, "Proactive message on per-key cooldown, skipping");
        } else if gate_suppressed {
            // Already logged
        } else {
            tracing::info!(
                text = msg.text,
                urges = ?msg.urge_ids,
                daily_count = companion.daily_proactive_count,
                "Delivering proactive message"
            );

            companion.record_sent_message(&msg.text);
            companion.record_proactive_context(&msg.text, msg.urge_ids.clone());

            let instinct_name = msg.urge_ids.first()
                .map(|s| s.split(':').next().unwrap_or("unknown").to_string())
                .unwrap_or_else(|| "unknown".to_string());
            companion.resonance.record_sent(&instinct_name, now_ts);

            let category = yantrik_companion::resonance::InstinctCategory::from_instinct(&instinct_name);
            companion.user_model.on_proactive_sent(&instinct_name, &format!("{:?}", category), now_ts);

            if instinct_name == "Cortex" || instinct_name == "CortexReasoner" {
                *last_cortex_fire_ts = now_ts;
            }

            // Deliver via Telegram
            if companion.config.telegram.enabled && companion.config.telegram.forward_proactive {
                match yantrik_companion::telegram::send_message(&companion.config.telegram, &msg.text) {
                    Ok(_) => tracing::info!("Proactive message sent to Telegram"),
                    Err(e) => tracing::warn!(error = %e, "Failed to send proactive to Telegram"),
                }
            } else {
                // Log to stdout if Telegram is not configured
                println!("\n[PROACTIVE] {}\n", msg.text);
            }

            if !delivery_key.is_empty() {
                delivered_cooldowns.insert(delivery_key, now_ts);
            }
        }
    } else {
        let pending = companion.urge_queue.count_pending(companion.db.conn());
        tracing::debug!(pending_urges = pending, "No proactive message this cycle");
    }

    // 8. Save adaptive user model
    companion.user_model.save(companion.db.conn());
}

/// Check if an EXECUTE urge response has no actionable content.
fn is_nothing_response(text: &str) -> bool {
    let lower = text.to_lowercase();
    let patterns = [
        "no major news", "nothing significant", "nothing to share",
        "nothing to report", "nothing interesting", "nothing noteworthy",
        "no need for a full briefing", "no breaking news", "nothing earth-shattering",
        "nothing genuinely", "nothing stands out", "no trending", "nothing worth",
        "no developments",
    ];
    patterns.iter().any(|p| lower.contains(p))
}

fn cmd_chat(config_path: Option<PathBuf>) {
    let config = load_config(config_path);
    let mut companion = build_companion(config);

    println!("Yantrik interactive chat. Type 'quit' to exit.\n");

    let stdin = std::io::stdin();
    let mut input = String::new();

    loop {
        print!("You: ");
        use std::io::Write;
        std::io::stdout().flush().ok();

        input.clear();
        match stdin.read_line(&mut input) {
            Ok(0) | Err(_) => break, // EOF or error
            _ => {}
        }

        let text = input.trim();
        if text.is_empty() {
            continue;
        }
        if text == "quit" || text == "exit" {
            break;
        }

        print!("Yantrik: ");
        std::io::stdout().flush().ok();

        let mut replace_next = false;
        let _response = companion.handle_message_streaming(text, |token| {
            if token == "__REPLACE__" {
                replace_next = true;
                return;
            }
            if replace_next {
                print!("\rYantrik: {token}");
                replace_next = false;
            } else {
                print!("{token}");
            }
            std::io::stdout().flush().ok();
        });
        println!("\n");
    }
}

fn cmd_ask(config_path: Option<PathBuf>, message: &str, json_output: bool) {
    let config = load_config(config_path);
    let mut companion = build_companion(config);

    // Print the user message so the conversation is visible
    eprintln!("You: {message}");
    eprint!("Yantrik: ");

    let mut full_response = String::new();
    let mut replace_next = false;
    let response = companion.handle_message_streaming(message, |token| {
        if token == "__REPLACE__" {
            replace_next = true;
            return;
        }
        if replace_next {
            full_response.clear();
            full_response.push_str(token);
            replace_next = false;
        } else {
            full_response.push_str(token);
        }
    });
    // Print the final (clean) response
    eprint!("{full_response}");
    eprintln!("\n");

    if json_output {
        // Structured output for programmatic use
        let meta = serde_json::json!({
            "message": message,
            "response": response.message.trim(),
            "memories_recalled": response.memories_recalled,
            "tool_calls_made": response.tool_calls_made,
            "urges_delivered": response.urges_delivered,
            "offline_mode": response.offline_mode,
        });
        println!("{}", serde_json::to_string_pretty(&meta).unwrap());
    } else {
        // Human-friendly summary on stderr
        eprintln!("--- pipeline stats ---");
        eprintln!("  memories recalled: {}", response.memories_recalled);
        if !response.tool_calls_made.is_empty() {
            eprintln!("  tools used: {}", response.tool_calls_made.join(", "));
        }
        if !response.urges_delivered.is_empty() {
            eprintln!("  urges delivered: {}", response.urges_delivered.len());
        }
        if response.offline_mode {
            eprintln!("  (offline mode — LLM unreachable)");
        }
    }
}

fn cmd_voice(config_path: Option<PathBuf>) {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use yantrik_companion::voice::{voice_profile_for_bond, SimpleVAD, VADEvent};
    use yantrik_ml::{TTSEngine, WhisperEngine};

    let config = load_config(config_path);
    let voice_config = config.voice.clone();

    // Load Whisper STT
    let stt = if let Some(ref dir) = voice_config.whisper_model_dir {
        tracing::info!(dir, "Loading Whisper from directory");
        WhisperEngine::from_dir(std::path::Path::new(dir))
            .expect("failed to load Whisper from directory")
    } else {
        tracing::info!(
            model = voice_config.whisper_model,
            "Loading Whisper from HuggingFace Hub"
        );
        WhisperEngine::from_hub(&voice_config.whisper_model)
            .expect("failed to load Whisper from hub")
    };

    // Load system TTS
    let tts = TTSEngine::new().expect("failed to initialize system TTS");

    // Build companion
    let mut companion = build_companion(config);

    println!("Yantrik voice mode. Speak into your microphone. Ctrl+C to exit.\n");

    // Set up microphone capture via cpal
    let host = cpal::default_host();
    let input_device = host
        .default_input_device()
        .expect("no microphone found — falling back to text mode is not supported yet");

    let input_config = input_device
        .default_input_config()
        .expect("failed to get default input config");

    let mic_sample_rate = input_config.sample_rate().0;
    let mic_channels = input_config.channels() as usize;
    tracing::info!(
        sample_rate = mic_sample_rate,
        channels = mic_channels,
        "Microphone configured"
    );

    // Audio buffer shared between capture thread and main thread
    let audio_buffer: Arc<std::sync::Mutex<Vec<f32>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let buffer_clone = audio_buffer.clone();

    // Running flag for graceful shutdown
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    // Handle Ctrl+C
    ctrlc_handler(running.clone());

    // Build cpal stream config
    let stream_config = cpal::StreamConfig {
        channels: input_config.channels(),
        sample_rate: input_config.sample_rate(),
        buffer_size: cpal::BufferSize::Default,
    };

    // Start microphone capture
    let input_stream = input_device
        .build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                // Convert to mono if needed, then push to buffer
                let mono: Vec<f32> = if mic_channels > 1 {
                    data.chunks(mic_channels)
                        .map(|frame| frame.iter().sum::<f32>() / mic_channels as f32)
                        .collect()
                } else {
                    data.to_vec()
                };
                if let Ok(mut buf) = buffer_clone.lock() {
                    buf.extend_from_slice(&mono);
                }
            },
            |err| {
                tracing::error!("Microphone error: {err}");
            },
            None,
        )
        .expect("failed to build input stream");

    input_stream.play().expect("failed to start microphone");
    println!("Listening...\n");

    // VAD setup
    let chunk_duration_ms: u64 = 100;
    let chunk_samples = (mic_sample_rate as u64 * chunk_duration_ms / 1000) as usize;
    let mut vad = SimpleVAD::new(
        voice_config.silence_threshold,
        voice_config.silence_duration_ms,
        chunk_duration_ms,
        300, // min 300ms of speech
    );

    let whisper_sample_rate = stt.sample_rate();
    let mut speech_buffer: Vec<f32> = Vec::new();

    // Main voice loop
    while running_clone.load(Ordering::Relaxed) {
        // Sleep briefly to let audio accumulate
        std::thread::sleep(std::time::Duration::from_millis(chunk_duration_ms));

        // Drain audio buffer
        let chunk: Vec<f32> = {
            let mut buf = audio_buffer.lock().unwrap();
            if buf.len() < chunk_samples {
                continue;
            }
            let drained: Vec<f32> = buf.drain(..chunk_samples).collect();
            drained
        };

        let event = vad.process_chunk(&chunk);

        match event {
            VADEvent::Speech | VADEvent::Silence => {
                if vad.is_in_speech() {
                    speech_buffer.extend_from_slice(&chunk);
                }
            }
            VADEvent::EndOfSpeech => {
                // Include the silence tail
                speech_buffer.extend_from_slice(&chunk);

                if speech_buffer.is_empty() {
                    continue;
                }

                // Resample to 16kHz if needed
                let pcm_16k = if mic_sample_rate != whisper_sample_rate {
                    resample(&speech_buffer, mic_sample_rate, whisper_sample_rate)
                } else {
                    speech_buffer.clone()
                };

                speech_buffer.clear();

                // STT
                print!("You: ");
                use std::io::Write;
                std::io::stdout().flush().ok();

                match stt.transcribe(&pcm_16k) {
                    Ok(result) => {
                        let text = result.text.trim().to_string();
                        if text.is_empty() {
                            println!("(silence)");
                            continue;
                        }
                        println!("{text}");

                        // Companion response
                        print!("Yantrik: ");
                        std::io::stdout().flush().ok();

                        let mut response_text = String::new();
                        let mut replace_next = false;
                        let _response =
                            companion.handle_message_streaming(&text, |token| {
                                if token == "__REPLACE__" {
                                    replace_next = true;
                                    return;
                                }
                                if replace_next {
                                    response_text.clear();
                                    print!("\rYantrik: {token}");
                                    response_text.push_str(token);
                                    replace_next = false;
                                } else {
                                    print!("{token}");
                                    response_text.push_str(token);
                                }
                                std::io::stdout().flush().ok();
                            });
                        println!("\n");

                        // TTS with bond-adaptive voice (speaks through system speakers)
                        let bond_level = companion.bond_level();
                        let profile = voice_profile_for_bond(&bond_level);
                        let params = profile.to_voice_params();

                        if let Err(e) = tts.speak(&response_text, Some(&params)) {
                            tracing::warn!("TTS failed: {e}");
                        }
                    }
                    Err(e) => {
                        println!("(STT error: {e})");
                    }
                }
            }
        }
    }

    drop(input_stream);
    println!("\nGoodbye!");
}

/// Resample audio from source_rate to target_rate using linear interpolation.
fn resample(samples: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
    if source_rate == target_rate || samples.is_empty() {
        return samples.to_vec();
    }

    use rubato::{FftFixedIn, Resampler};

    let mut resampler = FftFixedIn::<f32>::new(
        source_rate as usize,
        target_rate as usize,
        samples.len(),
        1, // sub_chunks
        1, // channels
    )
    .expect("failed to create resampler");

    let input = vec![samples.to_vec()];
    match resampler.process(&input, None) {
        Ok(output) => output.into_iter().next().unwrap_or_default(),
        Err(e) => {
            tracing::warn!("Resample failed: {e}, using original");
            samples.to_vec()
        }
    }
}

/// Install a Ctrl+C handler that sets the running flag to false.
fn ctrlc_handler(running: Arc<AtomicBool>) {
    ctrlc::set_handler(move || {
        running.store(false, Ordering::Relaxed);
    })
    .expect("failed to set Ctrl+C handler");
}

fn cmd_think(db_path: PathBuf) {
    let db = yantrikdb_core::YantrikDB::new(
        db_path.to_str().unwrap(),
        384,
    )
    .expect("failed to open YantrikDB");

    let config = yantrikdb_core::types::ThinkConfig::default();
    match db.think(&config) {
        Ok(result) => {
            println!("Think cycle complete:");
            println!("  Triggers: {}", result.triggers.len());
            println!("  Consolidations: {}", result.consolidation_count);
            println!("  Conflicts found: {}", result.conflicts_found);
            println!("  New patterns: {}", result.patterns_new);
            println!("  Duration: {}ms", result.duration_ms);
        }
        Err(e) => {
            eprintln!("Think failed: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_stats(db_path: PathBuf) {
    let db = yantrikdb_core::YantrikDB::new(
        db_path.to_str().unwrap(),
        384,
    )
    .expect("failed to open YantrikDB");

    match db.stats(None) {
        Ok(stats) => {
            println!("YantrikDB Statistics:");
            println!("  Active memories:       {}", stats.active_memories);
            println!("  Consolidated memories: {}", stats.consolidated_memories);
            println!("  Tombstoned memories:   {}", stats.tombstoned_memories);
            println!("  Archived memories:     {}", stats.archived_memories);
            println!("  Edges:                 {}", stats.edges);
            println!("  Entities:              {}", stats.entities);
        }
        Err(e) => {
            eprintln!("Stats failed: {e}");
            std::process::exit(1);
        }
    }
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
    tracing::info!("Shutting down...");
}
