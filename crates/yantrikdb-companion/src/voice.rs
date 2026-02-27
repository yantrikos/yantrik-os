//! Voice orchestration — VAD, STT→companion→TTS pipeline, bond-adaptive voice.
//!
//! Connects the Whisper STT engine and system TTS engine with the companion
//! pipeline, adding voice activity detection and bond-level voice adaptation.

use crate::bond::BondLevel;
use yantrikdb_ml::tts::VoiceParams;

/// Voice Activity Detection events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VADEvent {
    /// Audio chunk contains speech.
    Speech,
    /// Audio chunk is silence (but still within speech).
    Silence,
    /// Enough silence accumulated — end of speech detected.
    EndOfSpeech,
}

/// Simple energy-based Voice Activity Detection.
///
/// Detects end-of-speech by counting consecutive silent chunks.
/// No external dependencies — just RMS energy thresholding.
pub struct SimpleVAD {
    threshold: f32,
    silence_chunks_needed: usize,
    min_speech_chunks: usize,
    // State
    consecutive_silence: usize,
    speech_chunks_seen: usize,
    in_speech: bool,
}

impl SimpleVAD {
    /// Create a new VAD.
    ///
    /// - `threshold`: RMS energy threshold (0.01 is a good default)
    /// - `silence_duration_ms`: how long silence must last to trigger end-of-speech
    /// - `chunk_duration_ms`: duration of each audio chunk (typically 30-100ms)
    /// - `min_speech_ms`: minimum speech duration before we start looking for silence
    pub fn new(
        threshold: f32,
        silence_duration_ms: u64,
        chunk_duration_ms: u64,
        min_speech_ms: u64,
    ) -> Self {
        let silence_chunks_needed = (silence_duration_ms / chunk_duration_ms).max(1) as usize;
        let min_speech_chunks = (min_speech_ms / chunk_duration_ms).max(1) as usize;

        Self {
            threshold,
            silence_chunks_needed,
            min_speech_chunks,
            consecutive_silence: 0,
            speech_chunks_seen: 0,
            in_speech: false,
        }
    }

    /// Process an audio chunk and return the VAD event.
    pub fn process_chunk(&mut self, chunk: &[f32]) -> VADEvent {
        let rms = compute_rms(chunk);
        let is_speech = rms > self.threshold;

        if is_speech {
            self.consecutive_silence = 0;
            self.speech_chunks_seen += 1;
            self.in_speech = true;
            VADEvent::Speech
        } else if self.in_speech {
            self.consecutive_silence += 1;
            if self.speech_chunks_seen >= self.min_speech_chunks
                && self.consecutive_silence >= self.silence_chunks_needed
            {
                // End of speech detected — reset state
                self.reset();
                VADEvent::EndOfSpeech
            } else {
                VADEvent::Silence
            }
        } else {
            VADEvent::Silence
        }
    }

    /// Reset VAD state (call after processing a complete utterance).
    pub fn reset(&mut self) {
        self.consecutive_silence = 0;
        self.speech_chunks_seen = 0;
        self.in_speech = false;
    }

    /// Check if currently in a speech segment.
    pub fn is_in_speech(&self) -> bool {
        self.in_speech
    }
}

/// Compute RMS (root mean square) energy of an audio chunk.
fn compute_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Voice profile mapped from bond level.
///
/// Controls how the companion sounds — from formal and measured (Stranger)
/// to fast and expressive (Partner-in-Crime).
///
/// Maps to system TTS parameters: rate, pitch, volume.
#[derive(Debug, Clone)]
pub struct VoiceProfile {
    /// Speech rate multiplier: 1.0 = normal, <1.0 = slower, >1.0 = faster.
    pub rate: f32,
    /// Pitch multiplier: 1.0 = normal, <1.0 = lower, >1.0 = higher.
    pub pitch: f32,
    /// Volume multiplier: 1.0 = full.
    pub volume: f32,
}

impl VoiceProfile {
    /// Convert to TTS VoiceParams.
    pub fn to_voice_params(&self) -> VoiceParams {
        VoiceParams {
            rate: self.rate,
            pitch: self.pitch,
            volume: self.volume,
        }
    }
}

/// Get the voice profile for a given bond level.
///
/// As the bond deepens, the voice becomes faster and more expressive:
/// - Stranger: slow, measured, formal (rate 0.85, low pitch variation)
/// - Acquaintance: normal pace, warmer (rate 0.95)
/// - Friend: slightly faster, expressive (rate 1.0)
/// - Confidant: quick, very expressive (rate 1.1, higher pitch)
/// - Partner-in-Crime: fast, maximum expressiveness (rate 1.2, lively pitch)
pub fn voice_profile_for_bond(bond_level: &BondLevel) -> VoiceProfile {
    match bond_level {
        BondLevel::Stranger => VoiceProfile {
            rate: 0.85,
            pitch: 0.95,
            volume: 1.0,
        },
        BondLevel::Acquaintance => VoiceProfile {
            rate: 0.95,
            pitch: 1.0,
            volume: 1.0,
        },
        BondLevel::Friend => VoiceProfile {
            rate: 1.0,
            pitch: 1.0,
            volume: 1.0,
        },
        BondLevel::Confidant => VoiceProfile {
            rate: 1.1,
            pitch: 1.05,
            volume: 1.0,
        },
        BondLevel::PartnerInCrime => VoiceProfile {
            rate: 1.2,
            pitch: 1.1,
            volume: 1.0,
        },
    }
}

/// Result of a voice turn (STT → companion → TTS).
#[derive(Debug)]
pub struct VoiceTurnResult {
    /// What the user said (from STT).
    pub user_text: String,
    /// What the companion responded.
    pub response_text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vad_basic() {
        let mut vad = SimpleVAD::new(0.01, 500, 100, 200);

        // Silence before speech
        assert_eq!(vad.process_chunk(&[0.0; 1600]), VADEvent::Silence);

        // Speech starts
        let speech: Vec<f32> = (0..1600).map(|i| (i as f32 * 0.1).sin() * 0.1).collect();
        assert_eq!(vad.process_chunk(&speech), VADEvent::Speech);
        assert_eq!(vad.process_chunk(&speech), VADEvent::Speech);
        assert!(vad.is_in_speech());

        // Silence during speech (not enough to trigger end)
        assert_eq!(vad.process_chunk(&[0.0; 1600]), VADEvent::Silence);

        // More silence → end of speech (need 5 chunks of silence at 100ms each for 500ms)
        for _ in 0..4 {
            let event = vad.process_chunk(&[0.0; 1600]);
            if event == VADEvent::EndOfSpeech {
                return; // Success
            }
        }
        // The 5th silence chunk should trigger EndOfSpeech
        assert_eq!(vad.process_chunk(&[0.0; 1600]), VADEvent::EndOfSpeech);
    }

    #[test]
    fn test_voice_profiles() {
        let stranger = voice_profile_for_bond(&BondLevel::Stranger);
        let partner = voice_profile_for_bond(&BondLevel::PartnerInCrime);

        // Partner speaks faster (higher rate)
        assert!(partner.rate > stranger.rate);
        // Partner has livelier pitch
        assert!(partner.pitch > stranger.pitch);
    }

    #[test]
    fn test_rms() {
        assert_eq!(compute_rms(&[]), 0.0);
        assert!((compute_rms(&[1.0, -1.0, 1.0, -1.0]) - 1.0).abs() < 0.001);
        assert!((compute_rms(&[0.0, 0.0]) - 0.0).abs() < 0.001);
    }
}
