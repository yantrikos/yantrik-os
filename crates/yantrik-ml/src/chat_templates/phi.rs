//! Microsoft Phi-3/4 chat template.
//!
//! Uses `<|system|>`, `<|user|>`, `<|assistant|>` format.
//! Tool calling uses the same JSON `<tool_call>` format as Qwen.

use super::ChatTemplate;
use crate::types::{ChatMessage, ToolCall};
use super::qwen::QwenTemplate;

pub struct PhiTemplate;

impl ChatTemplate for PhiTemplate {
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
            let tag = match msg.role.as_str() {
                "system" => "system",
                "user" => "user",
                "assistant" => "assistant",
                "tool" => "user",
                other => other,
            };
            prompt.push_str("<|");
            prompt.push_str(tag);
            prompt.push_str("|>\n");
            prompt.push_str(&msg.content);
            prompt.push_str("<|end|>\n");
        }
        prompt.push_str("<|assistant|>\n");
        Some(prompt)
    }

    fn format_tool_result(&self, call_id: &str, tool_name: &str, result: &str) -> ChatMessage {
        ChatMessage::tool(call_id, tool_name, result)
    }
}
