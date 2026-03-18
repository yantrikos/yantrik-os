//! Qwen 2.5/3.5 ChatML chat template.
//!
//! Format: `<|im_start|>role\ncontent<|im_end|>`
//! Tool calls: `<tool_call>{"name": "...", "arguments": {...}}</tool_call>`
//! Tool results: Standard `role: tool` messages.

use super::ChatTemplate;
use crate::types::{ChatMessage, ToolCall};

pub struct QwenTemplate;

impl ChatTemplate for QwenTemplate {
    fn format_tools(&self, tools: &[serde_json::Value]) -> String {
        if tools.is_empty() {
            return String::new();
        }

        let example_name = tools.first()
            .and_then(|t| t.get("function").and_then(|f| f.get("name")).and_then(|n| n.as_str()))
            .or_else(|| tools.first().and_then(|t| t.get("name")).and_then(|n| n.as_str()))
            .unwrap_or("memory_recall");

        let mut s = String::from(
            "\n\n# Tools\n\nYou may call one or more functions to assist with the user query.\n\n\
             You are provided with function signatures within <tools></tools> XML tags:\n<tools>\n"
        );
        for tool in tools {
            s.push_str(&serde_json::to_string(tool).unwrap_or_default());
            s.push('\n');
        }
        s.push_str(&format!(
            "</tools>\n\nTo call a tool, output a JSON object inside <tool_call></tool_call> XML tags. \
             The \"name\" field MUST be one of the tool names listed above. Example:\n\
             <tool_call>\n{{\"name\": \"{}\", \"arguments\": {{}}}}\n</tool_call>",
            example_name
        ));
        s
    }

    fn parse_tool_calls(&self, text: &str) -> Vec<ToolCall> {
        let mut calls = Vec::new();
        let mut search = text;

        while let Some(start) = search.find("<tool_call>") {
            let after_tag = &search[start + "<tool_call>".len()..];
            if let Some(end) = after_tag.find("</tool_call>") {
                let json_str = after_tag[..end].trim();
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let (Some(name), Some(args)) = (
                        parsed.get("name").and_then(|n| n.as_str()),
                        parsed.get("arguments"),
                    ) {
                        calls.push(ToolCall {
                            name: name.to_string(),
                            arguments: args.clone(),
                        });
                    }
                }
                search = &after_tag[end + "</tool_call>".len()..];
            } else {
                break;
            }
        }
        calls
    }

    fn extract_text_content(&self, text: &str) -> String {
        let mut result = String::new();
        let mut remaining = text;

        while let Some(start) = remaining.find("<tool_call>") {
            result.push_str(&remaining[..start]);
            let after_tag = &remaining[start + "<tool_call>".len()..];
            if let Some(end) = after_tag.find("</tool_call>") {
                remaining = &after_tag[end + "</tool_call>".len()..];
            } else {
                break;
            }
        }
        result.push_str(remaining);
        result.trim().to_string()
    }

    fn format_chat(&self, messages: &[ChatMessage]) -> Option<String> {
        let mut prompt = String::new();
        for msg in messages {
            prompt.push_str("<|im_start|>");
            prompt.push_str(&msg.role);
            prompt.push('\n');
            prompt.push_str(&msg.content);
            prompt.push_str("<|im_end|>\n");
        }
        prompt.push_str("<|im_start|>assistant\n");
        Some(prompt)
    }

    fn format_tool_result(&self, call_id: &str, tool_name: &str, result: &str) -> ChatMessage {
        ChatMessage::tool(call_id, tool_name, result)
    }

    fn disable_thinking(&self) -> bool { true }
}
