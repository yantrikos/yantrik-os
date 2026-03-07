//! llama.cpp-based LLM backend for GGUF models.
//!
//! Uses `llama-cpp-2` for inference with hardware-accelerated backends:
//! Vulkan (GPU), QNN (NPU), Metal, CUDA, or CPU with NEON.
//!
//! ```rust,ignore
//! let llm = LlamaCppLLM::from_gguf("/path/to/model.gguf", 99, 4096)?;
//! let resp = llm.chat(&messages, &config)?;
//! ```

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context as _, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;

use crate::chat_template;
use crate::traits::LLMBackend;
use crate::types::{ChatMessage, GenerationConfig, LLMResponse};

/// llama.cpp-backed LLM engine.
///
/// Thread-safe via internal Mutex. Stores the loaded model and creates
/// fresh inference contexts per generation call.
pub struct LlamaCppLLM {
    inner: Mutex<LlamaCppInner>,
}

struct LlamaCppInner {
    backend: LlamaBackend,
    model: LlamaModel,
    context_size: u32,
}

// Safety: Mutex serializes all access. The backend + model are only
// accessed under the lock, and llama.cpp is thread-safe when access
// is sequential (no concurrent calls to the same model/context).
unsafe impl Send for LlamaCppLLM {}
unsafe impl Sync for LlamaCppLLM {}

impl LlamaCppLLM {
    /// Load a GGUF model from a file path.
    ///
    /// - `gguf_path`: Path to the `.gguf` model file
    /// - `n_gpu_layers`: Number of layers to offload to GPU (99 = all)
    /// - `context_size`: Maximum context window (e.g. 4096, 8192)
    pub fn from_gguf(
        gguf_path: &Path,
        n_gpu_layers: u32,
        context_size: u32,
    ) -> Result<Self> {
        let backend = LlamaBackend::init().context("initializing llama.cpp backend")?;

        let model_params = LlamaModelParams::default()
            .with_n_gpu_layers(n_gpu_layers);

        tracing::info!(
            gguf_path = %gguf_path.display(),
            n_gpu_layers,
            context_size,
            gpu_offload = backend.supports_gpu_offload(),
            "Loading GGUF model via llama.cpp"
        );

        let model = LlamaModel::load_from_file(&backend, gguf_path, &model_params)
            .map_err(|e| anyhow::anyhow!("loading GGUF model: {e}"))?;

        tracing::info!(
            vocab_size = model.n_vocab(),
            n_embd = model.n_embd(),
            "LlamaCppLLM loaded"
        );

        Ok(Self {
            inner: Mutex::new(LlamaCppInner {
                backend,
                model,
                context_size,
            }),
        })
    }

    /// Format messages using the model's built-in chat template (from GGUF metadata),
    /// falling back to ChatML if unavailable.
    fn format_prompt(model: &LlamaModel, messages: &[ChatMessage]) -> Result<String> {
        match model.chat_template(None) {
            Ok(tmpl) => {
                let llama_messages: Vec<LlamaChatMessage> = messages
                    .iter()
                    .map(|m| {
                        LlamaChatMessage::new(m.role.clone(), m.content.clone())
                            .map_err(|e| anyhow::anyhow!("creating chat message: {e}"))
                    })
                    .collect::<Result<Vec<_>>>()?;

                model
                    .apply_chat_template(&tmpl, &llama_messages, true)
                    .map_err(|e| anyhow::anyhow!("applying chat template: {e}"))
            }
            Err(_) => {
                tracing::debug!("No chat template in GGUF, falling back to ChatML");
                Ok(chat_template::format_chat(messages))
            }
        }
    }

    /// Build a sampler chain from generation config.
    fn build_sampler(config: &GenerationConfig) -> LlamaSampler {
        if config.temperature <= 0.0 {
            LlamaSampler::chain_simple([LlamaSampler::greedy()])
        } else {
            let mut samplers: Vec<LlamaSampler> = Vec::new();

            if config.repeat_penalty != 1.0 {
                samplers.push(LlamaSampler::penalties(
                    config.repeat_last_n as i32,
                    config.repeat_penalty,
                    0.0, // frequency penalty
                    0.0, // presence penalty
                ));
            }

            if let Some(k) = config.top_k {
                samplers.push(LlamaSampler::top_k(k as i32));
            }

            if let Some(p) = config.top_p {
                samplers.push(LlamaSampler::top_p(p as f32, 1));
            }

            samplers.push(LlamaSampler::temp(config.temperature as f32));
            samplers.push(LlamaSampler::dist(config.seed as u32));

            LlamaSampler::chain_simple(samplers)
        }
    }

    /// Emit buffered UTF-8 bytes, streaming valid characters.
    /// Returns the number of valid bytes consumed from the buffer.
    fn flush_utf8(buf: &[u8], text: &mut String, on_token: &mut dyn FnMut(&str)) -> usize {
        match std::str::from_utf8(buf) {
            Ok(s) => {
                on_token(s);
                text.push_str(s);
                buf.len()
            }
            Err(e) => {
                let valid_up_to = e.valid_up_to();
                if valid_up_to > 0 {
                    let valid = std::str::from_utf8(&buf[..valid_up_to]).unwrap();
                    on_token(valid);
                    text.push_str(valid);
                }
                valid_up_to
            }
        }
    }

    /// Non-streaming generation.
    fn generate(inner: &mut LlamaCppInner, prompt: &str, config: &GenerationConfig) -> Result<LLMResponse> {
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(inner.context_size))
            .with_n_batch(512);

        let mut ctx = inner
            .model
            .new_context(&inner.backend, ctx_params)
            .map_err(|e| anyhow::anyhow!("creating context: {e}"))?;

        let prompt_tokens = inner
            .model
            .str_to_token(prompt, AddBos::Always)
            .map_err(|e| anyhow::anyhow!("tokenizing prompt: {e}"))?;

        let prompt_len = prompt_tokens.len();
        if prompt_len == 0 {
            anyhow::bail!("empty prompt after tokenization");
        }

        let n_ctx = ctx.n_ctx() as usize;
        if prompt_len >= n_ctx {
            anyhow::bail!("prompt ({prompt_len} tokens) exceeds context size ({n_ctx})");
        }

        let mut sampler = Self::build_sampler(config);
        let mut batch = LlamaBatch::new(512, 1);

        // Process prompt in batches
        let batch_size = 512usize;
        for chunk_start in (0..prompt_len).step_by(batch_size) {
            batch.clear();
            let chunk_end = (chunk_start + batch_size).min(prompt_len);
            let is_last_chunk = chunk_end == prompt_len;

            for (i, &token) in prompt_tokens[chunk_start..chunk_end].iter().enumerate() {
                let pos = (chunk_start + i) as i32;
                let logits = is_last_chunk && i == (chunk_end - chunk_start - 1);
                batch.add(token, pos, &[0], logits).context("batch add")?;
            }

            ctx.decode(&mut batch)
                .map_err(|e| anyhow::anyhow!("decode prompt: {e}"))?;
        }

        // Generation loop
        let mut generated_text = String::new();
        let mut completion_tokens = 0usize;
        let mut stop_reason = "length".to_string();
        let max_tokens = config.max_tokens.min(n_ctx - prompt_len);

        for _ in 0..max_tokens {
            let token = sampler.sample(&ctx, -1);
            completion_tokens += 1;

            if inner.model.is_eog_token(token) {
                stop_reason = "eos".to_string();
                break;
            }

            let piece = inner
                .model
                .token_to_piece_bytes(token, 128, false, None)
                .map_err(|e| anyhow::anyhow!("decode token: {e}"))?;

            // Accumulate bytes, decode to string at the end
            generated_text.push_str(&String::from_utf8_lossy(&piece));

            // Check stop sequences
            let mut found_stop = false;
            for stop_seq in &config.stop {
                if let Some(pos) = generated_text.find(stop_seq.as_str()) {
                    generated_text.truncate(pos);
                    stop_reason = "stop".to_string();
                    found_stop = true;
                    break;
                }
            }
            if found_stop {
                break;
            }

            // Feed token back for next step
            batch.clear();
            batch
                .add(token, (prompt_len + completion_tokens - 1) as i32, &[0], true)
                .context("batch add")?;

            ctx.decode(&mut batch)
                .map_err(|e| anyhow::anyhow!("decode token: {e}"))?;
        }

        let tool_calls = chat_template::parse_tool_calls(&generated_text);

        Ok(LLMResponse {
            text: generated_text,
            prompt_tokens: prompt_len,
            completion_tokens,
            tool_calls,
            api_tool_calls: Vec::new(),
            stop_reason,
        })
    }

    /// Streaming generation — calls on_token for each decoded text fragment.
    fn generate_streaming(
        inner: &mut LlamaCppInner,
        prompt: &str,
        config: &GenerationConfig,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(inner.context_size))
            .with_n_batch(512);

        let mut ctx = inner
            .model
            .new_context(&inner.backend, ctx_params)
            .map_err(|e| anyhow::anyhow!("creating context: {e}"))?;

        let prompt_tokens = inner
            .model
            .str_to_token(prompt, AddBos::Always)
            .map_err(|e| anyhow::anyhow!("tokenizing prompt: {e}"))?;

        let prompt_len = prompt_tokens.len();
        if prompt_len == 0 {
            anyhow::bail!("empty prompt after tokenization");
        }

        let n_ctx = ctx.n_ctx() as usize;
        if prompt_len >= n_ctx {
            anyhow::bail!("prompt ({prompt_len} tokens) exceeds context size ({n_ctx})");
        }

        let mut sampler = Self::build_sampler(config);
        let mut batch = LlamaBatch::new(512, 1);

        // Process prompt in batches
        let batch_size = 512usize;
        for chunk_start in (0..prompt_len).step_by(batch_size) {
            batch.clear();
            let chunk_end = (chunk_start + batch_size).min(prompt_len);
            let is_last_chunk = chunk_end == prompt_len;

            for (i, &token) in prompt_tokens[chunk_start..chunk_end].iter().enumerate() {
                let pos = (chunk_start + i) as i32;
                let logits = is_last_chunk && i == (chunk_end - chunk_start - 1);
                batch.add(token, pos, &[0], logits).context("batch add")?;
            }

            ctx.decode(&mut batch)
                .map_err(|e| anyhow::anyhow!("decode prompt: {e}"))?;
        }

        // Streaming generation loop
        let mut generated_text = String::new();
        let mut completion_tokens = 0usize;
        let mut stop_reason = "length".to_string();
        let mut utf8_buf: Vec<u8> = Vec::new();
        let max_tokens = config.max_tokens.min(n_ctx - prompt_len);

        for _ in 0..max_tokens {
            let token = sampler.sample(&ctx, -1);
            completion_tokens += 1;

            if inner.model.is_eog_token(token) {
                stop_reason = "eos".to_string();
                break;
            }

            let piece = inner
                .model
                .token_to_piece_bytes(token, 128, false, None)
                .map_err(|e| anyhow::anyhow!("decode token: {e}"))?;

            utf8_buf.extend_from_slice(&piece);

            // Flush valid UTF-8 from the buffer
            let consumed = Self::flush_utf8(&utf8_buf, &mut generated_text, on_token);
            if consumed > 0 {
                utf8_buf.drain(..consumed);
            }

            // Check stop sequences
            let mut found_stop = false;
            for stop_seq in &config.stop {
                if let Some(pos) = generated_text.find(stop_seq.as_str()) {
                    generated_text.truncate(pos);
                    stop_reason = "stop".to_string();
                    found_stop = true;
                    break;
                }
            }
            if found_stop {
                break;
            }

            // Feed token back
            batch.clear();
            batch
                .add(token, (prompt_len + completion_tokens - 1) as i32, &[0], true)
                .context("batch add")?;

            ctx.decode(&mut batch)
                .map_err(|e| anyhow::anyhow!("decode token: {e}"))?;
        }

        // Flush remaining bytes
        if !utf8_buf.is_empty() {
            if let Ok(s) = std::str::from_utf8(&utf8_buf) {
                on_token(s);
                generated_text.push_str(s);
            }
        }

        let tool_calls = chat_template::parse_tool_calls(&generated_text);

        Ok(LLMResponse {
            text: generated_text,
            prompt_tokens: prompt_len,
            completion_tokens,
            tool_calls,
            api_tool_calls: Vec::new(),
            stop_reason,
        })
    }
}

impl LLMBackend for LlamaCppLLM {
    fn chat(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        _tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        let mut inner = self.inner.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let prompt = Self::format_prompt(&inner.model, messages)?;
        Self::generate(&mut inner, &prompt, config)
    }

    fn chat_streaming(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        _tools: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        let mut inner = self.inner.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let prompt = Self::format_prompt(&inner.model, messages)?;
        Self::generate_streaming(&mut inner, &prompt, config, on_token)
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        let inner = self.inner.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let tokens = inner
            .model
            .str_to_token(text, AddBos::Never)
            .map_err(|e| anyhow::anyhow!("tokenizing: {e}"))?;
        Ok(tokens.len())
    }

    fn backend_name(&self) -> &str {
        "llama.cpp"
    }
}
