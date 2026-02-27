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
use yantrikdb_companion::{CompanionConfig, CompanionService};
use yantrikdb_ml::{CandleEmbedder, CandleLLM, GGUFFiles};

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

    // Load LLM
    let llm = if let Some(ref dir) = config.llm.model_dir {
        tracing::info!(dir, "Loading LLM from directory");
        CandleLLM::from_dir(std::path::Path::new(dir))
            .expect("failed to load LLM from directory")
    } else if let (Some(ref gguf), Some(ref tok)) = (&config.llm.gguf_path, &config.llm.tokenizer_path) {
        tracing::info!(gguf, tok, "Loading LLM from explicit paths");
        CandleLLM::from_gguf(
            std::path::Path::new(gguf),
            std::path::Path::new(tok),
        )
        .expect("failed to load LLM")
    } else {
        tracing::info!("Downloading Qwen2.5-0.5B from HuggingFace Hub");
        let files = GGUFFiles::from_hub(
            "Qwen/Qwen2.5-0.5B-Instruct-GGUF",
            "qwen2.5-0.5b-instruct-q4_k_m.gguf",
            "Qwen/Qwen2.5-0.5B-Instruct",
        )
        .expect("failed to download LLM");
        CandleLLM::from_gguf(&files.gguf, &files.tokenizer)
            .expect("failed to load LLM")
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

    CompanionService::new(db, Box::new(llm), config)
}

fn cmd_serve(config_path: Option<PathBuf>) {
    let config = load_config(config_path);
    let host = config.server.host.clone();
    let port = config.server.port;

    let companion = build_companion(config);
    let router = yantrikdb_server::build_router(companion);

    let addr = format!("{host}:{port}");
    tracing::info!(addr, "Starting Yantrik server");

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

        let _response = companion.handle_message_streaming(text, |token| {
            print!("{token}");
            std::io::stdout().flush().ok();
        });
        println!("\n");
    }
}

fn cmd_voice(config_path: Option<PathBuf>) {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use yantrikdb_companion::voice::{voice_profile_for_bond, SimpleVAD, VADEvent};
    use yantrikdb_ml::{TTSEngine, WhisperEngine};

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
                        let _response =
                            companion.handle_message_streaming(&text, |token| {
                                print!("{token}");
                                std::io::stdout().flush().ok();
                                response_text.push_str(token);
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
