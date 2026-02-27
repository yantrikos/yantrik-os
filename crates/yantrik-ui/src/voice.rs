//! Voice session — manages mic capture, VAD, STT, and TTS on a dedicated thread.
//!
//! The voice thread owns the audio pipeline independently from the companion
//! worker (which owns SQLite and can't be sent across threads). Communication
//! with the companion happens through the existing CompanionBridge channel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use yantrikdb_companion::config::VoiceConfig;
use yantrikdb_companion::voice::{voice_profile_for_bond, SimpleVAD, VADEvent};
use yantrikdb_ml::{TTSEngine, WhisperEngine};

use crate::bridge::CompanionBridge;
use crate::App;

/// A running voice session — mic capture + VAD + STT + TTS on a background thread.
pub struct VoiceSession {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl VoiceSession {
    /// Start a new voice session. Spawns a thread that captures audio,
    /// detects speech, transcribes, sends to companion, and speaks the response.
    pub fn start(
        bridge: Arc<CompanionBridge>,
        ui_weak: slint::Weak<App>,
        voice_config: VoiceConfig,
    ) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();

        let handle = std::thread::spawn(move || {
            if let Err(e) = voice_loop(bridge, ui_weak, voice_config, running_clone) {
                tracing::error!("Voice session error: {e}");
            }
        });

        Self {
            running,
            handle: Some(handle),
        }
    }

    /// Stop the voice session and wait for the thread to finish.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for VoiceSession {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The voice thread's main loop.
fn voice_loop(
    bridge: Arc<CompanionBridge>,
    ui_weak: slint::Weak<App>,
    voice_config: VoiceConfig,
    running: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    // Load Whisper STT
    let stt = if let Some(ref dir) = voice_config.whisper_model_dir {
        tracing::info!(dir, "Voice: loading Whisper from directory");
        WhisperEngine::from_dir(std::path::Path::new(dir))?
    } else {
        tracing::info!(model = voice_config.whisper_model, "Voice: loading Whisper from Hub");
        WhisperEngine::from_hub(&voice_config.whisper_model)?
    };

    // Load system TTS
    let tts = TTSEngine::new()?;

    // Set up microphone capture via cpal
    let host = cpal::default_host();
    let input_device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("no microphone found"))?;

    let input_config = input_device.default_input_config()?;
    let mic_sample_rate = input_config.sample_rate().0;
    let mic_channels = input_config.channels() as usize;
    tracing::info!(sample_rate = mic_sample_rate, channels = mic_channels, "Voice: mic configured");

    // Audio buffer shared between cpal callback and this thread
    let audio_buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let buffer_clone = audio_buffer.clone();

    let stream_config = cpal::StreamConfig {
        channels: input_config.channels(),
        sample_rate: input_config.sample_rate(),
        buffer_size: cpal::BufferSize::Default,
    };

    let input_stream = input_device.build_input_stream(
        &stream_config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            // Convert to mono if needed
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
    )?;

    input_stream.play()?;
    tracing::info!("Voice: listening");

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
    while running.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(chunk_duration_ms));

        // Drain audio buffer
        let chunk: Vec<f32> = {
            let mut buf = audio_buffer.lock().unwrap();
            if buf.len() < chunk_samples {
                continue;
            }
            buf.drain(..chunk_samples).collect()
        };

        let event = vad.process_chunk(&chunk);

        match event {
            VADEvent::Speech | VADEvent::Silence => {
                if vad.is_in_speech() {
                    speech_buffer.extend_from_slice(&chunk);
                }
            }
            VADEvent::EndOfSpeech => {
                speech_buffer.extend_from_slice(&chunk);

                if speech_buffer.is_empty() {
                    continue;
                }

                // Update UI: transcribing
                set_voice_state(&ui_weak, 1, "", "");

                // Resample to 16kHz if needed
                let pcm_16k = if mic_sample_rate != whisper_sample_rate {
                    resample(&speech_buffer, mic_sample_rate, whisper_sample_rate)
                } else {
                    speech_buffer.clone()
                };
                speech_buffer.clear();

                // STT
                let text = match stt.transcribe(&pcm_16k) {
                    Ok(result) => {
                        let t = result.text.trim().to_string();
                        if t.is_empty() {
                            // Silent utterance — go back to listening
                            set_voice_state(&ui_weak, 0, "", "");
                            continue;
                        }
                        t
                    }
                    Err(e) => {
                        tracing::warn!("STT error: {e}");
                        set_voice_state(&ui_weak, 0, "", "");
                        continue;
                    }
                };

                // Update UI: show transcribed text
                set_voice_state(&ui_weak, 1, &text, "");

                // Send to companion and collect response
                let token_rx = bridge.send_message(text.clone());
                let mut response_text = String::new();

                loop {
                    match token_rx.recv() {
                        Ok(token) => {
                            if token == "__DONE__" {
                                break;
                            }
                            response_text.push_str(&token);
                        }
                        Err(_) => break,
                    }
                }

                if response_text.is_empty() {
                    set_voice_state(&ui_weak, 0, "", "");
                    continue;
                }

                // Update UI: speaking
                set_voice_state(&ui_weak, 2, &text, &response_text);

                // Get bond level for voice adaptation
                let bond_rx = bridge.request_bond_level();
                let bond_level = bond_rx
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap_or(yantrikdb_companion::bond::BondLevel::Stranger);

                let profile = voice_profile_for_bond(&bond_level);
                let params = profile.to_voice_params();

                // TTS — blocks until speech is done
                if let Err(e) = tts.speak(&response_text, Some(&params)) {
                    tracing::warn!("TTS failed: {e}");
                }

                // Back to listening (if still running)
                if running.load(Ordering::Relaxed) {
                    set_voice_state(&ui_weak, 0, "", "");
                }
            }
        }
    }

    drop(input_stream);
    tracing::info!("Voice: session ended");
    Ok(())
}

/// Update voice UI state from the voice thread.
fn set_voice_state(
    ui_weak: &slint::Weak<App>,
    state: i32,
    transcribed: &str,
    response: &str,
) {
    let transcribed = transcribed.to_string();
    let response = response.to_string();
    let weak = ui_weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_voice_state(state);
            ui.set_voice_transcribed(transcribed.into());
            ui.set_voice_response(response.into());
        }
    });
}

/// Resample audio from source_rate to target_rate using rubato.
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
