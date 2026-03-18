//! Llama 3.x chat template.
//!
//! Uses OpenAI-compatible function calling format through Ollama.
//! Text-based fallback uses the same `<tool_call>` JSON format as Qwen.

use super::ChatTemplate;
use crate::types::{ChatMessage, ToolCall};
use super::qwen::QwenTemplate;

pub struct LlamaTemplate;

impl ChatTemplate for LlamaTemplate {
    fn format_tools(&self, tools: &[serde_json::Value]) -> String {
        // Llama uses same JSON tool format as Qwen for text injection
        QwenTemplate.format_tools(tools)
    }

    fn parse_tool_calls(&self, text: &str) -> Vec<ToolCall> {
        QwenTemplate.parse_tool_calls(text)
    }

    fn extract_text_content(&self, text: &str) -> String {
        QwenTemplate.extract_text_content(text)
    }

    fn format_chat(&self, messages: &[ChatMessage]) -> Option<String> {
        let mut prompt = String::new();
        prompt.push_str("<|begin_of_text|>");

        for msg in messages {
            prompt.push_str("<|start_header_id|>");
            prompt.push_str(&msg.role);
            prompt.push_str("<|end_header_id|>\n\n");
            prompt.push_str(&msg.content);
            prompt.push_str("<|eot_id|>");
        }

        prompt.push_str("<|start_header_id|>assistant<|end_header_id|>\n\n");
        Some(prompt)
    }

    fn format_tool_result(&self, call_id: &str, tool_name: &str, result: &str) -> ChatMessage {
        ChatMessage::tool(call_id, tool_name, result)
    }
}
