//! Native API template for OpenAI / Anthropic models.
//!
//! These families use native API-level tool calling, so the text-based
//! format_tools / parse_tool_calls are no-ops. format_chat returns None
//! because the API handles message formatting.

use super::ChatTemplate;
use crate::types::{ChatMessage, ToolCall};

pub struct NativeTemplate;

impl ChatTemplate for NativeTemplate {
    fn format_tools(&self, _tools: &[serde_json::Value]) -> String {
        // Native API handles tool definitions — nothing to inject
        String::new()
    }

    fn parse_tool_calls(&self, _text: &str) -> Vec<ToolCall> {
        // Native API returns structured tool_calls — no text parsing needed
        Vec::new()
    }

    fn extract_text_content(&self, text: &str) -> String {
        text.trim().to_string()
    }

    fn format_chat(&self, _messages: &[ChatMessage]) -> Option<String> {
        // API handles message formatting
        None
    }

    fn format_tool_result(&self, call_id: &str, tool_name: &str, result: &str) -> ChatMessage {
        ChatMessage::tool(call_id, tool_name, result)
    }
}
