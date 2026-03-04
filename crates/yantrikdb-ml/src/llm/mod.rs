//! LLM backend modules.
//!
//! Each submodule implements `LLMBackend` for a different inference engine.

#[cfg(feature = "candle-llm")]
mod candle;
#[cfg(feature = "api-llm")]
mod api;
#[cfg(feature = "llamacpp")]
mod llamacpp;

#[cfg(feature = "candle-llm")]
pub use self::candle::CandleLLM;
#[cfg(feature = "api-llm")]
pub use self::api::ApiLLM;
#[cfg(feature = "llamacpp")]
pub use self::llamacpp::LlamaCppLLM;

/// Backward-compatible alias for `CandleLLM`.
#[cfg(feature = "candle-llm")]
pub type LLMEngine = CandleLLM;
