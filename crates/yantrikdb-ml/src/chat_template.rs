//! Qwen2.5 ChatML formatting and tool call parsing.
//!
//! Qwen2.5 uses the ChatML format:
//! ```text
//! <|im_start|>system
//! {content}<|im_end|>
//! <|im_start|>user
//! {content}<|im_end|>
//! <|im_start|>assistant
//! {content}<|im_end|>
//! ```

pub use crate::types::{ChatMessage, ToolCall};

/// Format chat messages into Qwen2.5 ChatML prompt.
pub fn format_chat(messages: &[ChatMessage]) -> String {
    let mut prompt = String::new();
    for msg in messages {
        prompt.push_str("<|im_start|>");
        prompt.push_str(&msg.role);
        prompt.push('\n');
        prompt.push_str(&msg.content);
        prompt.push_str("<|im_end|>\n");
    }
    // Add assistant turn start for generation
    prompt.push_str("<|im_start|>assistant\n");
    prompt
}

/// Format tool definitions for inclusion in the system prompt.
///
/// Qwen2.5 expects tool schemas in the system prompt. This formats them
/// as a standard tool definition block. Uses the first real tool name in
/// the example to prevent small models from copying a placeholder literally.
pub fn format_tools(tools: &[serde_json::Value]) -> String {
    if tools.is_empty() {
        return String::new();
    }

    // Use first real tool name so small models don't copy a placeholder literally
    let example_name = tools.first()
        .and_then(|t| t.get("function").and_then(|f| f.get("name")).and_then(|n| n.as_str()))
        .or_else(|| tools.first().and_then(|t| t.get("name")).and_then(|n| n.as_str()))
        .unwrap_or("memory_recall");

    let mut s = String::from("\n\n# Tools\n\nYou may call one or more functions to assist with the user query.\n\nYou are provided with function signatures within <tools></tools> XML tags:\n<tools>\n");
    for tool in tools {
        s.push_str(&serde_json::to_string(tool).unwrap_or_default());
        s.push('\n');
    }
    s.push_str(&format!(
        "</tools>\n\nTo call a tool, output a JSON object inside <tool_call></tool_call> XML tags. The \"name\" field MUST be one of the tool names listed above. Example:\n<tool_call>\n{{\"name\": \"{}\", \"arguments\": {{}}}}\n</tool_call>",
        example_name
    ));
    s
}

/// Parse tool calls from model output.
///
/// Looks for `<tool_call>...</tool_call>` blocks in the text.
pub fn parse_tool_calls(text: &str) -> Vec<ToolCall> {
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

/// Extract the text content from model output, excluding tool call blocks.
pub fn extract_text_content(text: &str) -> String {
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

/// Qwen2.5 special token IDs.
#[cfg(feature = "candle-llm")]
pub struct Qwen2Tokens {
    pub im_start: u32,
    pub im_end: u32,
    pub eos: u32,
}

#[cfg(feature = "candle-llm")]
impl Qwen2Tokens {
    /// Resolve special token IDs from the tokenizer.
    pub fn from_tokenizer(tokenizer: &tokenizers::Tokenizer) -> Self {
        Self {
            im_start: tokenizer.token_to_id("<|im_start|>").unwrap_or(151644),
            im_end: tokenizer.token_to_id("<|im_end|>").unwrap_or(151645),
            eos: tokenizer.token_to_id("<|endoftext|>").unwrap_or(151643),
        }
    }

    /// Check if a token is a stop token (should end generation).
    pub fn is_stop(&self, token: u32) -> bool {
        token == self.im_end || token == self.eos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_chat() {
        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Hello!"),
        ];
        let prompt = format_chat(&messages);
        assert!(prompt.contains("<|im_start|>system\nYou are helpful.<|im_end|>"));
        assert!(prompt.contains("<|im_start|>user\nHello!<|im_end|>"));
        assert!(prompt.ends_with("<|im_start|>assistant\n"));
    }

    #[test]
    fn test_parse_tool_calls() {
        let text = r#"I'll remember that for you.
<tool_call>
{"name": "memory_record", "arguments": {"text": "User likes chess", "importance": 0.7}}
</tool_call>"#;

        let calls = parse_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "memory_record");
        assert_eq!(calls[0].arguments["text"], "User likes chess");
    }

    #[test]
    fn test_parse_multiple_tool_calls() {
        let text = r#"Let me do both things.
<tool_call>
{"name": "recall", "arguments": {"query": "hobbies"}}
</tool_call>
<tool_call>
{"name": "record", "arguments": {"text": "likes hiking"}}
</tool_call>"#;

        let calls = parse_tool_calls(text);
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn test_extract_text_content() {
        let text = "Hello! <tool_call>{\"name\": \"x\", \"arguments\": {}}</tool_call> Done.";
        let content = extract_text_content(text);
        assert_eq!(content, "Hello!  Done.");
    }

    #[test]
    fn test_no_tool_calls() {
        let text = "Just a normal response with no tools.";
        let calls = parse_tool_calls(text);
        assert!(calls.is_empty());
        assert_eq!(extract_text_content(text), text);
    }
}
