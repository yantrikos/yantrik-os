//! LLM backend modules.
//!
//! Each submodule implements `LLMBackend` for a different inference engine.

#[cfg(feature = "candle-llm")]
mod candle;
#[cfg(feature = "api-llm")]
mod api;
#[cfg(feature = "claude-cli")]
mod claude_cli;
#[cfg(feature = "llamacpp")]
mod llamacpp;
mod fallback;

#[cfg(feature = "candle-llm")]
pub use self::candle::CandleLLM;
#[cfg(feature = "api-llm")]
pub use self::api::ApiLLM;
#[cfg(feature = "claude-cli")]
pub use self::claude_cli::ClaudeCliLLM;
#[cfg(feature = "llamacpp")]
pub use self::llamacpp::LlamaCppLLM;
pub use self::fallback::{FallbackLLM, FallbackConfig};

/// Backward-compatible alias for `CandleLLM`.
#[cfg(feature = "candle-llm")]
pub type LLMEngine = CandleLLM;
