//! Google Gemma 2/3 chat template.
//!
//! Uses `<start_of_turn>role\ncontent<end_of_turn>` format.
//! Tool calling uses text-based JSON format similar to Qwen.

use super::ChatTemplate;
use crate::types::{ChatMessage, ToolCall};
use super::qwen::QwenTemplate;

pub struct GemmaTemplate;

impl ChatTemplate for GemmaTemplate {
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
        let mut prompt = String::new();
        for msg in messages {
            let role = match msg.role.as_str() {
                "system" | "user" => "user",
                "assistant" => "model",
                other => other,
            };
            prompt.push_str("<start_of_turn>");
            prompt.push_str(role);
            prompt.push('\n');
            prompt.push_str(&msg.content);
            prompt.push_str("<end_of_turn>\n");
        }
        prompt.push_str("<start_of_turn>model\n");
        Some(prompt)
    }

    fn format_tool_result(&self, call_id: &str, tool_name: &str, result: &str) -> ChatMessage {
        ChatMessage::tool(call_id, tool_name, result)
    }
}
