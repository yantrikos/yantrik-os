//! Claude Code CLI LLM backend — shells out to `claude -p` for inference.
//!
//! Implements `LLMBackend` by invoking the Claude Code CLI in print mode.
//! The conversation history is flattened into a single prompt with context,
//! and tool definitions are passed via `--system-prompt`.
//!
//! Config: `backend: "claude-cli"` in config.yaml.
//! Requires `claude` CLI installed on the system (npm i -g @anthropic-ai/claude-code).

use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::traits::LLMBackend;
use crate::types::{
    ApiToolCall, ApiToolCallFunction, ChatMessage, GenerationConfig, LLMResponse, ToolCall,
};

/// Claude Code CLI backend.
pub struct ClaudeCliLLM {
    /// Claude model to use (e.g. "sonnet", "opus", "haiku").
    model: Option<String>,
    /// Max tokens budget per session (--max-turns maps to turns, not tokens).
    max_tokens: usize,
}

impl ClaudeCliLLM {
    pub fn new(model: Option<String>, max_tokens: usize) -> Self {
        Self { model, max_tokens }
    }

    /// Format conversation messages into a single prompt string.
    /// System messages become context, tool results are inlined.
    fn format_messages(messages: &[ChatMessage]) -> (Option<String>, String) {
        let mut system_prompt = None;
        let mut conversation = String::new();

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    system_prompt = Some(msg.content.clone());
                }
                "user" => {
                    if !conversation.is_empty() {
                        conversation.push_str("\n\n");
                    }
                    conversation.push_str(&msg.content);
                }
                "assistant" => {
                    if !msg.content.is_empty() {
                        if !conversation.is_empty() {
                            conversation.push_str("\n\n");
                        }
                        conversation.push_str("[Previous assistant response]\n");
                        conversation.push_str(&msg.content);
                    }
                    // Include tool calls the assistant made
                    if let Some(ref calls) = msg.tool_calls {
                        for tc in calls {
                            conversation.push_str(&format!(
                                "\n[Called tool: {}({})]\n",
                                tc.function.name, tc.function.arguments
                            ));
                        }
                    }
                }
                "tool" => {
                    let tool_name = msg.name.as_deref().unwrap_or("unknown");
                    if !conversation.is_empty() {
                        conversation.push_str("\n\n");
                    }
                    conversation.push_str(&format!(
                        "[Tool result from {}]\n{}",
                        tool_name, msg.content
                    ));
                }
                _ => {
                    if !conversation.is_empty() {
                        conversation.push_str("\n\n");
                    }
                    conversation.push_str(&msg.content);
                }
            }
        }

        (system_prompt, conversation)
    }

    /// Build the system prompt with tool definitions appended.
    fn build_system_with_tools(
        base_system: Option<&str>,
        tools: Option<&[serde_json::Value]>,
    ) -> Option<String> {
        let has_tools = tools.map_or(false, |t| !t.is_empty());
        let has_system = base_system.map_or(false, |s| !s.is_empty());

        if !has_system && !has_tools {
            return None;
        }

        let mut prompt = String::new();

        if let Some(sys) = base_system {
            prompt.push_str(sys);
        }

        if has_tools {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str("You have access to the following tools. To call a tool, respond with a JSON block:\n");
            prompt.push_str("```tool_call\n{\"name\": \"tool_name\", \"arguments\": {\"param\": \"value\"}}\n```\n\n");
            prompt.push_str("You may call multiple tools by including multiple tool_call blocks.\n\n");
            prompt.push_str("Available tools:\n");

            for tool_def in tools.unwrap() {
                if let Some(func) = tool_def.get("function") {
                    let name = func["name"].as_str().unwrap_or("?");
                    let desc = func["description"].as_str().unwrap_or("");
                    let params = &func["parameters"];
                    prompt.push_str(&format!("\n### {}\n{}\nParameters: {}\n", name, desc, params));
                }
            }
        }

        Some(prompt)
    }

    /// Parse tool calls from Claude's text response.
    /// Looks for ```tool_call blocks or direct JSON with name/arguments.
    fn parse_tool_calls_from_text(text: &str) -> Vec<ApiToolCall> {
        let mut calls = Vec::new();
        let mut call_id = 0;

        // Find ```tool_call blocks
        let mut remaining = text;
        while let Some(start) = remaining.find("```tool_call") {
            let after_marker = &remaining[start + 12..];
            // Skip optional newline after marker
            let block_start = if after_marker.starts_with('\n') {
                &after_marker[1..]
            } else {
                after_marker
            };

            if let Some(end) = block_start.find("```") {
                let json_str = block_start[..end].trim();
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(name) = val["name"].as_str() {
                        let args = val.get("arguments").cloned().unwrap_or(serde_json::json!({}));
                        let arguments = serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string());
                        calls.push(ApiToolCall {
                            id: format!("call_{}", call_id),
                            call_type: "function".to_string(),
                            function: ApiToolCallFunction {
                                name: name.to_string(),
                                arguments,
                            },
                        });
                        call_id += 1;
                    }
                }
                remaining = &block_start[end + 3..];
            } else {
                break;
            }
        }

        calls
    }

    /// Strip tool_call blocks from the response text.
    fn strip_tool_call_blocks(text: &str) -> String {
        let mut result = String::new();
        let mut remaining = text;

        while let Some(start) = remaining.find("```tool_call") {
            result.push_str(&remaining[..start]);
            let after = &remaining[start + 12..];
            let skip = if after.starts_with('\n') { &after[1..] } else { after };
            if let Some(end) = skip.find("```") {
                remaining = &skip[end + 3..];
                // Skip trailing newline
                if remaining.starts_with('\n') {
                    remaining = &remaining[1..];
                }
            } else {
                break;
            }
        }
        result.push_str(remaining);
        result.trim().to_string()
    }

    /// Run claude CLI and get the result.
    fn run_claude(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        json_output: bool,
    ) -> Result<String> {
        let mut cmd = Command::new("claude");
        cmd.arg("-p")
            .arg(prompt)
            .env("CLAUDECODE", "") // Unset to allow nested calls
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if json_output {
            cmd.arg("--output-format").arg("json");
        }

        if let Some(sys) = system_prompt {
            cmd.arg("--system-prompt").arg(sys);
        }

        if let Some(ref model) = self.model {
            cmd.arg("--model").arg(model);
        }

        let output = cmd.output().context("Failed to run claude CLI. Is it installed?")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Claude CLI error (exit {}): {}", output.status, stderr.trim());
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(stdout)
    }
}

impl LLMBackend for ClaudeCliLLM {
    fn chat(
        &self,
        messages: &[ChatMessage],
        _config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        let (system, prompt) = Self::format_messages(messages);
        let system_with_tools = Self::build_system_with_tools(
            system.as_deref(),
            tools,
        );

        let raw = self.run_claude(
            &prompt,
            system_with_tools.as_deref(),
            true,
        )?;

        // Parse JSON output: {"type":"result","result":"...","cost_usd":...}
        let text = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) {
            json["result"].as_str().unwrap_or("").to_string()
        } else {
            // Fallback: treat as plain text
            raw.trim().to_string()
        };

        // Parse tool calls from the response text
        let api_tool_calls = Self::parse_tool_calls_from_text(&text);
        let clean_text = if !api_tool_calls.is_empty() {
            Self::strip_tool_call_blocks(&text)
        } else {
            text.clone()
        };

        let tool_calls: Vec<ToolCall> = api_tool_calls
            .iter()
            .filter_map(ToolCall::from_api)
            .collect();

        let stop_reason = if !api_tool_calls.is_empty() {
            "tool_calls".to_string()
        } else {
            "stop".to_string()
        };

        Ok(LLMResponse {
            text: clean_text,
            prompt_tokens: 0,
            completion_tokens: text.len() / 4, // rough estimate
            tool_calls,
            api_tool_calls,
            stop_reason,
        })
    }

    fn chat_streaming(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        // Claude CLI doesn't do real token-by-token streaming in -p mode.
        // Use the non-streaming chat() and emit the full response as one token.
        let response = self.chat(messages, config, tools)?;
        if !response.text.is_empty() {
            on_token(&response.text);
        }
        Ok(response)
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        // Claude uses ~4 chars per token on average
        Ok(text.len() / 4)
    }

    fn backend_name(&self) -> &str {
        "api" // Return "api" so companion.rs enables native tool calling path
    }

    fn model_id(&self) -> &str {
        self.model.as_deref().unwrap_or("claude-sonnet")
    }
}
