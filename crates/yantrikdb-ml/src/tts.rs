//! System TTS engine — text-to-speech via OS native APIs.
//!
//! Uses the `tts` crate which wraps:
//! - macOS: AVSpeechSynthesizer
//! - Linux: speech-dispatcher
//! - Windows: SAPI
//!
//! This is a synchronous, blocking TTS that speaks through the system speakers.
//! For neural TTS (Piper), see the `piper` feature (requires cmake + espeak-ng).
//!
//! ```rust,ignore
//! let engine = TTSEngine::new()?;
//! engine.speak("Hello world!", None)?;
//! ```

use std::sync::Mutex;

use anyhow::Result;

pub use crate::types::VoiceParams;

/// System TTS engine.
///
/// Thread-safe via internal Mutex (same pattern as LLMEngine).
/// Speaks through the OS default audio output — no PCM buffers needed.
pub struct TTSEngine {
    inner: Mutex<tts::Tts>,
}

// Safety: Mutex serializes all access.
unsafe impl Send for TTSEngine {}
unsafe impl Sync for TTSEngine {}

impl TTSEngine {
    /// Create a new TTS engine using the OS default voice.
    pub fn new() -> Result<Self> {
        let tts = tts::Tts::default().map_err(|e| anyhow::anyhow!("TTS init failed: {e}"))?;
        tracing::info!("TTSEngine initialized (system TTS)");
        Ok(Self {
            inner: Mutex::new(tts),
        })
    }

    /// Speak text aloud through system speakers (blocking until complete).
    ///
    /// If `params` is provided, adjusts rate/pitch/volume before speaking.
    pub fn speak(&self, text: &str, params: Option<&VoiceParams>) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }

        let mut tts = self
            .inner
            .lock()
            .map_err(|e| anyhow::anyhow!("lock: {e}"))?;

        // Apply voice parameters
        if let Some(p) = params {
            // Map rate: tts crate uses absolute rate where normal differs by platform.
            // We'll get the normal rate and scale it.
            if let Ok(normal) = tts.get_rate() {
                let _ = tts.set_rate(normal * p.rate);
            }
            if let Ok(normal) = tts.get_pitch() {
                let _ = tts.set_pitch(normal * p.pitch);
            }
            if let Ok(normal) = tts.get_volume() {
                let _ = tts.set_volume(normal * p.volume);
            }
        }

        // Speak (blocking — waits until utterance is complete)
        tts.speak(text, false)
            .map_err(|e| anyhow::anyhow!("TTS speak failed: {e}"))?;

        // Wait for speech to finish
        // The tts crate's speak() with interrupt=false queues the utterance.
        // We need to poll until it's done.
        loop {
            match tts.is_speaking() {
                Ok(true) => std::thread::sleep(std::time::Duration::from_millis(50)),
                _ => break,
            }
        }

        Ok(())
    }

    /// Check if the TTS engine is currently speaking.
    pub fn is_speaking(&self) -> bool {
        self.inner
            .lock()
            .ok()
            .and_then(|tts| tts.is_speaking().ok())
            .unwrap_or(false)
    }
}
