//! Model-family-aware chat templates for tool calling.
//!
//! Different LLM families use different formats for tool definitions,
//! tool calls, and tool results. This module provides a `ChatTemplate` trait
//! with per-family implementations.

pub mod qwen;
pub mod nemotron;
pub mod llama;
pub mod gemma;
pub mod phi;
pub mod native;
pub mod generic;

use crate::capability::{ModelFamily, ModelTier};
use crate::types::{ChatMessage, ToolCall};

/// Chat template trait — defines how a model family formats tool calling.
///
/// Each model family (Qwen, Nemotron, Llama, etc.) implements this trait
/// to provide the correct format for:
/// - Tool definitions in the system prompt (text-injection mode)
/// - Parsing tool calls from model output
/// - Formatting tool results back to the model
/// - Full chat prompt formatting (for embedded backends like llama.cpp)
pub trait ChatTemplate: Send + Sync {
    /// Format tool definitions for text injection into the system prompt.
    /// Called when the model does NOT use native API tool calling.
    fn format_tools(&self, tools: &[serde_json::Value]) -> String;

    /// Format tool definitions for a model of this capability tier.
    ///
    /// Tiny/Small tiers get a compact plain-text form (see
    /// [`crate::compact_tools`]) because a full JSON schema costs a 0.5B model
    /// hundreds of tokens per tool and buries the parameter names in schema
    /// noise; Medium/Large keep the full schema. Families that render tools as
    /// text override this — the default keeps the full form for families that
    /// have no compact rendering.
    fn format_tools_compact(
        &self,
        tools: &[serde_json::Value],
        _tier: ModelTier,
    ) -> String {
        self.format_tools(tools)
    }

    /// Parse tool calls from raw model text output.
    /// Used as fallback when native API tool_calls are empty.
    fn parse_tool_calls(&self, text: &str) -> Vec<ToolCall>;

    /// Extract text content, stripping tool call blocks.
    fn extract_text_content(&self, text: &str) -> String;

    /// Format a complete chat prompt from messages (for embedded backends).
    /// Returns None if the model should use the GGUF-embedded template instead.
    fn format_chat(&self, messages: &[ChatMessage]) -> Option<String>;

    /// Format a tool result message to send back to the model.
    /// Some families (Nemotron) use `role: user` with XML wrapping
    /// instead of the standard `role: tool`.
    fn format_tool_result(
        &self,
        call_id: &str,
        tool_name: &str,
        result: &str,
    ) -> ChatMessage;

    /// Whether this family needs `think: false` in the API request.
    fn disable_thinking(&self) -> bool { false }
}

/// Create a chat template for the given model family.
pub fn template_for_family(family: ModelFamily) -> Box<dyn ChatTemplate> {
    match family {
        ModelFamily::Qwen => Box::new(qwen::QwenTemplate),
        ModelFamily::Nemotron => Box::new(nemotron::NemotronTemplate),
        ModelFamily::Llama => Box::new(llama::LlamaTemplate),
        ModelFamily::Gemma => Box::new(gemma::GemmaTemplate),
        ModelFamily::Phi => Box::new(phi::PhiTemplate),
        ModelFamily::OpenAI | ModelFamily::Anthropic => Box::new(native::NativeTemplate),
        ModelFamily::Generic => Box::new(generic::GenericTemplate),
    }
}
