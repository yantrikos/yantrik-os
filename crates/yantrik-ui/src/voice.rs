//! Voice Runtime — persistent, always-available voice interaction engine.
//!
//! Replaces the old session-based VoiceSession with a continuous runtime that:
//! - Responds to push-to-talk activation (Super+Space)
//! - Supports conversational follow mode (15s window after each exchange)
//! - Implements barge-in (instant TTS stop when user speaks during playback)
//! - Streams the reply → sentence-split → TTS for low latency
//! - Puts OS context (active app, selection, etc.) in each turn's context
//!
//! What the person says goes to whichever mind the Lens talks to — the built-in, Hermes, Pi, any
//! attached mind — through the Lens's own send path (`wire::chat::send_turn`), with
//! `Origin::voice()`. The conversation is the Lens's: the spoken turn and its reply show there.
//!
//! Whisper STT and TTS run on this machine.
//!
//! LAZY INITIALIZATION: Models (Whisper, TTS) and audio devices are only loaded
//! on first activation, not at boot. This avoids blocking the UI during startup
//! and gracefully handles VMs without audio hardware.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use yantrik_companion::config::VoiceConfig;
use yantrik_companion::voice::{voice_profile_for_bond, SimpleVAD, VADEvent, VoiceProfile};
use yantrik_ml::{TTSEngine, WhisperEngine};

use crate::bridge::CompanionBridge;
use crate::wire::chat::TurnStream;
use crate::App;

/// What a voice turn sends: the person's own words, and what the desk was showing as they said
/// them, which goes in the turn's context and never in the words.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceTurn {
    pub text: String,
    pub desk: String,
    pub origin: yantrik_harness::protocol::Origin,
}

/// The turn for what Whisper heard.
pub fn voice_turn(transcript: &str, os_context: &str) -> VoiceTurn {
    VoiceTurn {
        text: transcript.trim().to_string(),
        desk: os_context.trim().to_string(),
        origin: yantrik_harness::protocol::Origin::voice(),
    }
}

/// Hands a voice turn to the Lens's send path on the UI thread and returns its reply; None when
/// the shell could not take it.
pub type SendTurn = Arc<dyn Fn(VoiceTurn) -> Option<TurnStream> + Send + Sync>;

// ─── Voice mode state machine ───────────────────────────────────────────────

/// Voice runtime mode.
/// Stored as AtomicU8 for lock-free thread sharing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VoiceMode {
    /// Not listening. Waiting for push-to-talk activation.
    Idle = 0,
    /// Microphone active, capturing speech via VAD.
    Listening = 1,
    /// Speech captured, running STT + LLM processing.
    Processing = 2,
    /// Playing TTS response. Monitoring mic for barge-in.
    Speaking = 3,
    /// Follow mode — listening for 15s without requiring hotkey.
    Following = 4,
}

impl VoiceMode {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Idle,
            1 => Self::Listening,
            2 => Self::Processing,
            3 => Self::Speaking,
            4 => Self::Following,
            _ => Self::Idle,
        }
    }

    /// Map to Slint voice-state property (0=listening, 1=transcribing, 2=speaking).
    fn to_ui_state(self) -> i32 {
        match self {
            Self::Idle => -1,       // hidden
            Self::Listening => 0,   // listening
            Self::Processing => 1,  // transcribing
            Self::Speaking => 2,    // speaking
            Self::Following => 0,   // listening (follow mode)
        }
    }
}

// ─── Shared state ───────────────────────────────────────────────────────────

/// Thread-safe shared state between VoiceRuntime, UI thread, and hotkey handler.
pub struct VoiceState {
    /// Current mode (AtomicU8 for lock-free access).
    mode: AtomicU8,
    /// Signal to activate listening (set by hotkey handler, cleared by runtime).
    activate: AtomicBool,
    /// Signal to stop TTS immediately (barge-in).
    stop_tts: AtomicBool,
    /// Signal to shut down the runtime.
    shutdown: AtomicBool,
    /// Conversation turn count (for follow mode context).
    turn_count: AtomicI32,
    /// OS context snapshot for the next voice turn.
    context_snapshot: Mutex<String>,
}

impl VoiceState {
    fn new() -> Self {
        Self {
            mode: AtomicU8::new(VoiceMode::Idle as u8),
            activate: AtomicBool::new(false),
            stop_tts: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            turn_count: AtomicI32::new(0),
            context_snapshot: Mutex::new(String::new()),
        }
    }

    pub fn mode(&self) -> VoiceMode {
        VoiceMode::from_u8(self.mode.load(Ordering::Relaxed))
    }

    fn set_mode(&self, mode: VoiceMode) {
        self.mode.store(mode as u8, Ordering::Relaxed);
    }

    /// Called by the hotkey handler (Super+Space) to start listening.
    pub fn activate(&self) {
        self.activate.store(true, Ordering::Relaxed);
    }

    /// Check and clear the activation signal.
    fn take_activate(&self) -> bool {
        self.activate.swap(false, Ordering::Relaxed)
    }

    /// Signal barge-in — stop TTS immediately. The mic button and Super+Space call it while a
    /// reply is being spoken: the runtime is inside the reply loop then, which reads this signal
    /// and not `activate`.
    pub fn request_stop_tts(&self) {
        self.stop_tts.store(true, Ordering::Relaxed);
    }

    /// Check and clear the stop-TTS signal.
    fn take_stop_tts(&self) -> bool {
        self.stop_tts.swap(false, Ordering::Relaxed)
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Relaxed)
    }

    /// Set the OS context snapshot for the next voice turn.
    pub fn set_context(&self, ctx: String) {
        if let Ok(mut c) = self.context_snapshot.lock() {
            *c = ctx;
        }
    }

    /// Take the current context snapshot (clears it).
    fn take_context(&self) -> String {
        self.context_snapshot
            .lock()
            .ok()
            .map(|mut c| std::mem::take(&mut *c))
            .unwrap_or_default()
    }
}

// ─── Voice Runtime ──────────────────────────────────────────────────────────

/// The persistent voice runtime. Created once at startup, lives for the entire session.
pub struct VoiceRuntime {
    pub state: Arc<VoiceState>,
    handle: Option<JoinHandle<()>>,
}

impl VoiceRuntime {
    /// Start the voice runtime on a background thread.
    /// Returns immediately — the runtime runs until shutdown() is called.
    ///
    /// LAZY: Models and audio devices are NOT loaded until first activation.
    /// The thread just sleeps in idle mode, consuming zero CPU.
    pub fn start(
        bridge: Arc<CompanionBridge>,
        ui_weak: slint::Weak<App>,
        voice_config: VoiceConfig,
        send: SendTurn,
    ) -> Self {
        let state = Arc::new(VoiceState::new());
        let state_clone = state.clone();

        let handle = std::thread::Builder::new()
            .name("voice-runtime".into())
            .spawn(move || {
                if let Err(e) = voice_runtime_loop(bridge, ui_weak, voice_config, state_clone, send) {
                    tracing::error!("Voice runtime error: {e}");
                }
            })
            .expect("failed to spawn voice runtime thread");

        tracing::info!("Voice runtime started (lazy — models load on first activation)");
        Self {
            state,
            handle: Some(handle),
        }
    }

    /// Shut down the voice runtime and wait for the thread to finish.
    pub fn stop(&mut self) {
        self.state.shutdown();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for VoiceRuntime {
    fn drop(&mut self) {
        self.stop();
    }
}

// ─── Follow mode config ─────────────────────────────────────────────────────

const FOLLOW_MODE_DURATION: Duration = Duration::from_secs(15);
const BARGE_IN_ENERGY_THRESHOLD: f32 = 0.015;

// ─── Lazy-loaded audio pipeline ─────────────────────────────────────────────

/// TTS backend — tries Piper neural first, falls back to system TTS.
enum SmartTTS {
    Piper(yantrik_ml::PiperTTS),
    System(TTSEngine),
}

impl SmartTTS {
    fn speak(&self, text: &str, params: Option<&yantrik_ml::VoiceParams>) -> anyhow::Result<()> {
        match self {
            SmartTTS::Piper(p) => p.speak(text, params),
            SmartTTS::System(s) => s.speak(text, params),
        }
    }

    fn is_speaking(&self) -> bool {
        match self {
            SmartTTS::Piper(p) => p.is_speaking(),
            SmartTTS::System(s) => s.is_speaking(),
        }
    }
}

/// All the heavy resources that are loaded lazily on first voice activation.
struct AudioPipeline {
    stt: WhisperEngine,
    tts: SmartTTS,
    audio_buffer: Arc<Mutex<Vec<f32>>>,
    mic_energy: Arc<Mutex<f32>>,
    _input_stream: cpal::Stream, // kept alive via ownership
    mic_sample_rate: u32,
    whisper_sample_rate: u32,
    chunk_samples: usize,
}

/// The microphone, or why there is none. Asked once at start, without loading a model, so the
/// Lens's mic button can say so before anyone presses it; and again by the pipeline.
fn input_device() -> anyhow::Result<cpal::Device> {
    cpal::default_host()
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("no microphone found"))
}

/// Tell the Lens's mic button that voice cannot work here, and why.
fn set_mic_unavailable(ui_weak: &slint::Weak<App>, why: String) {
    let weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_voice_unavailable(why.into());
        }
    });
}

/// Try to initialize the audio pipeline. Returns None if audio hardware is unavailable.
fn init_audio_pipeline(voice_config: &VoiceConfig) -> anyhow::Result<AudioPipeline> {
    // Load Whisper STT
    tracing::info!("Voice: loading STT engine...");
    let stt = if let Some(ref dir) = voice_config.whisper_model_dir {
        tracing::info!(dir, "Voice: loading Whisper from directory");
        WhisperEngine::from_dir(std::path::Path::new(dir))?
    } else {
        tracing::info!(model = voice_config.whisper_model, "Voice: loading Whisper from Hub");
        WhisperEngine::from_hub(&voice_config.whisper_model)?
    };

    // Load TTS — try Piper (neural) first, fall back to system TTS
    tracing::info!("Voice: loading TTS engine...");
    let tts = {
        let tts_dir = std::path::Path::new("/opt/yantrik/models/tts");
        match yantrik_ml::PiperTTS::from_dir(tts_dir) {
            Ok(piper) => {
                tracing::info!("Voice: using Piper neural TTS");
                SmartTTS::Piper(piper)
            }
            Err(e) => {
                tracing::info!(err = %e, "Voice: Piper not available, using system TTS");
                SmartTTS::System(TTSEngine::new()?)
            }
        }
    };

    // Set up microphone
    let input_device = input_device()?;

    let input_config = input_device.default_input_config()?;
    let mic_sample_rate = input_config.sample_rate().0;
    let mic_channels = input_config.channels() as usize;
    tracing::info!(sample_rate = mic_sample_rate, channels = mic_channels, "Voice: mic configured");

    let audio_buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let buffer_clone = audio_buffer.clone();

    let mic_energy: Arc<Mutex<f32>> = Arc::new(Mutex::new(0.0));
    let energy_clone = mic_energy.clone();

    let stream_config = cpal::StreamConfig {
        channels: input_config.channels(),
        sample_rate: input_config.sample_rate(),
        buffer_size: cpal::BufferSize::Default,
    };

    let input_stream = input_device.build_input_stream(
        &stream_config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            let mono: Vec<f32> = if mic_channels > 1 {
                data.chunks(mic_channels)
                    .map(|frame| frame.iter().sum::<f32>() / mic_channels as f32)
                    .collect()
            } else {
                data.to_vec()
            };

            if !mono.is_empty() {
                let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len() as f32).sqrt();
                if let Ok(mut e) = energy_clone.lock() {
                    *e = rms;
                }
            }

            if let Ok(mut buf) = buffer_clone.lock() {
                buf.extend_from_slice(&mono);
            }
        },
        |err| {
            tracing::error!("Microphone error: {err}");
        },
        None,
    )?;

    input_stream.play()?;

    let chunk_duration_ms: u64 = 100;
    let chunk_samples = (mic_sample_rate as u64 * chunk_duration_ms / 1000) as usize;
    let whisper_sample_rate = stt.sample_rate();

    tracing::info!("Voice: audio pipeline initialized");

    Ok(AudioPipeline {
        stt,
        tts,
        audio_buffer,
        mic_energy,
        _input_stream: input_stream,
        mic_sample_rate,
        whisper_sample_rate,
        chunk_samples,
    })
}

// ─── The main runtime loop ──────────────────────────────────────────────────

fn voice_runtime_loop(
    bridge: Arc<CompanionBridge>,
    ui_weak: slint::Weak<App>,
    voice_config: VoiceConfig,
    state: Arc<VoiceState>,
    send: SendTurn,
) -> anyhow::Result<()> {
    // ── LAZY: Don't load anything at boot. Just wait for activation. ──
    let mut pipeline: Option<AudioPipeline> = None;
    let mut vad: Option<SimpleVAD> = None;
    let mut speech_buffer: Vec<f32> = Vec::new();
    let mut follow_mode_deadline: Option<Instant> = None;
    let mut init_failed = false; // Don't retry if audio init already failed

    // No input device (a VM without audio): the mic button is drawn disabled and says why. Only
    // the device is asked here; the models still load on first activation.
    if let Err(e) = input_device() {
        tracing::info!("Voice: {e}; the mic button is disabled");
        set_mic_unavailable(&ui_weak, format!("Voice is unavailable: {e}."));
    }

    tracing::info!("Voice runtime: idle (waiting for activation)");

    let chunk_duration_ms: u64 = 100;

    // ── Main loop ──
    while !state.is_shutdown() {
        let current_mode = state.mode();

        // ── Check for activation signal ──
        if state.take_activate() {
            match current_mode {
                VoiceMode::Idle => {
                    // ── LAZY INIT: Load models + mic on first activation ──
                    if pipeline.is_none() && !init_failed {
                        tracing::info!("Voice: first activation — initializing audio pipeline...");
                        match init_audio_pipeline(&voice_config) {
                            Ok(p) => {
                                vad = Some(SimpleVAD::new(
                                    voice_config.silence_threshold,
                                    voice_config.silence_duration_ms,
                                    chunk_duration_ms,
                                    300,
                                ));
                                pipeline = Some(p);
                                // A microphone plugged in after boot: the Lens's mic works now.
                                set_mic_unavailable(&ui_weak, String::new());
                                tracing::info!("Voice: audio pipeline ready");
                            }
                            Err(e) => {
                                tracing::error!("Voice: audio init failed: {e}");
                                tracing::error!("Voice: voice interaction unavailable (no audio hardware?)");
                                init_failed = true;
                                // Show brief error toast via UI
                                let weak = ui_weak.clone();
                                let err_msg = format!("Voice unavailable: {e}");
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(ui) = weak.upgrade() {
                                        ui.set_voice_active(false);
                                        ui.set_voice_unavailable(format!("{err_msg}.").into());
                                        ui.set_voice_response(err_msg.into());
                                    }
                                });
                                continue;
                            }
                        }
                    }

                    if pipeline.is_some() {
                        state.set_mode(VoiceMode::Listening);
                        set_voice_ui(&ui_weak, VoiceMode::Listening, "", "");
                        // Clear stale audio
                        if let Some(ref p) = pipeline {
                            if let Ok(mut buf) = p.audio_buffer.lock() {
                                buf.clear();
                            }
                        }
                        if let Some(ref mut v) = vad {
                            v.reset();
                        }
                        speech_buffer.clear();
                        tracing::info!("Voice: activated → listening");
                    }
                }
                VoiceMode::Speaking => {
                    state.request_stop_tts();
                }
                _ => {}
            }
        }

        // ── Mode-specific logic ──
        match current_mode {
            VoiceMode::Idle => {
                // Check if we should enter follow mode
                if let Some(deadline) = follow_mode_deadline {
                    if Instant::now() < deadline {
                        state.set_mode(VoiceMode::Following);
                        set_voice_ui(&ui_weak, VoiceMode::Following, "", "");
                        continue;
                    } else {
                        follow_mode_deadline = None;
                    }
                }
                // Sleep longer when idle — near-zero CPU usage
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }

            VoiceMode::Listening | VoiceMode::Following => {
                let p = match pipeline.as_ref() {
                    Some(p) => p,
                    None => {
                        state.set_mode(VoiceMode::Idle);
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                };
                let v = match vad.as_mut() {
                    Some(v) => v,
                    None => {
                        state.set_mode(VoiceMode::Idle);
                        continue;
                    }
                };

                // Check follow mode timeout
                if current_mode == VoiceMode::Following {
                    if let Some(deadline) = follow_mode_deadline {
                        if Instant::now() >= deadline {
                            state.set_mode(VoiceMode::Idle);
                            follow_mode_deadline = None;
                            set_voice_ui_hidden(&ui_weak);
                            tracing::info!("Voice: follow mode expired → idle");
                            continue;
                        }
                    }
                }

                std::thread::sleep(Duration::from_millis(chunk_duration_ms));

                // Drain audio buffer
                let chunk: Vec<f32> = {
                    let mut buf = p.audio_buffer.lock().unwrap();
                    if buf.len() < p.chunk_samples {
                        continue;
                    }
                    buf.drain(..p.chunk_samples).collect()
                };

                let event = v.process_chunk(&chunk);

                match event {
                    VADEvent::Speech | VADEvent::Silence => {
                        if v.is_in_speech() {
                            speech_buffer.extend_from_slice(&chunk);
                            if current_mode == VoiceMode::Following {
                                follow_mode_deadline = Some(Instant::now() + FOLLOW_MODE_DURATION);
                            }
                        }
                    }
                    VADEvent::EndOfSpeech => {
                        speech_buffer.extend_from_slice(&chunk);

                        if speech_buffer.is_empty() {
                            continue;
                        }

                        // ── Transition to Processing ──
                        state.set_mode(VoiceMode::Processing);
                        set_voice_ui(&ui_weak, VoiceMode::Processing, "", "");

                        // Resample to 16kHz if needed
                        let pcm_16k = if p.mic_sample_rate != p.whisper_sample_rate {
                            resample(&speech_buffer, p.mic_sample_rate, p.whisper_sample_rate)
                        } else {
                            speech_buffer.clone()
                        };
                        speech_buffer.clear();

                        // ── STT ──
                        let text = match p.stt.transcribe(&pcm_16k) {
                            Ok(result) => {
                                let t = result.text.trim().to_string();
                                if t.is_empty() {
                                    let next = if follow_mode_deadline.is_some() {
                                        VoiceMode::Following
                                    } else {
                                        VoiceMode::Idle
                                    };
                                    state.set_mode(next);
                                    if next == VoiceMode::Idle {
                                        set_voice_ui_hidden(&ui_weak);
                                    } else {
                                        set_voice_ui(&ui_weak, next, "", "");
                                    }
                                    continue;
                                }
                                t
                            }
                            Err(e) => {
                                tracing::warn!("STT error: {e}");
                                state.set_mode(VoiceMode::Idle);
                                set_voice_ui_hidden(&ui_weak);
                                continue;
                            }
                        };

                        tracing::info!(text = %text, "Voice: transcribed");
                        set_voice_ui(&ui_weak, VoiceMode::Processing, &text, "");

                        // ── Send to the mind the Lens talks to (streaming) ──
                        // The person's own words; what the desk was showing goes in the turn's
                        // context. The Lens shows the turn and its reply as they arrive.
                        let turn = voice_turn(&text, &state.take_context());
                        let Some(reply) = send(turn) else {
                            tracing::warn!("Voice: the shell did not take the turn");
                            state.set_mode(VoiceMode::Idle);
                            set_voice_ui_hidden(&ui_weak);
                            continue;
                        };
                        let token_rx = &reply.tokens;
                        let mut response_text = String::new();
                        let mut sentences_spoken = 0usize;
                        let mut tts_buffer = String::new();

                        state.set_mode(VoiceMode::Speaking);

                        // The built-in's voice follows the bond; any other mind speaks in the
                        // default voice.
                        let builtin = crate::wire::harness::host()
                            .map_or(true, |h| h.active_id() == crate::wire::harness::BUILTIN_ID);
                        let params = if builtin {
                            let bond_level = bridge
                                .request_bond_level()
                                .recv_timeout(Duration::from_secs(2))
                                .unwrap_or(yantrik_companion::bond::BondLevel::Stranger);
                            voice_profile_for_bond(&bond_level).to_voice_params()
                        } else {
                            VoiceProfile { rate: 1.0, pitch: 1.0, volume: 1.0 }.to_voice_params()
                        };

                        // ── Stream response → sentence-split → TTS ──
                        let mut barged_in = false;
                        // Whether any of the reply has arrived: until it has, the mind may still
                        // be at other work in this conversation (a turn queues behind a task), and
                        // a cough must not stop that work.
                        let mut heard_reply = false;

                        loop {
                            // Check barge-in
                            if state.take_stop_tts() || check_barge_in(&p.mic_energy) {
                                barged_in = true;
                                tracing::info!("Voice: barge-in detected");
                                break;
                            }

                            match token_rx.recv_timeout(Duration::from_millis(50)) {
                                Ok(token) => {
                                    if token == "__DONE__" || token == "__REPLACE__" {
                                        if token == "__REPLACE__" {
                                            response_text.clear();
                                            continue;
                                        }
                                        break;
                                    }
                                    heard_reply = true;
                                    response_text.push_str(&token);
                                    tts_buffer.push_str(&token);

                                    // Try to speak complete sentences incrementally
                                    if let Some(pos) = find_sentence_break(&tts_buffer) {
                                        let sentence = tts_buffer[..pos].trim().to_string();
                                        tts_buffer = tts_buffer[pos..].to_string();

                                        if sentence.len() > 2 {
                                            set_voice_ui(
                                                &ui_weak,
                                                VoiceMode::Speaking,
                                                &text,
                                                &response_text,
                                            );

                                            if let Err(e) = p.tts.speak(&sentence, Some(&params)) {
                                                tracing::warn!("TTS failed: {e}");
                                            }
                                            sentences_spoken += 1;

                                            // Check barge-in after each sentence
                                            if state.take_stop_tts() || check_barge_in(&p.mic_energy)
                                            {
                                                barged_in = true;
                                                tracing::info!(
                                                    "Voice: barge-in after sentence {sentences_spoken}"
                                                );
                                                break;
                                            }
                                        }
                                    }
                                }
                                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                                Err(_) => break,
                            }
                        }

                        // Speak any remaining text in buffer
                        if !barged_in {
                            let remaining = tts_buffer.trim().to_string();
                            if remaining.len() > 2 {
                                set_voice_ui(
                                    &ui_weak,
                                    VoiceMode::Speaking,
                                    &text,
                                    &response_text,
                                );
                                if let Err(e) = p.tts.speak(&remaining, Some(&params)) {
                                    tracing::warn!("TTS failed: {e}");
                                }
                            }
                        }

                        // The person spoke over the reply: the mind stops too, not only the voice —
                        // but only while this reply is still being written. `interrupt` stops the
                        // conversation's turn in flight, so a reply that has already ended (its end
                        // is waiting unread) or never began leaves the mind alone.
                        if barged_in {
                            let pending: Vec<String> = token_rx.try_iter().collect();
                            if should_interrupt(heard_reply, &pending) {
                                reply.interrupt();
                            }
                            // Drain remaining tokens. The desktop's markers are not the reply.
                            let mut rest = pending.into_iter().chain(std::iter::from_fn(|| {
                                token_rx.recv_timeout(Duration::from_millis(10)).ok()
                            }));
                            for token in rest.by_ref() {
                                if token == "__DONE__" {
                                    break;
                                }
                                if token == "__REPLACE__" {
                                    response_text.clear();
                                    continue;
                                }
                                response_text.push_str(&token);
                            }
                        }

                        // The conversation itself is the Lens's; only the count is kept here.
                        if !response_text.is_empty() {
                            state.turn_count.fetch_add(1, Ordering::Relaxed);
                        }

                        // Update final UI
                        set_voice_ui(&ui_weak, VoiceMode::Speaking, &text, &response_text);

                        // ── Enter follow mode ──
                        follow_mode_deadline = Some(Instant::now() + FOLLOW_MODE_DURATION);
                        state.set_mode(VoiceMode::Following);
                        set_voice_ui(&ui_weak, VoiceMode::Following, &text, &response_text);
                        if let Some(ref mut v2) = vad {
                            v2.reset();
                        }

                        // Clear audio buffer
                        if let Ok(mut buf) = p.audio_buffer.lock() {
                            buf.clear();
                        }

                        if barged_in {
                            tracing::info!("Voice: barged in → listening for interruption");
                        } else {
                            tracing::info!("Voice: response complete → follow mode (15s)");
                        }
                    }
                }
            }

            VoiceMode::Processing | VoiceMode::Speaking => {
                // Handled inline above
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    tracing::info!("Voice runtime: shut down");
    Ok(())
}

// ─── Barge-in detection ─────────────────────────────────────────────────────

/// Whether talking over a reply should stop the mind as well as the voice: only once some of the
/// reply has arrived, and only while it has not ended (`pending` is what is still unread).
fn should_interrupt(heard_reply: bool, pending: &[String]) -> bool {
    heard_reply && !pending.iter().any(|t| t == "__DONE__")
}

fn check_barge_in(mic_energy: &Arc<Mutex<f32>>) -> bool {
    mic_energy
        .lock()
        .ok()
        .map(|e| *e > BARGE_IN_ENERGY_THRESHOLD)
        .unwrap_or(false)
}

// ─── Sentence splitting ─────────────────────────────────────────────────────

fn find_sentence_break(text: &str) -> Option<usize> {
    for (i, ch) in text.char_indices() {
        if matches!(ch, '.' | '!' | '?' | '\n') {
            let pos = i + ch.len_utf8();
            if text[..i].trim().len() > 2 {
                return Some(pos);
            }
        }
    }
    None
}

// ─── UI updates ─────────────────────────────────────────────────────────────

fn set_voice_ui(
    ui_weak: &slint::Weak<App>,
    mode: VoiceMode,
    transcribed: &str,
    response: &str,
) {
    let state = mode.to_ui_state();
    let transcribed = transcribed.to_string();
    let response = response.to_string();
    let weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_voice_active(state >= 0);
            ui.set_voice_state(state.max(0));
            ui.set_voice_transcribed(transcribed.into());
            ui.set_voice_response(response.into());
        }
    });
}

fn set_voice_ui_hidden(ui_weak: &slint::Weak<App>) {
    let weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_voice_active(false);
            ui.set_voice_state(0);
        }
    });
}

// ─── Audio resampling ───────────────────────────────────────────────────────

fn resample(samples: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
    if source_rate == target_rate || samples.is_empty() {
        return samples.to_vec();
    }

    use rubato::{FftFixedIn, Resampler};

    let mut resampler = FftFixedIn::<f32>::new(
        source_rate as usize,
        target_rate as usize,
        samples.len(),
        1,
        1,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A spoken turn sends what the person said, not a prompt wrapped around it: the desk's state
    /// rides beside the words, and the origin tells the mind it was spoken.
    /// Talking over a reply stops the mind only while that reply is being written: not before any
    /// of it has come (the mind may be at other work), and not once it has ended.
    #[test]
    fn talking_over_a_reply_interrupts_only_a_reply_in_flight() {
        let none: Vec<String> = vec![];
        assert!(!should_interrupt(false, &none), "nothing has arrived yet");
        assert!(should_interrupt(true, &none), "mid-reply");
        assert!(should_interrupt(true, &["more ".to_string()]));
        assert!(!should_interrupt(true, &["last.".to_string(), "__DONE__".to_string()]), "already ended");
    }

    #[test]
    fn a_voice_turn_sends_the_transcript_said_aloud() {
        let turn = voice_turn(" what time is it ", "Active app: Terminal\nCompanion status: listening\n");
        assert_eq!(turn.text, "what time is it");
        assert!(!turn.text.contains("Voice interaction") && !turn.text.contains("Active app"), "{}", turn.text);
        assert_eq!(turn.desk, "Active app: Terminal\nCompanion status: listening");
        assert_eq!(turn.origin, yantrik_harness::protocol::Origin::voice());
        assert_eq!(turn.origin.channel, "voice");
    }
}
