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

// Provider registry (multi-provider management, secret storage, key validation)
#[cfg(feature = "api-llm")]
pub mod provider;

// Hardware detection & AI recommendation engine
#[cfg(feature = "api-llm")]
pub mod hardware;

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
pub use capability::{ModelCapabilityProfile, ModelTier, ToolCallMode, SlotMode, ToolFamily};

// ── Chat template exports ────────────────────────────────────────────

pub use chat_template::{extract_text_content, format_chat, format_tools, parse_tool_calls};
#[cfg(feature = "candle-llm")]
pub use chat_template::Qwen2Tokens;

// ── LLM backend exports ─────────────────────────────────────────────

#[cfg(feature = "candle-llm")]
pub use llm::{CandleLLM, LLMEngine};
#[cfg(feature = "api-llm")]
pub use llm::ApiLLM;
#[cfg(feature = "claude-cli")]
pub use llm::ClaudeCliLLM;
#[cfg(feature = "llamacpp")]
pub use llm::LlamaCppLLM;
pub use llm::{FallbackLLM, FallbackConfig};

// ── Provider registry exports ──────────────────────────────────────

#[cfg(feature = "api-llm")]
pub use provider::{
    ProviderRegistry, RegisteredProvider, ProviderId, UsageStats,
    ProviderDescriptor, ProviderKind, AuthScheme, SetupTier, KNOWN_PROVIDERS,
    GenericOpenAIBackend, AnthropicBackend, GoogleGeminiBackend,
    ProviderHealth, HealthStatus, TaskType,
    SecretStore, AutoSecretStore, KeyringSecretStore, EncryptedFileStore, SecretRef,
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
