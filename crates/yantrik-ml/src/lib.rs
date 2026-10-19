//! Yantrik ML — Pluggable inference for embeddings, LLM, STT, and TTS.
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
pub mod capability;

// A Qwen3's thinking switch on a self-run server, and the all-thinking-no-answer failure
pub mod thinking;
// An error a server sends inside a streamed reply, after its 200
pub mod stream_error;

// A provider that answered and refused the request, and the parameters a model will not take.
pub mod refusal;

// Chat template formatting (always compiled — used by candle + llamacpp backends)
mod chat_template;
pub mod chat_templates;

// Compact tool-schema compiler for small models (always compiled)
pub mod compact_tools;

// Candle-specific modules (gated behind candle-llm / candle-stt features)
#[cfg(feature = "candle-llm")]
mod embedder;
#[cfg(feature = "candle-llm")]
mod model_loader;
#[cfg(feature = "candle-llm")]
mod token_stream;

// LLM backend modules
mod llm;

// The companion's model when none is chosen: its chat says how to choose one.
pub mod not_set_up;

// What each model can do and how hard it can be asked to think (the AI accounts catalogue).
pub mod model_caps;

// Provider registry (multi-provider management, key validation)
#[cfg(feature = "api-llm")]
pub mod provider;

// Hardware detection & AI recommendation engine
#[cfg(feature = "api-llm")]
pub mod hardware;

// Judges: typed answers with probabilities from System One models (Jev, Kev, ...)
pub mod judge;

// Private per-user scratch and state directories — the replacement for fixed names in /tmp
pub mod private_dir;

// Voice modules
pub mod stt;
pub mod tts;

// ── Candle exports (feature-gated) ───────────────────────────────────

#[cfg(feature = "candle-llm")]
pub use embedder::CandleEmbedder;
#[cfg(feature = "candle-llm")]
pub use model_loader::{GGUFFiles, ModelFiles};

// ── Shared type + trait exports ──────────────────────────────────────

pub use types::{ApiToolCall, ApiToolCallFunction, ChatMessage, GenerationConfig, LLMResponse, ToolCall, TranscribeResult, VoiceParams, ProviderConfigEntry};
pub use traits::{LLMBackend, STTBackend, Embedder};
pub use capability::{ModelCapabilityProfile, ModelFamily, ModelTier, ToolCallMode, SlotMode, ToolFamily};
pub use thinking::{ThoughtWithoutAnswer, THOUGHT_WITHOUT_ANSWER};
pub use stream_error::StreamError;
pub use refusal::ProviderRefused;

// ── Chat template exports ────────────────────────────────────────────

pub use chat_template::{extract_text_content, format_chat, format_tools, format_tools_compact, parse_tool_calls};
#[cfg(feature = "candle-llm")]
pub use chat_template::Qwen2Tokens;
pub use chat_templates::{ChatTemplate, template_for_family};

// ── LLM backend exports ─────────────────────────────────────────────

#[cfg(feature = "candle-llm")]
pub use llm::{CandleLLM, LLMEngine};
#[cfg(feature = "api-llm")]
pub use llm::ApiLLM;
#[cfg(feature = "claude-cli")]
pub use llm::ClaudeCliLLM;
#[cfg(feature = "llamacpp")]
pub use llm::LlamaCppLLM;
#[cfg(feature = "cognitive-kernel")]
pub use llm::CognitiveKernelLLM;
pub use llm::{CognitiveRouter, RouteDecision, PlanShape, RouteExplanation, RouteLayer};
pub use llm::{FallbackLLM, FallbackConfig};

// ── Provider registry exports ──────────────────────────────────────

#[cfg(feature = "api-llm")]
pub use provider::{
    ProviderRegistry, RegisteredProvider, ProviderId, UsageStats,
    ProviderDescriptor, ProviderKind, AuthScheme, SetupTier, KNOWN_PROVIDERS,
    GenericOpenAIBackend, AnthropicBackend, GoogleGeminiBackend,
    ProviderHealth, HealthStatus, TaskType,
    KeyValidator, KeyValidationResult, KeyValidationError,
};

// ── Hardware detection exports ───────────────────────────────────────

#[cfg(feature = "api-llm")]
pub use hardware::{
    HardwareProfile, GpuProfile, LocalRuntime, RuntimeKind,
    SetupMode, SetupRecommendation, ModeScores,
    detect_hardware, recommend_setup,
};

// ── Voice exports ────────────────────────────────────────────────────

#[cfg(feature = "candle-stt")]
pub use stt::{CandleWhisper, WhisperEngine};
pub use tts::TTSEngine;
pub use tts::PiperTTS;
#[cfg(feature = "kokoro-neural-tts")]
pub use tts::KokoroTTS;
