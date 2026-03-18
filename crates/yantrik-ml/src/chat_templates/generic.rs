//! Generic fallback template.
//!
//! Delegates everything to QwenTemplate since ChatML is the most widely
//! supported format for unknown models.

use super::ChatTemplate;
use crate::types::{ChatMessage, ToolCall};
use super::qwen::QwenTemplate;

pub struct GenericTemplate;

impl ChatTemplate for GenericTemplate {
    fn format_tools(&self, tools: &[serde_json::Value]) -> String {
        QwenTemplate.format_tools(tools)
    }

    fn parse_tool_calls(&self, text: &str) -> Vec<ToolCall> {
        QwenTemplate.parse_tool_calls(text)
    }

    fn extract_text_content(&self, text: &str) -> String {
        QwenTemplate.extract_text_content(text)
    }

    fn format_chat(&self, messages: &[ChatMessage]) -> Option<String> {
        QwenTemplate.format_chat(messages)
    }

    fn format_tool_result(&self, call_id: &str, tool_name: &str, result: &str) -> ChatMessage {
        ChatMessage::tool(call_id, tool_name, result)
    }
}
