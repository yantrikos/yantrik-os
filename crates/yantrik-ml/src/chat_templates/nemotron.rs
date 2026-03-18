//! NVIDIA Nemotron-3-Nano chat template.
//!
//! Format: `<|im_start|>role\ncontent<|im_end|>` (same base as Qwen ChatML)
//! Tool definitions: XML `<tools><function><name>...</name>...</function></tools>`
//! Tool calls: `<tool_call><function=name><parameter=p>value</parameter></function></tool_call>`
//! Tool results: `role: user` with `<tool_response>` wrapper (NOT role: tool)
//!
//! Based on Ollama's `model/renderers/nemotron3nano.go` and
//! `model/parsers/nemotron3nano.go`.

use super::ChatTemplate;
use crate::types::{ChatMessage, ToolCall};

pub struct NemotronTemplate;

impl NemotronTemplate {
    /// Parse `<function=name>...<parameter=p>value</parameter>...</function>` blocks from text.
    fn parse_function_block(text: &str, calls: &mut Vec<ToolCall>) {
        let mut search = text;
        while let Some(fn_start) = search.find("<function=") {
            let after_fn = &search[fn_start + "<function=".len()..];
            let fn_name_end = after_fn.find('>').unwrap_or(0);
            let fn_name = after_fn[..fn_name_end].trim().to_string();

            let fn_body = &after_fn[fn_name_end + 1..];
            let fn_body_end = fn_body.find("</function>").unwrap_or(fn_body.len());
            let fn_body_content = &fn_body[..fn_body_end];

            // Parse parameters
            let mut args = serde_json::Map::new();
            let mut param_search = fn_body_content;
            while let Some(p_start) = param_search.find("<parameter=") {
                let after_p = &param_search[p_start + "<parameter=".len()..];
                let p_name_end = after_p.find('>').unwrap_or(0);
                let p_name = after_p[..p_name_end].trim().to_string();

                let p_body = &after_p[p_name_end + 1..];
                let p_end = p_body.find("</parameter>").unwrap_or(p_body.len());
                let p_value = p_body[..p_end].trim();

                // Try to parse as JSON value, fall back to string
                let json_val = serde_json::from_str::<serde_json::Value>(p_value)
                    .unwrap_or_else(|_| serde_json::Value::String(p_value.to_string()));
                args.insert(p_name, json_val);

                param_search = &p_body[p_end..];
            }

            calls.push(ToolCall {
                name: fn_name,
                arguments: serde_json::Value::Object(args),
            });

            search = &fn_body[fn_body_end..];
            if search.starts_with("</function>") {
                search = &search["</function>".len()..];
            }
        }
    }
}

impl ChatTemplate for NemotronTemplate {
    fn format_tools(&self, tools: &[serde_json::Value]) -> String {
        if tools.is_empty() {
            return String::new();
        }

        let mut s = String::from("\n\n# Tools\n\nYou have access to the following functions:\n\n<tools>");

        for tool in tools {
            let func = tool.get("function").unwrap_or(tool);
            let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
            let desc = func.get("description").and_then(|d| d.as_str()).unwrap_or("");

            s.push_str("\n<function>\n<name>");
            s.push_str(name);
            s.push_str("</name>");

            if !desc.is_empty() {
                s.push_str("\n<description>");
                s.push_str(desc.trim());
                s.push_str("</description>");
            }

            s.push_str("\n<parameters>");
            if let Some(params) = func.get("parameters") {
                if let Some(props) = params.get("properties").and_then(|p| p.as_object()) {
                    for (param_name, param_fields) in props {
                        s.push_str("\n<parameter>");
                        s.push_str("\n<name>");
                        s.push_str(param_name);
                        s.push_str("</name>");

                        if let Some(ptype) = param_fields.get("type").and_then(|t| t.as_str()) {
                            s.push_str("\n<type>");
                            s.push_str(ptype);
                            s.push_str("</type>");
                        }

                        if let Some(pdesc) = param_fields.get("description").and_then(|d| d.as_str()) {
                            s.push_str("\n<description>");
                            s.push_str(pdesc.trim());
                            s.push_str("</description>");
                        }

                        s.push_str("\n</parameter>");
                    }
                }

                if let Some(required) = params.get("required") {
                    if let Ok(req_json) = serde_json::to_string(required) {
                        s.push_str("\n<required>");
                        s.push_str(&req_json);
                        s.push_str("</required>");
                    }
                }
            }

            s.push_str("\n</parameters>");
            s.push_str("\n</function>");
        }

        s.push_str("\n</tools>");

        s.push_str(
            "\n\nIf you choose to call a function ONLY reply in the following format with NO suffix:\n\n\
             <tool_call>\n<function=example_function_name>\n<parameter=example_parameter_1>\n\
             value_1\n</parameter>\n<parameter=example_parameter_2>\n\
             This is the value for the second parameter\nthat can span\nmultiple lines\n\
             </parameter>\n</function>\n</tool_call>\n\n<IMPORTANT>\nReminder:\n\
             - Function calls MUST follow the specified format: an inner <function=...></function> \
             block must be nested within <tool_call></tool_call> XML tags\n\
             - Required parameters MUST be specified\n\
             - You MUST ONLY call functions listed above. NEVER invent or guess function names. \
             If no listed function fits, answer normally without a function call\n\
             - Use 'recall' to check what you know about the user before asking them questions\n\
             - Prefer using tools to gather information rather than asking the user\n\
             - You may provide optional reasoning for your function call in natural language \
             BEFORE the function call, but NOT after\n</IMPORTANT>"
        );

        s
    }

    fn parse_tool_calls(&self, text: &str) -> Vec<ToolCall> {
        let mut calls = Vec::new();
        let mut search = text;

        // Try parsing with <tool_call> wrapper first
        while let Some(tc_start) = search.find("<tool_call>") {
            let after_tc = &search[tc_start + "<tool_call>".len()..];
            let tc_end = after_tc.find("</tool_call>").unwrap_or(after_tc.len());
            let tc_block = &after_tc[..tc_end];

            Self::parse_function_block(tc_block, &mut calls);

            search = &after_tc[tc_end..];
            if search.starts_with("</tool_call>") {
                search = &search["</tool_call>".len()..];
            }
        }

        // Fallback: some models (e.g. via llama-server) omit the <tool_call> wrapper
        // and output bare <function=name>...</function> blocks
        if calls.is_empty() {
            Self::parse_function_block(text, &mut calls);
        }

        calls
    }

    fn extract_text_content(&self, text: &str) -> String {
        // Strip tool call blocks: <tool_call>...</tool_call> and bare <function=...>...</function>
        let mut result = String::new();
        let mut remaining = text;

        loop {
            // Find the next tool-related tag
            let tc_pos = remaining.find("<tool_call>");
            let fn_pos = remaining.find("<function=");

            let next = match (tc_pos, fn_pos) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };

            let Some(start) = next else {
                result.push_str(remaining);
                break;
            };

            result.push_str(&remaining[..start]);

            if remaining[start..].starts_with("<tool_call>") {
                let after = &remaining[start + "<tool_call>".len()..];
                if let Some(end) = after.find("</tool_call>") {
                    remaining = &after[end + "</tool_call>".len()..];
                } else {
                    break;
                }
            } else {
                // Bare <function=...>...</function>
                let after = &remaining[start..];
                if let Some(end) = after.find("</function>") {
                    remaining = &after[end + "</function>".len()..];
                    // Skip trailing </tool_call> if present
                    if remaining.starts_with("</tool_call>") {
                        remaining = &remaining["</tool_call>".len()..];
                    }
                } else {
                    break;
                }
            }
        }

        result.trim().to_string()
    }

    fn format_chat(&self, messages: &[ChatMessage]) -> Option<String> {
        let mut sb = String::new();

        // Extract system message
        let (system_msg, loop_msgs) = if !messages.is_empty() && messages[0].role == "system" {
            (Some(&messages[0].content), &messages[1..])
        } else {
            (None, messages.as_ref())
        };

        sb.push_str("<|im_start|>system\n");
        if let Some(sys) = system_msg {
            sb.push_str(sys);
        }
        sb.push_str("<|im_end|>\n");

        for msg in loop_msgs {
            match msg.role.as_str() {
                "tool" => {
                    // Nemotron wraps tool results in <tool_response> inside a user message
                    sb.push_str("<|im_start|>user\n<tool_response>\n");
                    sb.push_str(&msg.content);
                    sb.push_str("\n</tool_response>\n<|im_end|>\n");
                }
                _ => {
                    sb.push_str("<|im_start|>");
                    sb.push_str(&msg.role);
                    sb.push('\n');
                    sb.push_str(&msg.content);
                    sb.push_str("<|im_end|>\n");
                }
            }
        }

        sb.push_str("<|im_start|>assistant\n<think></think>");
        Some(sb)
    }

    fn format_tool_result(&self, _call_id: &str, _tool_name: &str, result: &str) -> ChatMessage {
        // Nemotron uses role: user with <tool_response> wrapper, NOT role: tool
        ChatMessage::user(format!("<tool_response>\n{}\n</tool_response>", result))
    }

    fn disable_thinking(&self) -> bool { true }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_nemotron_tool_call() {
        let text = r#"<tool_call>
<function=get_weather>
<parameter=location>
Bentonville, AR
</parameter>
</function>
</tool_call>"#;

        let calls = NemotronTemplate.parse_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments["location"], "Bentonville, AR");
    }

    #[test]
    fn test_parse_nemotron_multiple_params() {
        let text = r#"<tool_call>
<function=remember>
<parameter=topic>
location
</parameter>
<parameter=content>
User lives in Bentonville, AR
</parameter>
</function>
</tool_call>"#;

        let calls = NemotronTemplate.parse_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "remember");
        assert_eq!(calls[0].arguments["topic"], "location");
        assert_eq!(calls[0].arguments["content"], "User lives in Bentonville, AR");
    }

    #[test]
    fn test_tool_result_is_user_role() {
        let msg = NemotronTemplate.format_tool_result("call_1", "get_weather", "72F Sunny");
        assert_eq!(msg.role, "user");
        assert!(msg.content.contains("<tool_response>"));
        assert!(msg.content.contains("72F Sunny"));
    }
}
