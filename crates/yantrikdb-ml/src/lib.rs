//! YantrikDB ML — Pluggable inference for embeddings, LLM, STT, and TTS.
//!
//! This crate provides multiple backend implementations:
//! - **Candle** (default): `CandleLLM`, `CandleWhisper`, `CandleEmbedder` — in-process via candle
//! - **llama.cpp**: `LlamaCppLLM` — hardware-accelerated GGUF inference (Vulkan/QNN/Metal/CPU)
//! - **API**: `ApiLLM` — OpenAI-compatible HTTP endpoints
//! - **TTS**: `TTSEngine` — OS-native text-to-speech (always available)
//!
//! Enable backends via feature flags:
//! - `candle-llm` + `candle-stt` (default)
//! - `llamacpp` (+ `vulkan` for GPU)
//! - `api-llm`

// Shared types and traits (always compiled, backend-agnostic)
pub mod types;
pub mod traits;

// Chat template formatting (always compiled — used by candle + llamacpp backends)
mod chat_template;

// Candle-specific modules (gated behind candle-llm / candle-stt features)
#[cfg(feature = "candle-llm")]
mod embedder;
#[cfg(feature = "candle-llm")]
mod model_loader;
#[cfg(feature = "candle-llm")]
mod token_stream;

// LLM backend modules
mod llm;

// Voice modules
pub mod stt;
pub mod tts;

// ── Candle exports (feature-gated) ───────────────────────────────────

#[cfg(feature = "candle-llm")]
pub use embedder::CandleEmbedder;
#[cfg(feature = "candle-llm")]
pub use model_loader::{GGUFFiles, ModelFiles};

// ── Shared type + trait exports ──────────────────────────────────────

pub use types::{ApiToolCall, ApiToolCallFunction, ChatMessage, GenerationConfig, LLMResponse, ToolCall, TranscribeResult, VoiceParams};
pub use traits::{LLMBackend, STTBackend};

// ── Chat template exports ────────────────────────────────────────────

pub use chat_template::{extract_text_content, format_chat, format_tools, parse_tool_calls};
#[cfg(feature = "candle-llm")]
pub use chat_template::Qwen2Tokens;

// ── LLM backend exports ─────────────────────────────────────────────

#[cfg(feature = "candle-llm")]
pub use llm::{CandleLLM, LLMEngine};
#[cfg(feature = "api-llm")]
pub use llm::ApiLLM;
#[cfg(feature = "llamacpp")]
pub use llm::LlamaCppLLM;

// ── Voice exports ────────────────────────────────────────────────────

#[cfg(feature = "candle-stt")]
pub use stt::{CandleWhisper, WhisperEngine};
pub use tts::TTSEngine;
