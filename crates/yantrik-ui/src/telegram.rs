//! Telegram poller — background thread for bidirectional Telegram chat.
//!
//! Inbound:  Long-polls getUpdates → sends to CompanionBridge → collects
//!           streaming response → sends back via sendMessage.
//! Outbound: Receives proactive messages via channel → forwards to Telegram.
//! Voice:    Jarvis mode — voice messages transcribed via Whisper, response
//!           sent back as text + voice (espeak-ng + ffmpeg).

use std::sync::Arc;

use slint::{Model, ModelRc, SharedString, VecModel};
use yantrikdb_companion::config::{TelegramConfig, VoiceConfig};

use crate::bridge::CompanionBridge;
use crate::App;

/// Handle returned by `start_poller()`.
pub struct TelegramHandle {
    outbound_tx: crossbeam_channel::Sender<String>,
    shutdown_tx: crossbeam_channel::Sender<()>,
}

impl TelegramHandle {
    /// Queue a message to be sent to Telegram (e.g. proactive messages).
    pub fn send(&self, text: String) {
        let _ = self.outbound_tx.send(text);
    }

    /// Signal the poller to shut down.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}

/// Start the Telegram polling thread. Returns a handle for outbound messages.
pub fn start_poller(
    config: TelegramConfig,
    bridge: Arc<CompanionBridge>,
    ui_weak: slint::Weak<App>,
    voice_config: VoiceConfig,
) -> TelegramHandle {
    let (outbound_tx, outbound_rx) = crossbeam_channel::unbounded::<String>();
    let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded::<()>(1);

    let config_clone = config.clone();
    std::thread::Builder::new()
        .name("telegram-poller".into())
        .spawn(move || {
            poller_loop(config_clone, bridge, ui_weak, outbound_rx, shutdown_rx, voice_config);
        })
        .expect("Failed to start Telegram poller thread");

    TelegramHandle {
        outbound_tx,
        shutdown_tx,
    }
}

/// Check if current local time is within quiet hours (22:00–06:00).
fn is_quiet_hours() -> bool {
    let hour: u32 = std::process::Command::new("date")
        .arg("+%H")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(12);
    hour >= 22 || hour < 6
}

fn poller_loop(
    config: TelegramConfig,
    bridge: Arc<CompanionBridge>,
    ui_weak: slint::Weak<App>,
    outbound_rx: crossbeam_channel::Receiver<String>,
    shutdown_rx: crossbeam_channel::Receiver<()>,
    voice_config: VoiceConfig,
) {
    // Check voice dependencies (ffmpeg + espeak-ng) once at startup
    let voice_available = match yantrikdb_companion::audio_convert::check_dependencies() {
        Ok(()) => {
            tracing::info!("Telegram voice: ffmpeg + espeak-ng available");
            true
        }
        Err(e) => {
            tracing::warn!("Telegram voice disabled: {e}");
            false
        }
    };
    let mut offset: i64 = 0;
    let poll_timeout = config.poll_interval_secs.max(1);

    // V15: Daily digest buffer — messages queued during quiet hours
    let mut digest_buffer: Vec<String> = Vec::new();
    let mut was_quiet = is_quiet_hours();

    tracing::info!("Telegram poller started (poll_interval={}s)", poll_timeout);

    loop {
        // Check for shutdown signal
        if shutdown_rx.try_recv().is_ok() {
            tracing::info!("Telegram poller shutting down");
            break;
        }

        // V15: Check quiet hours transition — flush digest when quiet hours end
        let now_quiet = is_quiet_hours();
        if was_quiet && !now_quiet && !digest_buffer.is_empty() {
            let count = digest_buffer.len();
            let digest = format!(
                "\u{1f305} Morning digest ({} messages overnight):\n\n{}",
                count,
                digest_buffer.join("\n\n\u{2500}\u{2500}\u{2500}\n\n")
            );
            if let Err(e) = yantrikdb_companion::telegram::send_message(&config, &digest) {
                tracing::warn!(error = %e, "Failed to send Telegram digest");
            } else {
                tracing::info!(count, "Telegram daily digest sent");
            }
            digest_buffer.clear();
        }
        was_quiet = now_quiet;

        // Process any outbound messages (proactive messages forwarded from bridge)
        while let Ok(text) = outbound_rx.try_recv() {
            if now_quiet {
                // Buffer during quiet hours
                digest_buffer.push(text);
                tracing::debug!(
                    buffered = digest_buffer.len(),
                    "Telegram message buffered (quiet hours)"
                );
            } else if let Err(e) = yantrikdb_companion::telegram::send_message(&config, &text) {
                tracing::warn!(error = %e, "Failed to send outbound Telegram message");
            }
        }

        // Long-poll for inbound messages
        match yantrikdb_companion::telegram::get_updates(&config, offset, poll_timeout) {
            Ok(updates) => {
                for update in updates {
                    offset = update.update_id + 1;

                    // Voice message — Jarvis pipeline
                    if update.voice.is_some() && voice_available {
                        handle_voice_message(
                            &config, &bridge, &ui_weak, &update, &voice_config,
                        );
                        continue;
                    }

                    // Skip voice-only messages when voice deps aren't available
                    if update.text.is_empty() {
                        continue;
                    }

                    tracing::info!(
                        text = %update.text,
                        chat_id = %update.chat_id,
                        "Telegram inbound message"
                    );

                    // Add user message to desktop UI chat
                    let user_text = update.text.clone();
                    let weak = ui_weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = weak.upgrade() {
                            let messages = ui.get_messages();
                            let model = messages
                                .as_any()
                                .downcast_ref::<VecModel<crate::MessageData>>()
                                .unwrap();
                            model.push(crate::MessageData {
                                role: SharedString::from("user"),
                                content: SharedString::from(format!("[Telegram] {}", user_text)),
                                is_streaming: false,
                                blocks: ModelRc::default(),
                            });
                        }
                    });

                    // React with eyes emoji to show we're reading the message
                    let _ = yantrikdb_companion::telegram::set_reaction(
                        &config, update.message_id, "\u{1f440}",
                    );

                    // Show "typing..." indicator
                    let _ = yantrikdb_companion::telegram::send_typing(&config);

                    // Send to companion and collect full response
                    let token_rx = bridge.send_message(update.text);
                    let mut response = String::new();
                    let mut typing_refresh = std::time::Instant::now();

                    // Collect all streaming tokens.
                    // __REPLACE__ means "discard everything so far, next token is the new start".
                    // This can happen multiple times (tool progress → final response).
                    while let Ok(token) = token_rx.recv() {
                        if token == "__DONE__" {
                            break;
                        }
                        if token == "__REPLACE__" {
                            response.clear();
                            continue;
                        }
                        response.push_str(&token);

                        // Refresh typing indicator every 4s (Telegram expires it after 5s)
                        if typing_refresh.elapsed().as_secs() >= 4 {
                            let _ = yantrikdb_companion::telegram::send_typing(&config);
                            typing_refresh = std::time::Instant::now();
                        }
                    }

                    // Strip any leftover tool-progress lines like "[Using recall...]"
                    let clean: String = response
                        .lines()
                        .filter(|line| !line.starts_with("[Using "))
                        .collect::<Vec<_>>()
                        .join("\n");
                    response = clean.trim().to_string();

                    if response.is_empty() {
                        response = "(no response)".to_string();
                    }

                    // Clear the eyes reaction now that we're responding
                    let _ = yantrikdb_companion::telegram::clear_reaction(
                        &config, update.message_id,
                    );

                    // Send response back to Telegram
                    if let Err(e) = yantrikdb_companion::telegram::send_message(&config, &response) {
                        tracing::warn!(error = %e, "Failed to send Telegram response");
                    }

                    // Add assistant response to desktop UI
                    let resp_text = response.clone();
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
                                content: SharedString::from(format!("[Telegram] {}", resp_text)),
                                is_streaming: false,
                                blocks: ModelRc::default(),
                            });
                        }
                    });
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Telegram getUpdates failed");
                // Back off on error
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }
    }
}

/// Jarvis pipeline: voice message → STT → companion → text + TTS voice reply.
fn handle_voice_message(
    config: &TelegramConfig,
    bridge: &Arc<CompanionBridge>,
    ui_weak: &slint::Weak<App>,
    update: &yantrikdb_companion::telegram::TelegramUpdate,
    voice_config: &VoiceConfig,
) {
    let voice = match &update.voice {
        Some(v) => v,
        None => return,
    };

    tracing::info!(
        duration = voice.duration,
        message_id = update.message_id,
        "Telegram voice message received"
    );

    // React with eyes + typing to show we're processing
    let _ = yantrikdb_companion::telegram::set_reaction(
        config, update.message_id, "\u{1f440}",
    );
    let _ = yantrikdb_companion::telegram::send_typing(config);

    // 1. Download the voice file
    let ogg_path = format!("/tmp/tg_voice_{}.ogg", update.message_id);
    let file_path = match yantrikdb_companion::telegram::get_file(config, &voice.file_id) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to get voice file path");
            let _ = yantrikdb_companion::telegram::send_message(
                config, "(couldn't download your voice message)",
            );
            let _ = yantrikdb_companion::telegram::clear_reaction(config, update.message_id);
            return;
        }
    };

    if let Err(e) = yantrikdb_companion::telegram::download_file(config, &file_path, &ogg_path) {
        tracing::warn!(error = %e, "Failed to download voice file");
        let _ = yantrikdb_companion::telegram::send_message(
            config, "(couldn't download your voice message)",
        );
        let _ = yantrikdb_companion::telegram::clear_reaction(config, update.message_id);
        return;
    }

    // 2. Convert OGG → PCM f32
    let pcm = match yantrikdb_companion::audio_convert::ogg_to_pcm_f32(&ogg_path) {
        Ok(samples) => {
            tracing::info!(samples = samples.len(), "Voice decoded to PCM");
            samples
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to decode voice OGG");
            let _ = yantrikdb_companion::telegram::send_message(
                config, "(couldn't decode your voice message)",
            );
            let _ = std::fs::remove_file(&ogg_path);
            let _ = yantrikdb_companion::telegram::clear_reaction(config, update.message_id);
            return;
        }
    };

    // Clean up input file
    let _ = std::fs::remove_file(&ogg_path);

    // 3. Whisper STT
    let transcribed = match load_whisper(voice_config) {
        Some(stt) => match stt.transcribe(&pcm) {
            Ok(result) => {
                let t = result.text.trim().to_string();
                if t.is_empty() {
                    tracing::info!("Voice message transcribed to empty text");
                    let _ = yantrikdb_companion::telegram::send_message(
                        config, "(couldn't understand your voice message)",
                    );
                    let _ = yantrikdb_companion::telegram::clear_reaction(config, update.message_id);
                    return;
                }
                tracing::info!(text = %t, "Voice transcribed");
                t
            }
            Err(e) => {
                tracing::warn!(error = %e, "Whisper STT failed");
                let _ = yantrikdb_companion::telegram::send_message(
                    config, "(speech recognition failed)",
                );
                let _ = yantrikdb_companion::telegram::clear_reaction(config, update.message_id);
                return;
            }
        },
        None => {
            tracing::warn!("Whisper engine not available");
            let _ = yantrikdb_companion::telegram::send_message(
                config, "(speech recognition not configured)",
            );
            let _ = yantrikdb_companion::telegram::clear_reaction(config, update.message_id);
            return;
        }
    };

    // 4. Show transcription in desktop UI
    let ui_text = format!("[Voice] {}", &transcribed);
    let weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            let messages = ui.get_messages();
            let model = messages
                .as_any()
                .downcast_ref::<VecModel<crate::MessageData>>()
                .unwrap();
            model.push(crate::MessageData {
                role: SharedString::from("user"),
                content: SharedString::from(ui_text),
                is_streaming: false,
                blocks: ModelRc::default(),
            });
        }
    });

    // 5. Send to companion and collect response
    let token_rx = bridge.send_message(transcribed);
    let mut response = String::new();
    let mut typing_refresh = std::time::Instant::now();

    while let Ok(token) = token_rx.recv() {
        if token == "__DONE__" {
            break;
        }
        if token == "__REPLACE__" {
            response.clear();
            continue;
        }
        response.push_str(&token);

        if typing_refresh.elapsed().as_secs() >= 4 {
            let _ = yantrikdb_companion::telegram::send_typing(config);
            typing_refresh = std::time::Instant::now();
        }
    }

    // Strip tool progress lines
    let clean: String = response
        .lines()
        .filter(|line| !line.starts_with("[Using "))
        .collect::<Vec<_>>()
        .join("\n");
    response = clean.trim().to_string();

    if response.is_empty() {
        response = "(no response)".to_string();
    }

    // 6. Clear eyes reaction
    let _ = yantrikdb_companion::telegram::clear_reaction(config, update.message_id);

    // 7. Send text response
    if let Err(e) = yantrikdb_companion::telegram::send_message(config, &response) {
        tracing::warn!(error = %e, "Failed to send text response");
    }

    // 8. Generate and send voice response
    let _ = yantrikdb_companion::telegram::send_recording_voice(config);

    let (rate, pitch) = tts_params_for_bond(bridge);
    let reply_ogg = format!("/tmp/tg_reply_{}.ogg", update.message_id);

    match yantrikdb_companion::audio_convert::text_to_ogg(&response, &reply_ogg, rate, pitch) {
        Ok(()) => {
            if let Err(e) = yantrikdb_companion::telegram::send_voice(config, &reply_ogg) {
                tracing::warn!(error = %e, "Failed to send voice reply");
            } else {
                tracing::info!("Voice reply sent");
            }
            let _ = std::fs::remove_file(&reply_ogg);
        }
        Err(e) => {
            tracing::warn!(error = %e, "TTS failed, text-only response sent");
        }
    }

    // 9. Add to desktop UI
    let resp_text = response;
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
                content: SharedString::from(format!("[Telegram] {}", resp_text)),
                is_streaming: false,
                blocks: ModelRc::default(),
            });
        }
    });
}

/// Lazily load Whisper STT engine (loaded once on first voice message).
fn load_whisper(voice_config: &VoiceConfig) -> Option<&'static yantrikdb_ml::WhisperEngine> {
    use std::sync::OnceLock;
    static WHISPER: OnceLock<Option<yantrikdb_ml::WhisperEngine>> = OnceLock::new();

    WHISPER.get_or_init(|| {
        tracing::info!("Loading Whisper for Telegram voice...");
        let result = if let Some(ref dir) = voice_config.whisper_model_dir {
            yantrikdb_ml::WhisperEngine::from_dir(std::path::Path::new(dir))
        } else {
            yantrikdb_ml::WhisperEngine::from_hub(&voice_config.whisper_model)
        };
        match result {
            Ok(engine) => {
                tracing::info!("Whisper loaded for Telegram voice");
                Some(engine)
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to load Whisper for Telegram");
                None
            }
        }
    }).as_ref()
}

/// Get espeak-ng TTS parameters (rate, pitch) adapted to companion's bond level.
fn tts_params_for_bond(bridge: &CompanionBridge) -> (u32, u32) {
    let bond_cached = bridge.bond_level_cached();
    // Map bond level (1-5) to espeak-ng rate (wpm) and pitch (0-99)
    match bond_cached {
        1 => (160, 45),  // Stranger: formal, measured
        2 => (170, 48),  // Acquaintance: warmer
        3 => (180, 50),  // Friend: natural
        4 => (185, 52),  // Confidant: expressive
        5 => (190, 55),  // Partner-in-Crime: fast, lively
        _ => (175, 50),  // Default
    }
}
