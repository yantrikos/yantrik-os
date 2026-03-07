//! STT backend modules.
//!
//! Each submodule implements `STTBackend` for a different inference engine.

#[cfg(feature = "candle-stt")]
mod candle_whisper;

#[cfg(feature = "candle-stt")]
pub use self::candle_whisper::CandleWhisper;

/// Backward-compatible alias for `CandleWhisper`.
#[cfg(feature = "candle-stt")]
pub type WhisperEngine = CandleWhisper;

// Re-export TranscribeResult for backward compatibility
pub use crate::types::TranscribeResult;
