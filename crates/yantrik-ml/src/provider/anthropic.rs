//! AnthropicBackend — direct Anthropic Messages API backend.
//!
//! Uses the Anthropic Messages API (not OpenAI-compatible) with:
//! - `x-api-key` header authentication
//! - System prompt as top-level field (not in messages array)
//! - Anthropic-specific tool use format
//! - Streaming via SSE with `event: content_block_delta`

use std::io::{BufRead, BufReader};

use anyhow::{Context, Result};

use crate::chat_template;
use crate::traits::LLMBackend;
use crate::types::{ApiToolCall, ApiToolCallFunction, ChatMessage, GenerationConfig, LLMResponse, ToolCall};

/// Anthropic Messages API backend.
///
/// Directly calls the Anthropic API without going through an OpenAI-compatible proxy.
/// Supports Claude Haiku, Sonnet, and Opus models.
pub struct AnthropicBackend {
    /// API key for authentication.
    api_key: String,
    /// Base URL (default: "https://api.anthropic.com").
    base_url: String,
    /// Model name (e.g. "claude-sonnet-4-20250514", "claude-3-5-haiku-20241022").
    model: String,
    /// Anthropic API version header.
    api_version: String,
}

impl AnthropicBackend {
    /// Create a new AnthropicBackend.
    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.anthropic.com".to_string(),
            model: model.into(),
            api_version: "2023-06-01".to_string(),
        }
    }

    /// Create with a custom base URL.
    pub fn with_base_url(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            model: model.into(),
            api_version: "2023-06-01".to_string(),
        }
    }

    fn build_agent(&self) -> ureq::Agent {
        ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(std::time::Duration::from_secs(300)))
                .build()
        )
    }

    /// Convert ChatMessage list to Anthropic Messages format.
    ///
    /// Anthropic requires:
    /// - System prompt as a separate field (not in messages)
    /// - No "system" role messages in the messages array
    /// - Tool results as `tool_result` content blocks
    fn convert_messages(&self, messages: &[ChatMessage]) -> (Option<String>, Vec<serde_json::Value>) {
        let mut system_prompt = None;
        let mut anthropic_messages = Vec::new();

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    system_prompt = Some(msg.content.clone());
                }
                "assistant" => {
                    let mut content = Vec::new();

                    if !msg.content.is_empty() {
                        content.push(serde_json::json!({
                            "type": "text",
                            "text": msg.content,
                        }));
                    }

                    // Convert tool calls to Anthropic format
                    if let Some(ref calls) = msg.tool_calls {
                        for tc in calls {
                            let input: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                                .unwrap_or(serde_json::json!({}));
                            content.push(serde_json::json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.function.name,
                                "input": input,
                            }));
                        }
                    }

                    if content.is_empty() {
                        content.push(serde_json::json!({
                            "type": "text",
                            "text": "",
                        }));
                    }

                    anthropic_messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": content,
                    }));
                }
                "tool" => {
                    // Anthropic tool results go in a "user" message with tool_result blocks
                    let tool_result = serde_json::json!({
                        "type": "tool_result",
                        "tool_use_id": msg.tool_call_id.as_deref().unwrap_or(""),
                        "content": msg.content,
                    });

                    // Check if last message is already a user message with tool_results
                    if let Some(last) = anthropic_messages.last_mut() {
                        if last["role"].as_str() == Some("user") {
                            if let Some(content) = last["content"].as_array_mut() {
                                if content.iter().any(|c| c["type"].as_str() == Some("tool_result")) {
                                    content.push(tool_result);
                                    continue;
                                }
                            }
                        }
                    }

                    anthropic_messages.push(serde_json::json!({
                        "role": "user",
                        "content": [tool_result],
                    }));
                }
                "user" => {
                    anthropic_messages.push(serde_json::json!({
                        "role": "user",
                        "content": msg.content,
                    }));
                }
                _ => {
                    // Unknown role — treat as user
                    anthropic_messages.push(serde_json::json!({
                        "role": "user",
                        "content": msg.content,
                    }));
                }
            }
        }

        (system_prompt, anthropic_messages)
    }

    /// Convert OpenAI-format tool definitions to Anthropic format.
    fn convert_tools(&self, tools: Option<&[serde_json::Value]>) -> Option<Vec<serde_json::Value>> {
        let tools = tools?;
        if tools.is_empty() { return None; }

        let anthropic_tools: Vec<serde_json::Value> = tools.iter().filter_map(|t| {
            let func = t.get("function")?;
            let name = func["name"].as_str()?;
            let description = func["description"].as_str().unwrap_or("");
            let parameters = func.get("parameters").cloned().unwrap_or(serde_json::json!({"type": "object", "properties": {}}));

            Some(serde_json::json!({
                "name": name,
                "description": description,
                "input_schema": parameters,
            }))
        }).collect();

        if anthropic_tools.is_empty() { None } else { Some(anthropic_tools) }
    }

    fn build_body(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        stream: bool,
    ) -> serde_json::Value {
        let (system_prompt, anthropic_messages) = self.convert_messages(messages);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": anthropic_messages,
            "max_tokens": config.max_tokens,
            "stream": stream,
        });

        if let Some(sys) = system_prompt {
            body["system"] = serde_json::json!(sys);
        }

        if config.temperature > 0.0 {
            body["temperature"] = serde_json::json!(config.temperature);
        }

        if let Some(p) = config.top_p {
            body["top_p"] = serde_json::json!(p);
        }

        if !config.stop.is_empty() {
            body["stop_sequences"] = serde_json::json!(config.stop);
        }

        if let Some(anthropic_tools) = self.convert_tools(tools) {
            body["tools"] = serde_json::json!(anthropic_tools);
        }

        body
    }

    fn send_request(&self, body: &serde_json::Value) -> Result<ureq::Body> {
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let body_str = serde_json::to_string(body)?;

        tracing::debug!(body_bytes = body_str.len(), url = %url, "Anthropic request");

        let agent = self.build_agent();
        let resp = agent.post(&url)
            .header("Content-Type", "application/json")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.api_version)
            .send(body_str.as_bytes())
            .context("Anthropic API request failed")?;

        Ok(resp.into_body())
    }

    /// Parse tool calls from Anthropic response content blocks.
    fn parse_tool_use_blocks(content: &serde_json::Value) -> Vec<ApiToolCall> {
        let Some(blocks) = content.as_array() else {
            return Vec::new();
        };

        blocks
            .iter()
            .filter(|b| b["type"].as_str() == Some("tool_use"))
            .filter_map(|b| {
                let id = b["id"].as_str()?.to_string();
                let name = b["name"].as_str()?.to_string();
                let input = b.get("input").cloned().unwrap_or(serde_json::json!({}));
                let arguments = serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string());
                Some(ApiToolCall {
                    id,
                    call_type: "function".to_string(),
                    function: ApiToolCallFunction { name, arguments },
                })
            })
            .collect()
    }

    /// Extract text from Anthropic content blocks.
    fn extract_text_blocks(content: &serde_json::Value) -> String {
        let Some(blocks) = content.as_array() else {
            return content.as_str().unwrap_or("").to_string();
        };

        blocks
            .iter()
            .filter(|b| b["type"].as_str() == Some("text"))
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("")
    }

    /// Probe the provider's health. Returns latency in milliseconds.
    pub fn probe_health(&self) -> Result<u64> {
        let start = std::time::Instant::now();

        // Send a minimal message to check auth + connectivity
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": "ping"}],
            "max_tokens": 1,
        });

        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let body_str = serde_json::to_string(&body)?;

        let agent = self.build_agent();
        let _resp = agent.post(&url)
            .header("Content-Type", "application/json")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.api_version)
            .send(body_str.as_bytes())
            .context("Anthropic health probe failed")?;

        Ok(start.elapsed().as_millis() as u64)
    }
}

impl LLMBackend for AnthropicBackend {
    fn chat(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        let body = self.build_body(messages, config, tools, false);
        let mut resp_body = self.send_request(&body)?;

        let json: serde_json::Value = resp_body.read_json()?;

        let text = Self::extract_text_blocks(&json["content"]);
        let api_tool_calls = Self::parse_tool_use_blocks(&json["content"]);

        let prompt_tokens = json["usage"]["input_tokens"].as_u64().unwrap_or(0) as usize;
        let completion_tokens = json["usage"]["output_tokens"].as_u64().unwrap_or(0) as usize;

        let stop_reason = match json["stop_reason"].as_str() {
            Some("end_turn") => "stop".to_string(),
            Some("max_tokens") => "length".to_string(),
            Some("tool_use") => "tool_calls".to_string(),
            Some(other) => other.to_string(),
            None => "stop".to_string(),
        };

        let tool_calls = if !api_tool_calls.is_empty() {
            api_tool_calls.iter().filter_map(ToolCall::from_api).collect()
        } else {
            chat_template::parse_tool_calls(&text)
        };

        Ok(LLMResponse {
            text,
            prompt_tokens,
            completion_tokens,
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
        let body = self.build_body(messages, config, tools, true);
        let resp_body = self.send_request(&body)?;

        let reader = BufReader::new(resp_body.into_reader());
        let mut full_text = String::new();
        let mut stop_reason = "stop".to_string();
        let mut prompt_tokens = 0usize;
        let mut completion_tokens = 0usize;

        // Track tool use blocks being streamed
        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_input = String::new();
        let mut api_tool_calls = Vec::new();

        for line_result in reader.lines() {
            let line: String = line_result.context("reading Anthropic SSE line")?;

            let data = match line.strip_prefix("data: ") {
                Some(d) => d,
                None => continue,
            };

            let event: serde_json::Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            match event["type"].as_str() {
                Some("content_block_start") => {
                    let block = &event["content_block"];
                    if block["type"].as_str() == Some("tool_use") {
                        current_tool_id = block["id"].as_str().unwrap_or("").to_string();
                        current_tool_name = block["name"].as_str().unwrap_or("").to_string();
                        current_tool_input.clear();
                    }
                }
                Some("content_block_delta") => {
                    let delta = &event["delta"];
                    match delta["type"].as_str() {
                        Some("text_delta") => {
                            if let Some(text) = delta["text"].as_str() {
                                on_token(text);
                                full_text.push_str(text);
                            }
                        }
                        Some("input_json_delta") => {
                            if let Some(partial) = delta["partial_json"].as_str() {
                                current_tool_input.push_str(partial);
                            }
                        }
                        _ => {}
                    }
                }
                Some("content_block_stop") => {
                    if !current_tool_name.is_empty() {
                        api_tool_calls.push(ApiToolCall {
                            id: current_tool_id.clone(),
                            call_type: "function".to_string(),
                            function: ApiToolCallFunction {
                                name: current_tool_name.clone(),
                                arguments: if current_tool_input.is_empty() { "{}".to_string() } else { current_tool_input.clone() },
                            },
                        });
                        current_tool_id.clear();
                        current_tool_name.clear();
                        current_tool_input.clear();
                    }
                }
                Some("message_delta") => {
                    if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                        stop_reason = match reason {
                            "end_turn" => "stop".to_string(),
                            "max_tokens" => "length".to_string(),
                            "tool_use" => "tool_calls".to_string(),
                            other => other.to_string(),
                        };
                    }
                    if let Some(output_tokens) = event["usage"]["output_tokens"].as_u64() {
                        completion_tokens = output_tokens as usize;
                    }
                }
                Some("message_start") => {
                    if let Some(input_tokens) = event["message"]["usage"]["input_tokens"].as_u64() {
                        prompt_tokens = input_tokens as usize;
                    }
                }
                _ => {}
            }
        }

        let tool_calls = if !api_tool_calls.is_empty() {
            api_tool_calls.iter().filter_map(ToolCall::from_api).collect()
        } else {
            chat_template::parse_tool_calls(&full_text)
        };

        Ok(LLMResponse {
            text: full_text,
            prompt_tokens,
            completion_tokens,
            tool_calls,
            api_tool_calls,
            stop_reason,
        })
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        // Anthropic tokenizer is roughly 4 chars per token
        Ok(text.len() / 4)
    }

    fn backend_name(&self) -> &str {
        "anthropic"
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}
