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

/// Strip `<think>...</think>` blocks from model output (Qwen 3.5, DeepSeek, etc.).
///
/// Used by all backends as a safety net — even when `think: false` is sent,
/// some models still emit thinking tags.
pub fn strip_think_tags(text: &str) -> String {
    let mut result = text.to_string();
    while let Some(start) = result.find("<think>") {
        if let Some(end) = result.find("</think>") {
            let tag_end = end + "</think>".len();
            result = format!("{}{}", &result[..start], result[tag_end..].trim_start());
        } else {
            // Unclosed <think> — strip from <think> to end
            result.truncate(start);
            break;
        }
    }
    result.trim().to_string()
}
