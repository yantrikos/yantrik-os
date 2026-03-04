//! Backend traits for pluggable ML inference.
//!
//! `LLMBackend` and `STTBackend` abstract over different inference engines
//! (candle, llama.cpp, external API) so the companion can use any backend
//! via `Box<dyn LLMBackend>`.

use anyhow::Result;

use crate::types::{ChatMessage, GenerationConfig, LLMResponse, TranscribeResult};

// ── LLM Backend ────────────────────────────────────────────────────────

/// Trait for pluggable LLM inference backends.
///
/// Implementations: `CandleLLM` (candle GGUF), `LlamaCppLLM` (llama.cpp),
/// `ApiLLM` (OpenAI-compatible HTTP API).
///
/// Uses `&mut dyn FnMut(&str)` for streaming to keep the trait object-safe.
///
/// The optional `tools` parameter passes OpenAI-format tool definitions to
/// backends that support native tool calling (e.g. `ApiLLM` with `--jinja`).
/// Local backends (candle, llama.cpp) ignore it and rely on text-injected
/// tool definitions in the system prompt.
pub trait LLMBackend: Send + Sync {
    /// Non-streaming chat completion.
    ///
    /// `tools`: Optional OpenAI-format tool definitions for native tool calling.
    fn chat(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse>;

    /// Streaming chat completion — calls `on_token` for each decoded text fragment.
    ///
    /// `tools`: Optional OpenAI-format tool definitions for native tool calling.
    fn chat_streaming(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse>;

    /// Count tokens in a text string (for prompt budget calculations).
    fn count_tokens(&self, text: &str) -> Result<usize>;

    /// Human-readable backend name (e.g. "candle", "llama.cpp", "api").
    fn backend_name(&self) -> &str;
}

// ── STT Backend ────────────────────────────────────────────────────────

/// Trait for pluggable speech-to-text backends.
///
/// Implementations: `CandleWhisper` (candle Whisper), `ApiSTT` (cloud speech API).
pub trait STTBackend: Send + Sync {
    /// Transcribe 16kHz mono f32 PCM audio to text.
    fn transcribe(&self, pcm_16khz_mono: &[f32]) -> Result<TranscribeResult>;

    /// Expected audio sample rate (always 16000 for Whisper-based backends).
    fn sample_rate(&self) -> u32;

    /// Human-readable backend name (e.g. "candle-whisper", "api").
    fn backend_name(&self) -> &str;
}
