//! Fallback LLM — wraps a primary backend with an automatic fallback.
//!
//! When the primary backend fails (network error, timeout, CLI crash),
//! the fallback backend is lazily initialized and used instead.
//! This enables offline intelligence via a local GGUF model (e.g. Qwen3.5-0.8B)
//! while keeping the powerful primary backend (Claude CLI, API) for normal use.
//!
//! Supports two fallback modes:
//! - `api` — fallback to a local llama-server (or any OpenAI-compatible endpoint)
//! - `llamacpp` — fallback via embedded llama.cpp (requires `llamacpp` feature)

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tracing;

use crate::traits::LLMBackend;
use crate::types::{ChatMessage, GenerationConfig, LLMResponse};

/// Configuration for the fallback LLM backend.
#[derive(Debug, Clone)]
pub enum FallbackConfig {
    /// Use an API endpoint (e.g. local llama-server) as fallback.
    Api {
        base_url: String,
        model: String,
    },
    /// Use embedded llama.cpp (requires `llamacpp` feature at compile time).
    LlamaCpp {
        model_path: PathBuf,
        n_gpu_layers: u32,
        context_size: u32,
    },
}

/// An LLM backend that wraps a primary + fallback.
///
/// On every call, tries the primary backend first. If it fails,
/// lazily initializes and uses the fallback.
pub struct FallbackLLM {
    primary: Arc<dyn LLMBackend>,
    fallback: Mutex<Option<Arc<dyn LLMBackend>>>,
    fallback_config: Option<FallbackConfig>,
    /// Track consecutive primary failures to avoid slow retries
    primary_failures: Mutex<u32>,
}

impl FallbackLLM {
    /// Create a new FallbackLLM.
    ///
    /// The fallback is NOT initialized until the primary fails.
    /// If `fallback_config` is None, no fallback is available — primary errors pass through.
    pub fn new(
        primary: Arc<dyn LLMBackend>,
        fallback_config: Option<FallbackConfig>,
    ) -> Self {
        Self {
            primary,
            fallback: Mutex::new(None),
            fallback_config,
            primary_failures: Mutex::new(0),
        }
    }

    /// Create with an already-initialized fallback backend (for testing or pre-warming).
    pub fn with_fallback(
        primary: Arc<dyn LLMBackend>,
        fallback: Arc<dyn LLMBackend>,
    ) -> Self {
        Self {
            primary,
            fallback: Mutex::new(Some(fallback)),
            fallback_config: None,
            primary_failures: Mutex::new(0),
        }
    }

    /// Get or initialize the fallback backend.
    fn get_fallback(&self) -> Result<Arc<dyn LLMBackend>> {
        let mut guard = self.fallback.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;

        if let Some(ref fb) = *guard {
            return Ok(fb.clone());
        }

        let config = self.fallback_config.as_ref()
            .ok_or_else(|| anyhow::anyhow!("no fallback configured"))?;

        let fb: Arc<dyn LLMBackend> = match config {
            FallbackConfig::Api { base_url, model } => {
                tracing::info!(
                    base_url = %base_url,
                    model = %model,
                    "Initializing fallback LLM (API)"
                );
                #[cfg(feature = "api-llm")]
                {
                    Arc::new(crate::llm::ApiLLM::new(
                        base_url.clone(),
                        None, // no API key for local server
                        model,
                    ))
                }
                #[cfg(not(feature = "api-llm"))]
                {
                    anyhow::bail!("fallback API requires 'api-llm' feature at compile time")
                }
            }
            FallbackConfig::LlamaCpp { model_path, n_gpu_layers, context_size } => {
                if !model_path.exists() {
                    anyhow::bail!(
                        "fallback model not found: {}",
                        model_path.display()
                    );
                }
                tracing::info!(
                    model = %model_path.display(),
                    gpu_layers = n_gpu_layers,
                    ctx = context_size,
                    "Initializing fallback LLM (llama.cpp)"
                );
                #[cfg(feature = "llamacpp")]
                {
                    Arc::new(crate::llm::LlamaCppLLM::from_gguf(
                        model_path,
                        *n_gpu_layers,
                        *context_size,
                    )?)
                }
                #[cfg(not(feature = "llamacpp"))]
                {
                    anyhow::bail!("fallback requires 'llamacpp' feature at compile time")
                }
            }
        };

        *guard = Some(fb.clone());
        Ok(fb)
    }

    /// Record a primary success — reset failure counter.
    fn primary_success(&self) {
        if let Ok(mut count) = self.primary_failures.lock() {
            if *count > 0 {
                tracing::info!(prev_failures = *count, "Primary LLM recovered");
                *count = 0;
            }
        }
    }

    /// Record a primary failure — increment counter.
    fn primary_failure(&self) {
        if let Ok(mut count) = self.primary_failures.lock() {
            *count += 1;
        }
    }

    /// Check if we should skip the primary (too many consecutive failures).
    /// After 3 consecutive failures, go straight to fallback for 10 calls,
    /// then retry primary once.
    fn should_skip_primary(&self) -> bool {
        if let Ok(count) = self.primary_failures.lock() {
            *count >= 3 && *count % 10 != 0
        } else {
            false
        }
    }

    /// Slim down messages for the tiny fallback model.
    /// Keeps system prompt (truncated), last user message, and at most 2 history turns.
    fn slim_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
        let mut slim = Vec::with_capacity(4);

        // System message: truncate to ~600 chars
        if let Some(sys) = messages.first() {
            if sys.role == "system" {
                let content = if sys.content.len() > 600 {
                    // Find a safe truncation point
                    let mut end = 600;
                    while end > 0 && !sys.content.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}\nBe concise. Answer directly.", &sys.content[..end])
                } else {
                    sys.content.clone()
                };
                slim.push(ChatMessage::system(content));
            }
        }

        // Last user message (always keep)
        if let Some(last) = messages.last() {
            if last.role == "user" {
                // Keep at most 2 preceding history turns
                let history_start = if messages.len() > 5 { messages.len() - 5 } else { 1 };
                for msg in &messages[history_start..messages.len() - 1] {
                    slim.push(msg.clone());
                }
                slim.push(last.clone());
            }
        }

        slim
    }

    /// Reduced generation config for fallback: fewer tokens, lower temp.
    fn slim_config(config: &GenerationConfig) -> GenerationConfig {
        GenerationConfig {
            max_tokens: config.max_tokens.min(512),
            temperature: config.temperature.min(0.5),
            top_p: config.top_p,
            ..Default::default()
        }
    }
}

impl LLMBackend for FallbackLLM {
    fn chat(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        // If primary has been failing consistently, skip to fallback
        if self.should_skip_primary() {
            if let Ok(fb) = self.get_fallback() {
                tracing::debug!("Skipping primary (consecutive failures), using fallback");
                return fb.chat(messages, config, tools);
            }
        }

        match self.primary.chat(messages, config, tools) {
            Ok(resp) => {
                self.primary_success();
                Ok(resp)
            }
            Err(primary_err) => {
                self.primary_failure();
                tracing::warn!(
                    error = %primary_err,
                    primary = self.primary.backend_name(),
                    "Primary LLM failed, trying fallback"
                );

                match self.get_fallback() {
                    Ok(fb) => {
                        let slim = Self::slim_messages(messages);
                        let slim_cfg = Self::slim_config(config);
                        fb.chat(&slim, &slim_cfg, None)
                    }
                    Err(fb_err) => {
                        tracing::error!(
                            fallback_error = %fb_err,
                            "Fallback LLM also failed"
                        );
                        Err(primary_err)
                    }
                }
            }
        }
    }

    fn chat_streaming(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        if self.should_skip_primary() {
            if let Ok(fb) = self.get_fallback() {
                tracing::debug!("Skipping primary (consecutive failures), using fallback streaming");
                return fb.chat_streaming(messages, config, tools, on_token);
            }
        }

        match self.primary.chat_streaming(messages, config, tools, on_token) {
            Ok(resp) => {
                self.primary_success();
                Ok(resp)
            }
            Err(primary_err) => {
                self.primary_failure();
                tracing::warn!(
                    error = %primary_err,
                    primary = self.primary.backend_name(),
                    "Primary LLM streaming failed, trying fallback"
                );

                match self.get_fallback() {
                    Ok(fb) => {
                        let slim = Self::slim_messages(messages);
                        let slim_cfg = Self::slim_config(config);
                        fb.chat_streaming(&slim, &slim_cfg, None, on_token)
                    }
                    Err(_) => Err(primary_err),
                }
            }
        }
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        self.primary.count_tokens(text)
            .or_else(|_| {
                self.get_fallback()
                    .and_then(|fb| fb.count_tokens(text))
            })
    }

    fn backend_name(&self) -> &str {
        "fallback"
    }

    fn is_degraded(&self) -> bool {
        if let Ok(count) = self.primary_failures.lock() {
            *count > 0
        } else {
            false
        }
    }

    fn model_id(&self) -> &str {
        self.primary.model_id()
    }
}
