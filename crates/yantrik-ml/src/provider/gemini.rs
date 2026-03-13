//! GoogleGeminiBackend — Google Gemini REST API backend.
//!
//! Uses the Gemini `generateContent` endpoint (not OpenAI-compatible):
//! - API key via query parameter (`?key=<key>`)
//! - Different message format (parts/contents)
//! - Gemini-specific tool use format (functionCall / functionResponse)
//! - Streaming via SSE

use std::io::{BufRead, BufReader};

use anyhow::{Context, Result};

use crate::chat_template;
use crate::traits::LLMBackend;
use crate::types::{ApiToolCall, ApiToolCallFunction, ChatMessage, GenerationConfig, LLMResponse, ToolCall};

/// Google Gemini REST API backend.
///
/// Supports Gemini Flash, Pro, and Ultra models via the generateContent endpoint.
pub struct GoogleGeminiBackend {
    /// API key for authentication (sent as query parameter).
    api_key: String,
    /// Base URL (default: "https://generativelanguage.googleapis.com").
    base_url: String,
    /// Model name (e.g. "gemini-2.0-flash", "gemini-1.5-pro").
    model: String,
}

impl GoogleGeminiBackend {
    /// Create a new GoogleGeminiBackend.
    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            model: model.into(),
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
        }
    }

    fn build_agent(&self) -> ureq::Agent {
        ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(std::time::Duration::from_secs(300)))
                .build()
        )
    }

    /// Convert ChatMessage list to Gemini contents format.
    ///
    /// Gemini uses:
    /// - `system_instruction` for system messages (separate from contents)
    /// - `contents` array with `role: "user"` or `role: "model"`
    /// - Tool results as `functionResponse` parts
    fn convert_messages(&self, messages: &[ChatMessage]) -> (Option<serde_json::Value>, Vec<serde_json::Value>) {
        let mut system_instruction = None;
        let mut contents = Vec::new();

        for msg in messages {
            match msg.role.as_str() {
                "system" => {
                    system_instruction = Some(serde_json::json!({
                        "parts": [{"text": msg.content}]
                    }));
                }
                "assistant" => {
                    let mut parts = Vec::new();

                    if !msg.content.is_empty() {
                        parts.push(serde_json::json!({"text": msg.content}));
                    }

                    // Convert tool calls to Gemini functionCall format
                    if let Some(ref calls) = msg.tool_calls {
                        for tc in calls {
                            let args: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                                .unwrap_or(serde_json::json!({}));
                            parts.push(serde_json::json!({
                                "functionCall": {
                                    "name": tc.function.name,
                                    "args": args,
                                }
                            }));
                        }
                    }

                    if parts.is_empty() {
                        parts.push(serde_json::json!({"text": ""}));
                    }

                    contents.push(serde_json::json!({
                        "role": "model",
                        "parts": parts,
                    }));
                }
                "tool" => {
                    // Gemini tool results use functionResponse parts
                    let response_content: serde_json::Value = serde_json::from_str(&msg.content)
                        .unwrap_or(serde_json::json!({"result": msg.content}));

                    let part = serde_json::json!({
                        "functionResponse": {
                            "name": msg.name.as_deref().unwrap_or("unknown"),
                            "response": response_content,
                        }
                    });

                    // Gemini expects tool responses in a "user" turn
                    if let Some(last) = contents.last_mut() {
                        if last["role"].as_str() == Some("user") {
                            if let Some(parts) = last["parts"].as_array_mut() {
                                if parts.iter().any(|p| p.get("functionResponse").is_some()) {
                                    parts.push(part);
                                    continue;
                                }
                            }
                        }
                    }

                    contents.push(serde_json::json!({
                        "role": "user",
                        "parts": [part],
                    }));
                }
                "user" | _ => {
                    contents.push(serde_json::json!({
                        "role": "user",
                        "parts": [{"text": msg.content}],
                    }));
                }
            }
        }

        (system_instruction, contents)
    }

    /// Convert OpenAI-format tool definitions to Gemini format.
    fn convert_tools(&self, tools: Option<&[serde_json::Value]>) -> Option<Vec<serde_json::Value>> {
        let tools = tools?;
        if tools.is_empty() { return None; }

        let declarations: Vec<serde_json::Value> = tools.iter().filter_map(|t| {
            let func = t.get("function")?;
            let name = func["name"].as_str()?;
            let description = func["description"].as_str().unwrap_or("");
            let parameters = func.get("parameters").cloned()
                .unwrap_or(serde_json::json!({"type": "object", "properties": {}}));

            Some(serde_json::json!({
                "name": name,
                "description": description,
                "parameters": parameters,
            }))
        }).collect();

        if declarations.is_empty() {
            None
        } else {
            Some(vec![serde_json::json!({
                "functionDeclarations": declarations,
            })])
        }
    }

    fn build_body(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
    ) -> serde_json::Value {
        let (system_instruction, contents) = self.convert_messages(messages);

        let mut body = serde_json::json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": config.max_tokens,
                "temperature": config.temperature,
            },
        });

        if let Some(sys) = system_instruction {
            body["systemInstruction"] = sys;
        }

        if let Some(p) = config.top_p {
            body["generationConfig"]["topP"] = serde_json::json!(p);
        }

        if !config.stop.is_empty() {
            body["generationConfig"]["stopSequences"] = serde_json::json!(config.stop);
        }

        if let Some(gemini_tools) = self.convert_tools(tools) {
            body["tools"] = serde_json::json!(gemini_tools);
        }

        body
    }

    fn generate_content_url(&self) -> String {
        format!(
            "{}/v1beta/models/{}:generateContent?key={}",
            self.base_url.trim_end_matches('/'),
            self.model,
            self.api_key,
        )
    }

    fn stream_generate_content_url(&self) -> String {
        format!(
            "{}/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
            self.base_url.trim_end_matches('/'),
            self.model,
            self.api_key,
        )
    }

    fn send_request(&self, url: &str, body: &serde_json::Value) -> Result<ureq::Body> {
        let body_str = serde_json::to_string(body)?;

        tracing::debug!(body_bytes = body_str.len(), url = %url, "Gemini request");

        let agent = self.build_agent();
        let resp = agent.post(url)
            .header("Content-Type", "application/json")
            .send(body_str.as_bytes())
            .context("Gemini API request failed")?;

        Ok(resp.into_body())
    }

    /// Parse function calls from Gemini response parts.
    fn parse_function_calls(parts: &[serde_json::Value]) -> Vec<ApiToolCall> {
        parts
            .iter()
            .filter(|p| p.get("functionCall").is_some())
            .enumerate()
            .filter_map(|(idx, p)| {
                let fc = p.get("functionCall")?;
                let name = fc["name"].as_str()?.to_string();
                let args = fc.get("args").cloned().unwrap_or(serde_json::json!({}));
                let arguments = serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string());
                Some(ApiToolCall {
                    id: format!("call_{idx}"),
                    call_type: "function".to_string(),
                    function: ApiToolCallFunction { name, arguments },
                })
            })
            .collect()
    }

    /// Extract text from Gemini response parts.
    fn extract_text_parts(parts: &[serde_json::Value]) -> String {
        parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("")
    }

    /// Probe the provider's health. Returns latency in milliseconds.
    pub fn probe_health(&self) -> Result<u64> {
        let start = std::time::Instant::now();

        // List models to check auth + connectivity
        let url = format!(
            "{}/v1beta/models?key={}",
            self.base_url.trim_end_matches('/'),
            self.api_key,
        );

        let agent = self.build_agent();
        let _resp = agent.get(&url)
            .call()
            .context("Gemini health probe failed")?;

        Ok(start.elapsed().as_millis() as u64)
    }
}

impl LLMBackend for GoogleGeminiBackend {
    fn chat(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        let body = self.build_body(messages, config, tools);
        let url = self.generate_content_url();
        let mut resp_body = self.send_request(&url, &body)?;

        let json: serde_json::Value = resp_body.read_json()?;

        // Check for API errors
        if let Some(error) = json.get("error") {
            let message = error["message"].as_str().unwrap_or("Unknown Gemini error");
            anyhow::bail!("Gemini API error: {message}");
        }

        let candidate = &json["candidates"][0];
        let parts = candidate["content"]["parts"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        let text = Self::extract_text_parts(&parts);
        let api_tool_calls = Self::parse_function_calls(&parts);

        let prompt_tokens = json["usageMetadata"]["promptTokenCount"].as_u64().unwrap_or(0) as usize;
        let completion_tokens = json["usageMetadata"]["candidatesTokenCount"].as_u64().unwrap_or(0) as usize;

        let stop_reason = match candidate["finishReason"].as_str() {
            Some("STOP") => "stop".to_string(),
            Some("MAX_TOKENS") => "length".to_string(),
            Some("SAFETY") => "safety".to_string(),
            Some(other) => other.to_lowercase(),
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
        let body = self.build_body(messages, config, tools);
        let url = self.stream_generate_content_url();
        let resp_body = self.send_request(&url, &body)?;

        let reader = BufReader::new(resp_body.into_reader());
        let mut full_text = String::new();
        let mut stop_reason = "stop".to_string();
        let mut prompt_tokens = 0usize;
        let mut completion_tokens = 0usize;
        let mut all_api_tool_calls = Vec::new();

        for line_result in reader.lines() {
            let line: String = line_result.context("reading Gemini SSE line")?;

            let data = match line.strip_prefix("data: ") {
                Some(d) => d,
                None => continue,
            };

            let chunk: serde_json::Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Check for error in chunk
            if chunk.get("error").is_some() {
                let message = chunk["error"]["message"].as_str().unwrap_or("Unknown error");
                anyhow::bail!("Gemini streaming error: {message}");
            }

            let candidate = &chunk["candidates"][0];
            if let Some(parts) = candidate["content"]["parts"].as_array() {
                let text_chunk = Self::extract_text_parts(parts);
                if !text_chunk.is_empty() {
                    on_token(&text_chunk);
                    full_text.push_str(&text_chunk);
                }

                let tc = Self::parse_function_calls(parts);
                if !tc.is_empty() {
                    all_api_tool_calls.extend(tc);
                }
            }

            if let Some(reason) = candidate["finishReason"].as_str() {
                stop_reason = match reason {
                    "STOP" => "stop".to_string(),
                    "MAX_TOKENS" => "length".to_string(),
                    "SAFETY" => "safety".to_string(),
                    other => other.to_lowercase(),
                };
            }

            if let Some(usage) = chunk.get("usageMetadata") {
                if let Some(pt) = usage["promptTokenCount"].as_u64() {
                    prompt_tokens = pt as usize;
                }
                if let Some(ct) = usage["candidatesTokenCount"].as_u64() {
                    completion_tokens = ct as usize;
                }
            }
        }

        let tool_calls = if !all_api_tool_calls.is_empty() {
            all_api_tool_calls.iter().filter_map(ToolCall::from_api).collect()
        } else {
            chat_template::parse_tool_calls(&full_text)
        };

        Ok(LLMResponse {
            text: full_text,
            prompt_tokens,
            completion_tokens,
            tool_calls,
            api_tool_calls: all_api_tool_calls,
            stop_reason,
        })
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        // Approximate: 1 token ≈ 4 chars
        Ok(text.len() / 4)
    }

    fn backend_name(&self) -> &str {
        "gemini"
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}
