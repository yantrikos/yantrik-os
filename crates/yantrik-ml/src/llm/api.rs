//! API-based LLM backend — sends requests to OpenAI-compatible HTTP endpoints.
//!
//! Supports both OpenAI `/v1/chat/completions` and Ollama native `/api/chat`.
//! Auto-detects Ollama by URL pattern and uses native endpoint for proper
//! thinking mode control.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read as _};

use anyhow::{Context, Result};

use crate::chat_template;
use crate::chat_templates;
use crate::capability::ModelFamily;
use crate::llm::strip_think_tags;
use crate::traits::LLMBackend;
use crate::types::{ApiToolCall, ApiToolCallFunction, ChatMessage, GenerationConfig, LLMResponse, ToolCall};

/// OpenAI-compatible API LLM backend with Ollama native support.
pub struct ApiLLM {
    base_url: String,
    api_key: Option<String>,
    model: String,
    /// True when talking to Ollama — use native /api/chat for think control.
    is_ollama: bool,
    /// Model family for family-aware template selection.
    family: ModelFamily,
}

impl ApiLLM {
    pub fn new(base_url: impl Into<String>, api_key: Option<String>, model: impl Into<String>) -> Self {
        let base_url: String = base_url.into();
        let model: String = model.into();
        let is_ollama = base_url.contains(":11434");
        let family = ModelFamily::from_model_name(&model);
        Self {
            base_url,
            api_key,
            is_ollama,
            family,
            model,
        }
    }

    /// Get the model name/identifier for capability profiling.
    pub fn model_name(&self) -> &str {
        &self.model
    }

    /// Get the family-aware chat template for this model.
    fn template(&self) -> Box<dyn chat_templates::ChatTemplate> {
        chat_templates::template_for_family(self.family)
    }

    /// Serialize a ChatMessage to JSON, including tool_calls/tool_call_id/name when present.
    fn serialize_message(m: &ChatMessage, ollama_compat: bool) -> serde_json::Value {
        let mut msg = serde_json::json!({ "role": m.role });

        // Content can be null for assistant messages that only have tool_calls
        if m.role == "assistant" && m.content.is_empty() && m.tool_calls.is_some() {
            msg["content"] = serde_json::Value::Null;
        } else {
            msg["content"] = serde_json::json!(m.content);
        }

        if let Some(ref calls) = m.tool_calls {
            if ollama_compat {
                // Ollama expects arguments as JSON objects, not strings
                let fixed: Vec<serde_json::Value> = calls.iter().map(|tc| {
                    let args = serde_json::from_str::<serde_json::Value>(&tc.function.arguments)
                        .unwrap_or(serde_json::json!({}));
                    serde_json::json!({
                        "id": tc.id,
                        "function": { "name": tc.function.name, "arguments": args }
                    })
                }).collect();
                msg["tool_calls"] = serde_json::json!(fixed);
            } else {
                msg["tool_calls"] = serde_json::json!(calls);
            }
        }
        if let Some(ref id) = m.tool_call_id {
            msg["tool_call_id"] = serde_json::json!(id);
        }
        if let Some(ref name) = m.name {
            msg["name"] = serde_json::json!(name);
        }

        msg
    }

    // ── Ollama native API (/api/chat) ──

    /// Build request body for Ollama native /api/chat endpoint.
    fn build_ollama_body(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        stream: bool,
    ) -> serde_json::Value {
        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| Self::serialize_message(m, true))
            .collect();

        let mut options = serde_json::json!({
            "temperature": config.temperature,
            "num_predict": config.max_tokens,
            "num_ctx": config.max_context.unwrap_or(4096),
        });

        if let Some(p) = config.top_p {
            options["top_p"] = serde_json::json!(p);
        }
        if config.repeat_penalty != 1.0 {
            options["repeat_penalty"] = serde_json::json!(config.repeat_penalty);
        }
        if !config.stop.is_empty() {
            options["stop"] = serde_json::json!(config.stop);
        }

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": msgs,
            "stream": stream,
            "keep_alive": -1,
            "options": options,
        });

        // Disable thinking mode for families that need it (Qwen, Nemotron, etc.)
        if self.template().disable_thinking() {
            body["think"] = serde_json::json!(false);
        }

        if let Some(tools) = tools {
            if !tools.is_empty() {
                body["tools"] = serde_json::json!(tools);
            }
        }

        body
    }

    /// Get Ollama base URL (strip /v1 suffix if present).
    fn ollama_base_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        base.trim_end_matches("/v1").to_string()
    }

    fn send_ollama_request(&self, body: &serde_json::Value) -> Result<ureq::Body> {
        let url = format!("{}/api/chat", self.ollama_base_url());
        let body_str = serde_json::to_string(body)?;

        let tool_count = body.get("tools").and_then(|t| t.as_array()).map(|a| a.len()).unwrap_or(0);
        tracing::debug!(body_bytes = body_str.len(), tools = tool_count, url = %url, "Ollama request");

        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(std::time::Duration::from_secs(300)))
                .build()
        );

        let agent_no_err = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(std::time::Duration::from_secs(300)))
                .http_status_as_error(false)
                .build()
        );

        let resp = agent_no_err.post(&url)
            .header("Content-Type", "application/json")
            .send(body_str.as_bytes())
            .map_err(|e| {
                tracing::error!(error = %e, body_bytes = body_str.len(), tools = tool_count, "Ollama API request failed");
                e
            })
            .context("Ollama API request failed")?;

        let status = resp.status();
        if status != 200 {
            let err_body = resp.into_body().read_to_string().unwrap_or_default();
            let preview = &err_body[..err_body.len().min(1000)];
            tracing::error!(
                status = status.as_u16(),
                body_bytes = body_str.len(),
                tools = tool_count,
                error_response = %preview,
                "Ollama API returned error"
            );
            anyhow::bail!("Ollama API request failed: http status: {}", status.as_u16());
        }

        Ok(resp.into_body())
    }

    /// Non-streaming Ollama chat.
    fn ollama_chat(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        // Text-injection: for non-Ollama backends OR when Ollama's native template
        // can't handle the tool count (e.g. Nemotron with >12 tools).
        let tool_count = tools.map(|t| t.len()).unwrap_or(0);
        let force_text = self.use_text_injection_tools()
            || self.needs_text_injection_for_tool_count(tool_count);
        let patched_messages;
        let (final_messages, final_tools) = if force_text {
            if let Some(tools) = tools.filter(|t| !t.is_empty()) {
                if self.needs_text_injection_for_tool_count(tool_count) {
                    tracing::info!(
                        tool_count, max = Self::MAX_OLLAMA_NEMOTRON_NATIVE_TOOLS,
                        "Nemotron: too many tools for native, falling back to text-injection"
                    );
                }
                patched_messages = self.inject_tools_into_messages(messages, tools);
                (patched_messages.as_slice(), None)
            } else {
                (messages, None)
            }
        } else {
            (messages, tools)
        };
        let body = self.build_ollama_body(final_messages, config, final_tools, false);
        let mut resp_body = self.send_ollama_request(&body)?;

        let json: serde_json::Value = resp_body.read_json()?;

        let message = &json["message"];
        let raw_text = message["content"].as_str().unwrap_or("").to_string();
        let text = strip_think_tags(&raw_text);

        let eval_count = json["eval_count"].as_u64().unwrap_or(0) as usize;
        let prompt_eval_count = json["prompt_eval_count"].as_u64().unwrap_or(0) as usize;

        // Parse tool calls (Ollama uses same format as OpenAI)
        let api_tool_calls = Self::parse_api_tool_calls(message);
        let tool_calls = if !api_tool_calls.is_empty() {
            api_tool_calls.iter().filter_map(ToolCall::from_api).collect()
        } else {
            self.template().parse_tool_calls(&text)
        };

        Ok(LLMResponse {
            text,
            prompt_tokens: prompt_eval_count,
            completion_tokens: eval_count,
            tool_calls,
            api_tool_calls,
            stop_reason: if json["done"].as_bool() == Some(true) { "stop".to_string() } else { "length".to_string() },
        })
    }

    /// Streaming Ollama chat (newline-delimited JSON, not SSE).
    fn ollama_chat_streaming(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        // Text-injection: for non-Ollama backends OR when Ollama's native template
        // can't handle the tool count (e.g. Nemotron with >12 tools).
        let tool_count = tools.map(|t| t.len()).unwrap_or(0);
        let force_text = self.use_text_injection_tools()
            || self.needs_text_injection_for_tool_count(tool_count);
        let patched_messages;
        let (final_messages, final_tools) = if force_text {
            if let Some(tools) = tools.filter(|t| !t.is_empty()) {
                if self.needs_text_injection_for_tool_count(tool_count) {
                    tracing::info!(
                        tool_count, max = Self::MAX_OLLAMA_NEMOTRON_NATIVE_TOOLS,
                        "Nemotron: too many tools for native, falling back to text-injection"
                    );
                }
                patched_messages = self.inject_tools_into_messages(messages, tools);
                (patched_messages.as_slice(), None)
            } else {
                (messages, None)
            }
        } else {
            (messages, tools)
        };
        let body = self.build_ollama_body(final_messages, config, final_tools, true);
        let resp_body = self.send_ollama_request(&body)?;

        let reader = BufReader::new(resp_body.into_reader());

        let mut full_text = String::new();
        let mut stop_reason = "stop".to_string();
        let mut eval_count = 0usize;
        let mut api_tool_calls = Vec::new();
        let mut in_think = false;
        let mut think_buf = String::new();

        for line_result in reader.lines() {
            let line: String = line_result.context("reading Ollama stream line")?;
            if line.trim().is_empty() {
                continue;
            }

            let chunk: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Extract content token with think-tag filtering
            if let Some(content) = chunk["message"]["content"].as_str() {
                if !content.is_empty() {
                    if in_think {
                        think_buf.push_str(content);
                        if think_buf.contains("</think>") {
                            // Thinking block complete — discard it, emit any text after </think>
                            if let Some(after) = think_buf.split("</think>").last() {
                                let after = after.trim_start();
                                if !after.is_empty() {
                                    on_token(after);
                                    full_text.push_str(after);
                                }
                            }
                            in_think = false;
                            think_buf.clear();
                        }
                    } else if content.contains("<think>") {
                        // Start of think block — buffer any partial content after <think>
                        if let Some(before) = content.split("<think>").next() {
                            if !before.is_empty() {
                                on_token(before);
                                full_text.push_str(before);
                            }
                        }
                        let after_tag = content.splitn(2, "<think>").nth(1).unwrap_or("");
                        think_buf.push_str(after_tag);
                        if think_buf.contains("</think>") {
                            if let Some(after) = think_buf.split("</think>").last() {
                                let after = after.trim_start();
                                if !after.is_empty() {
                                    on_token(after);
                                    full_text.push_str(after);
                                }
                            }
                            think_buf.clear();
                        } else {
                            in_think = true;
                        }
                    } else {
                        on_token(content);
                        full_text.push_str(content);
                    }
                }
            }

            // Tool calls — Ollama sends them in a done:false chunk, not the final one
            let tc = Self::parse_api_tool_calls(&chunk["message"]);
            if !tc.is_empty() {
                api_tool_calls = tc;
            }

            if chunk["done"].as_bool() == Some(true) {
                eval_count = chunk["eval_count"].as_u64().unwrap_or(0) as usize;
                if chunk["done_reason"].as_str() == Some("length") {
                    stop_reason = "length".to_string();
                }
            }
        }

        // Final safety: strip any remaining think tags from assembled text
        let full_text = strip_think_tags(&full_text);

        let tool_calls = if !api_tool_calls.is_empty() {
            api_tool_calls.iter().filter_map(ToolCall::from_api).collect()
        } else {
            self.template().parse_tool_calls(&full_text)
        };

        Ok(LLMResponse {
            text: full_text,
            prompt_tokens: 0,
            completion_tokens: eval_count,
            tool_calls,
            api_tool_calls,
            stop_reason,
        })
    }

    // ── OpenAI-compatible API (/v1/chat/completions) ──

    /// Whether this backend should use text-injection for tools (vs native API tools).
    ///
    /// Text-injection injects tool definitions into the system prompt and parses
    /// tool calls from the model's text output using family-specific templates.
    ///
    /// Used when:
    /// - Nemotron models (always): Ollama's native tool template for nemotron produces
    ///   XML parsing errors with many tools. Our text-injection parser is more robust.
    /// - Non-Ollama local models (llama-server, etc.): typically lack proper jinja
    ///   templates for native tool calling.
    /// - NOT used for cloud APIs (OpenAI, Anthropic) which handle tools natively.
    /// Maximum tools Ollama's nemotron template can handle natively before
    /// its Go XML renderer hits "XML syntax error: unexpected EOF".
    /// Empirically safe at 12; above this we fall back to text-injection.
    const MAX_OLLAMA_NEMOTRON_NATIVE_TOOLS: usize = 12;

    fn use_text_injection_tools(&self) -> bool {
        // Cloud APIs handle tools natively
        if matches!(self.family, ModelFamily::OpenAI | ModelFamily::Anthropic) {
            return false;
        }
        // Other local models on non-Ollama servers need text-injection
        !self.is_ollama
    }

    /// Whether to fall back to text-injection for this specific call due to
    /// Ollama template limitations with many tools.
    fn needs_text_injection_for_tool_count(&self, tool_count: usize) -> bool {
        self.is_ollama
            && matches!(self.family, ModelFamily::Nemotron)
            && tool_count > Self::MAX_OLLAMA_NEMOTRON_NATIVE_TOOLS
    }

    /// Inject tool definitions into the system message using the family-specific template.
    fn inject_tools_into_messages(
        &self,
        messages: &[ChatMessage],
        tools: &[serde_json::Value],
    ) -> Vec<ChatMessage> {
        let template = self.template();
        let tool_block = template.format_tools(tools);
        if tool_block.is_empty() {
            return messages.to_vec();
        }

        let mut patched = messages.to_vec();
        if !patched.is_empty() && patched[0].role == "system" {
            patched[0] = ChatMessage::system(
                format!("{}{}", patched[0].content, tool_block),
            );
        } else {
            patched.insert(0, ChatMessage::system(tool_block));
        }
        patched
    }

    fn build_openai_body(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        stream: bool,
    ) -> serde_json::Value {
        // For local models on non-Ollama servers (e.g. llama-server), inject tools
        // into the system prompt instead of passing them via the API tools field.
        let use_text_injection = self.use_text_injection_tools();
        let patched_messages;
        let (final_messages, final_tools) = if use_text_injection {
            if let Some(tools) = tools.filter(|t| !t.is_empty()) {
                patched_messages = self.inject_tools_into_messages(messages, tools);
                tracing::debug!(
                    tool_count = tools.len(),
                    "Text-injecting tools into system prompt for non-Ollama endpoint"
                );
                (patched_messages.as_slice(), None)
            } else {
                (messages, None)
            }
        } else {
            (messages, tools)
        };

        let msgs: Vec<serde_json::Value> = final_messages
            .iter()
            .map(|m| Self::serialize_message(m, false))
            .collect();

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": msgs,
            "max_tokens": config.max_tokens,
            "temperature": config.temperature,
            "stream": stream,
        });

        if let Some(p) = config.top_p {
            body["top_p"] = serde_json::json!(p);
        }

        if !config.stop.is_empty() {
            body["stop"] = serde_json::json!(config.stop);
        }

        if config.repeat_penalty != 1.0 {
            body["frequency_penalty"] = serde_json::json!((config.repeat_penalty - 1.0).clamp(-2.0, 2.0));
        }

        // Disable thinking mode for families that need it (Qwen, Nemotron, etc.)
        if self.template().disable_thinking() {
            body["think"] = serde_json::json!(false);
        }

        if let Some(tools) = final_tools {
            if !tools.is_empty() {
                body["tools"] = serde_json::json!(tools);
            }
        }

        body
    }

    fn openai_endpoint_url(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    fn send_openai_request(&self, body: &serde_json::Value) -> Result<ureq::Body> {
        let url = self.openai_endpoint_url();
        let body_str = serde_json::to_string(body)?;

        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(std::time::Duration::from_secs(300)))
                .http_status_as_error(false)
                .build()
        );

        let mut req = agent.post(&url)
            .header("Content-Type", "application/json");

        if let Some(ref key) = self.api_key {
            req = req.header("Authorization", &format!("Bearer {key}"));
        }

        let resp = req
            .send(body_str.as_bytes())
            .context("API request failed")?;

        let status = resp.status();
        if status != 200 {
            let err_body = resp.into_body().read_to_string().unwrap_or_default();
            let preview = &err_body[..err_body.len().min(1000)];
            tracing::error!(
                status = status.as_u16(),
                body_bytes = body_str.len(),
                error_response = %preview,
                "OpenAI-compatible API returned error"
            );
            anyhow::bail!("API request failed: http status: {}", status.as_u16());
        }

        Ok(resp.into_body())
    }

    /// Parse native tool_calls from the response JSON.
    fn parse_api_tool_calls(message: &serde_json::Value) -> Vec<ApiToolCall> {
        let Some(calls) = message["tool_calls"].as_array() else {
            return Vec::new();
        };

        calls
            .iter()
            .filter_map(|tc| {
                let id = tc["id"].as_str()?.to_string();
                let call_type = tc["type"].as_str().unwrap_or("function").to_string();
                let name = tc["function"]["name"].as_str()?.to_string();
                // Ollama native returns arguments as JSON object; OpenAI returns as string
                let arguments = match &tc["function"]["arguments"] {
                    v if v.is_string() => v.as_str().unwrap().to_string(),
                    v if v.is_object() || v.is_array() => serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()),
                    _ => "{}".to_string(),
                };
                Some(ApiToolCall {
                    id,
                    call_type,
                    function: ApiToolCallFunction { name, arguments },
                })
            })
            .collect()
    }
}

impl LLMBackend for ApiLLM {
    fn chat(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        // Use Ollama native API for proper think control
        if self.is_ollama {
            return self.ollama_chat(messages, config, tools);
        }

        let body = self.build_openai_body(messages, config, tools, false);
        let mut resp_body = self.send_openai_request(&body)?;

        let json: serde_json::Value = resp_body.read_json()?;

        let message = &json["choices"][0]["message"];

        let text = strip_think_tags(
            message["content"].as_str().unwrap_or("")
        );

        let prompt_tokens = json["usage"]["prompt_tokens"]
            .as_u64()
            .unwrap_or(0) as usize;
        let completion_tokens = json["usage"]["completion_tokens"]
            .as_u64()
            .unwrap_or(0) as usize;

        let stop_reason = json["choices"][0]["finish_reason"]
            .as_str()
            .unwrap_or("stop")
            .to_string();

        let api_tool_calls = Self::parse_api_tool_calls(message);

        let tool_calls = if !api_tool_calls.is_empty() {
            api_tool_calls
                .iter()
                .filter_map(ToolCall::from_api)
                .collect()
        } else {
            self.template().parse_tool_calls(&text)
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
        // Use Ollama native streaming for proper think control
        if self.is_ollama {
            return self.ollama_chat_streaming(messages, config, tools, on_token);
        }

        let body = self.build_openai_body(messages, config, tools, true);
        let resp_body = self.send_openai_request(&body)?;

        let reader = BufReader::new(resp_body.into_reader());

        let mut full_text = String::new();
        let mut stop_reason = "stop".to_string();
        let mut in_think = false;
        let mut think_buf = String::new();

        let mut tc_ids: HashMap<usize, String> = HashMap::new();
        let mut tc_names: HashMap<usize, String> = HashMap::new();
        let mut tc_args: HashMap<usize, String> = HashMap::new();

        for line_result in reader.lines() {
            let line: String = line_result.context("reading SSE line")?;

            let data = if let Some(stripped) = line.strip_prefix("data: ") {
                stripped
            } else {
                continue;
            };

            if data == "[DONE]" {
                break;
            }

            let chunk: serde_json::Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let delta = &chunk["choices"][0]["delta"];

            if let Some(content) = delta["content"].as_str() {
                if in_think {
                    think_buf.push_str(content);
                    if think_buf.contains("</think>") {
                        if let Some(after) = think_buf.split("</think>").last() {
                            let after = after.trim_start();
                            if !after.is_empty() {
                                on_token(after);
                                full_text.push_str(after);
                            }
                        }
                        in_think = false;
                        think_buf.clear();
                    }
                } else if content.contains("<think>") {
                    if let Some(before) = content.split("<think>").next() {
                        if !before.is_empty() {
                            on_token(before);
                            full_text.push_str(before);
                        }
                    }
                    let after_tag = content.splitn(2, "<think>").nth(1).unwrap_or("");
                    think_buf.push_str(after_tag);
                    if think_buf.contains("</think>") {
                        if let Some(after) = think_buf.split("</think>").last() {
                            let after = after.trim_start();
                            if !after.is_empty() {
                                on_token(after);
                                full_text.push_str(after);
                            }
                        }
                        think_buf.clear();
                    } else {
                        in_think = true;
                    }
                } else {
                    on_token(content);
                    full_text.push_str(content);
                }
            }

            if let Some(tool_calls) = delta["tool_calls"].as_array() {
                for tc_delta in tool_calls {
                    let idx = tc_delta["index"].as_u64().unwrap_or(0) as usize;
                    if let Some(id) = tc_delta["id"].as_str() {
                        tc_ids.insert(idx, id.to_string());
                    }
                    if let Some(name) = tc_delta["function"]["name"].as_str() {
                        tc_names.entry(idx).or_default().push_str(name);
                    }
                    if let Some(args) = tc_delta["function"]["arguments"].as_str() {
                        tc_args.entry(idx).or_default().push_str(args);
                    }
                }
            }

            if let Some(reason) = chunk["choices"][0]["finish_reason"].as_str() {
                stop_reason = reason.to_string();
            }
        }

        let mut api_tool_calls = Vec::new();
        let max_idx = tc_names.keys().copied().max().unwrap_or(0);
        for idx in 0..=max_idx {
            if let Some(name) = tc_names.get(&idx) {
                let id = tc_ids.get(&idx).cloned().unwrap_or_else(|| format!("call_{idx}"));
                let arguments = tc_args.get(&idx).cloned().unwrap_or_else(|| "{}".to_string());
                api_tool_calls.push(ApiToolCall {
                    id,
                    call_type: "function".to_string(),
                    function: ApiToolCallFunction {
                        name: name.clone(),
                        arguments,
                    },
                });
            }
        }

        let full_text = strip_think_tags(&full_text);

        let tool_calls = if !api_tool_calls.is_empty() {
            api_tool_calls
                .iter()
                .filter_map(ToolCall::from_api)
                .collect()
        } else {
            self.template().parse_tool_calls(&full_text)
        };

        Ok(LLMResponse {
            text: full_text,
            prompt_tokens: 0,
            completion_tokens: 0,
            tool_calls,
            api_tool_calls,
            stop_reason,
        })
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        Ok(text.len() / 4)
    }

    fn backend_name(&self) -> &str {
        "api"
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}
