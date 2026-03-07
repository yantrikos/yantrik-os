//! Candle-based LLM backend for quantized Qwen2.5 GGUF models.
//!
//! Provides text generation, chat completion, and token counting —
//! all in-process, no HTTP servers.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::quantized_qwen2::ModelWeights;
use candle_transformers::utils::apply_repeat_penalty;
use tokenizers::Tokenizer;

use crate::chat_template::{self, Qwen2Tokens};
use crate::token_stream::TokenOutputStream;
use crate::traits::LLMBackend;
use crate::types::{ChatMessage, GenerationConfig, LLMResponse};

/// Quantized LLM engine backed by candle.
///
/// Thread-safe via internal Mutex (same pattern as CandleEmbedder).
/// For single-threaded Yantrik companion, the mutex is uncontended.
pub struct CandleLLM {
    inner: Mutex<CandleLLMInner>,
}

struct CandleLLMInner {
    model: ModelWeights,
    tokenizer: Tokenizer,
    device: Device,
    tokens: Qwen2Tokens,
}

// Safety: Mutex serializes all access. CPU tensors safe when access is serialized.
unsafe impl Send for CandleLLM {}
unsafe impl Sync for CandleLLM {}

impl CandleLLM {
    /// Load a quantized Qwen2/2.5 model from a GGUF file.
    ///
    /// The `tokenizer_path` should point to the `tokenizer.json` file
    /// (either in the same directory or downloaded separately).
    pub fn from_gguf(gguf_path: &Path, tokenizer_path: &Path) -> Result<Self> {
        // Device selection:
        // - CUDA: preferred on Windows/Linux with NVIDIA GPU
        // - Metal: NOT supported for quantized GGUF (missing rms-norm kernel in candle 0.9)
        // - CPU + Accelerate: fallback on macOS (uses vecLib BLAS)
        #[cfg(feature = "cuda")]
        let device = Device::new_cuda(0).unwrap_or_else(|e| {
            tracing::warn!("CUDA unavailable ({e}), falling back to CPU");
            Device::Cpu
        });
        #[cfg(not(feature = "cuda"))]
        let device = Device::Cpu;

        tracing::info!(device = ?device, "LLM device selected");

        // Load GGUF model
        let mut file = std::fs::File::open(gguf_path)
            .with_context(|| format!("opening GGUF: {}", gguf_path.display()))?;
        let ct = gguf_file::Content::read(&mut file)
            .context("reading GGUF content")?;

        tracing::info!(
            gguf_path = %gguf_path.display(),
            tensors = ct.tensor_infos.len(),
            "Loading quantized model"
        );

        let model = ModelWeights::from_gguf(ct, &mut file, &device)
            .context("building ModelWeights from GGUF")?;

        // Load tokenizer
        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("loading tokenizer: {e}"))?;

        let tokens = Qwen2Tokens::from_tokenizer(&tokenizer);

        tracing::info!(
            im_start = tokens.im_start,
            im_end = tokens.im_end,
            eos = tokens.eos,
            "CandleLLM loaded"
        );

        Ok(Self {
            inner: Mutex::new(CandleLLMInner {
                model,
                tokenizer,
                device,
                tokens,
            }),
        })
    }

    /// Load from a directory containing both the GGUF and tokenizer.
    ///
    /// Expects: `*.gguf` file and `tokenizer.json` in the directory.
    pub fn from_dir(dir: &Path) -> Result<Self> {
        // Find GGUF file
        let gguf_path = std::fs::read_dir(dir)
            .with_context(|| format!("reading dir: {}", dir.display()))?
            .filter_map(|e| e.ok())
            .find(|e| {
                e.path()
                    .extension()
                    .map_or(false, |ext| ext == "gguf")
            })
            .map(|e| e.path())
            .ok_or_else(|| anyhow::anyhow!("no .gguf file found in {}", dir.display()))?;

        let tokenizer_path = dir.join("tokenizer.json");
        if !tokenizer_path.exists() {
            anyhow::bail!(
                "tokenizer.json not found in {}",
                dir.display()
            );
        }

        Self::from_gguf(&gguf_path, &tokenizer_path)
    }

    /// Generate text from a raw prompt string.
    pub fn generate(&self, prompt: &str, config: &GenerationConfig) -> Result<LLMResponse> {
        let mut inner = self.inner.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        Self::generate_inner(&mut inner, prompt, config)
    }

    /// Chat completion — formats messages as ChatML, generates, parses tool calls.
    pub fn chat(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
    ) -> Result<LLMResponse> {
        let prompt = chat_template::format_chat(messages);
        self.generate(&prompt, config)
    }

    /// Generate with streaming — calls `on_token` for each decoded text fragment.
    pub fn generate_streaming<F>(
        &self,
        prompt: &str,
        config: &GenerationConfig,
        mut on_token: F,
    ) -> Result<LLMResponse>
    where
        F: FnMut(&str),
    {
        let mut inner = self.inner.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        Self::generate_streaming_inner(&mut inner, prompt, config, &mut on_token)
    }

    /// Chat completion with streaming.
    pub fn chat_streaming_generic<F>(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        on_token: F,
    ) -> Result<LLMResponse>
    where
        F: FnMut(&str),
    {
        let prompt = chat_template::format_chat(messages);
        self.generate_streaming(&prompt, config, on_token)
    }

    /// Count tokens in a text string using the loaded tokenizer.
    pub fn count_tokens(&self, text: &str) -> Result<usize> {
        let inner = self.inner.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let encoding = inner
            .tokenizer
            .encode(text, false)
            .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;
        Ok(encoding.get_ids().len())
    }

    /// Get a reference to the tokenizer (for external token counting, etc).
    pub fn tokenizer(&self) -> Result<Tokenizer> {
        let inner = self.inner.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        Ok(inner.tokenizer.clone())
    }

    fn generate_inner(
        inner: &mut CandleLLMInner,
        prompt: &str,
        config: &GenerationConfig,
    ) -> Result<LLMResponse> {
        // Tokenize prompt
        let encoding = inner
            .tokenizer
            .encode(prompt, false)
            .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;
        let prompt_tokens: Vec<u32> = encoding.get_ids().to_vec();
        let prompt_len = prompt_tokens.len();

        if prompt_len == 0 {
            anyhow::bail!("empty prompt after tokenization");
        }

        // Set up sampling
        let sampling = if config.temperature <= 0.0 {
            Sampling::ArgMax
        } else {
            match (config.top_k, config.top_p) {
                (None, None) => Sampling::All {
                    temperature: config.temperature,
                },
                (Some(k), None) => Sampling::TopK {
                    k,
                    temperature: config.temperature,
                },
                (None, Some(p)) => Sampling::TopP {
                    p,
                    temperature: config.temperature,
                },
                (Some(k), Some(p)) => Sampling::TopKThenTopP {
                    k,
                    p,
                    temperature: config.temperature,
                },
            }
        };
        let mut logits_processor = LogitsProcessor::from_sampling(config.seed, sampling);

        // Build stop token set
        let stop_token_ids: Vec<u32> = config
            .stop
            .iter()
            .filter_map(|s| inner.tokenizer.token_to_id(s))
            .collect();

        // Token output stream for decoding
        let mut token_stream = TokenOutputStream::new(inner.tokenizer.clone());

        // All tokens (prompt + generated) for repeat penalty context
        let mut all_tokens = prompt_tokens.clone();
        let mut generated_text = String::new();
        let mut completion_tokens = 0usize;
        let mut stop_reason = "length".to_string();

        // Process prompt (prefill)
        let mut index_pos = 0usize;
        let mut current_tokens = prompt_tokens;

        for _i in 0..config.max_tokens {
            let input = Tensor::new(current_tokens.as_slice(), &inner.device)?.unsqueeze(0)?;
            let logits = inner.model.forward(&input, index_pos)?;

            // logits shape: [1, seq_len, vocab_size] — take last position
            let logits = logits.squeeze(0)?;
            let logits = if logits.dims().len() == 2 {
                // Multiple tokens in input: take last position
                let seq_len = logits.dim(0)?;
                logits.get(seq_len - 1)?
            } else {
                // Single token: already 1D
                logits
            };

            // Apply repetition penalty
            let logits = if config.repeat_penalty != 1.0 && !all_tokens.is_empty() {
                let start = all_tokens.len().saturating_sub(config.repeat_last_n);
                apply_repeat_penalty(&logits, config.repeat_penalty, &all_tokens[start..])?
            } else {
                logits
            };

            // Sample next token
            let next_token = logits_processor.sample(&logits)?;
            completion_tokens += 1;
            all_tokens.push(next_token);

            // Check EOS / im_end
            if inner.tokens.is_stop(next_token) {
                stop_reason = "eos".to_string();
                break;
            }

            // Check custom stop tokens
            if stop_token_ids.contains(&next_token) {
                stop_reason = "stop".to_string();
                break;
            }

            // Decode token
            if let Some(text) = token_stream.next_token(next_token)? {
                generated_text.push_str(&text);

                // Check stop sequences in generated text
                let mut found_stop = false;
                for stop_seq in &config.stop {
                    if generated_text.contains(stop_seq) {
                        // Trim text at stop sequence
                        if let Some(pos) = generated_text.find(stop_seq) {
                            generated_text.truncate(pos);
                        }
                        stop_reason = "stop".to_string();
                        found_stop = true;
                        break;
                    }
                }
                if found_stop {
                    break;
                }
            }

            // Next iteration: only feed the new token
            index_pos += current_tokens.len();
            current_tokens = vec![next_token];
        }

        // Flush remaining bytes
        if let Some(rest) = token_stream.decode_rest()? {
            generated_text.push_str(&rest);
        }

        // Parse tool calls from output
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

    fn generate_streaming_inner<F>(
        inner: &mut CandleLLMInner,
        prompt: &str,
        config: &GenerationConfig,
        on_token: &mut F,
    ) -> Result<LLMResponse>
    where
        F: FnMut(&str) + ?Sized,
    {
        let encoding = inner
            .tokenizer
            .encode(prompt, false)
            .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;
        let prompt_tokens: Vec<u32> = encoding.get_ids().to_vec();
        let prompt_len = prompt_tokens.len();

        tracing::info!(prompt_tokens = prompt_len, max_tokens = config.max_tokens, "Starting generation");

        if prompt_len == 0 {
            anyhow::bail!("empty prompt after tokenization");
        }

        let sampling = if config.temperature <= 0.0 {
            Sampling::ArgMax
        } else {
            match (config.top_k, config.top_p) {
                (None, None) => Sampling::All {
                    temperature: config.temperature,
                },
                (Some(k), None) => Sampling::TopK {
                    k,
                    temperature: config.temperature,
                },
                (None, Some(p)) => Sampling::TopP {
                    p,
                    temperature: config.temperature,
                },
                (Some(k), Some(p)) => Sampling::TopKThenTopP {
                    k,
                    p,
                    temperature: config.temperature,
                },
            }
        };
        let mut logits_processor = LogitsProcessor::from_sampling(config.seed, sampling);

        let stop_token_ids: Vec<u32> = config
            .stop
            .iter()
            .filter_map(|s| inner.tokenizer.token_to_id(s))
            .collect();

        let mut token_stream = TokenOutputStream::new(inner.tokenizer.clone());
        let mut all_tokens = prompt_tokens.clone();
        let mut generated_text = String::new();
        let mut completion_tokens = 0usize;
        let mut stop_reason = "length".to_string();

        let mut index_pos = 0usize;
        let mut current_tokens = prompt_tokens;

        for _i in 0..config.max_tokens {
            let input = Tensor::new(current_tokens.as_slice(), &inner.device)?.unsqueeze(0)?;
            let logits = inner.model.forward(&input, index_pos)?;

            let logits = logits.squeeze(0)?;
            let logits = if logits.dims().len() == 2 {
                let seq_len = logits.dim(0)?;
                logits.get(seq_len - 1)?
            } else {
                logits
            };

            let logits = if config.repeat_penalty != 1.0 && !all_tokens.is_empty() {
                let start = all_tokens.len().saturating_sub(config.repeat_last_n);
                apply_repeat_penalty(&logits, config.repeat_penalty, &all_tokens[start..])?
            } else {
                logits
            };

            let next_token = logits_processor.sample(&logits)?;
            completion_tokens += 1;
            all_tokens.push(next_token);

            if inner.tokens.is_stop(next_token) {
                stop_reason = "eos".to_string();
                break;
            }

            if stop_token_ids.contains(&next_token) {
                stop_reason = "stop".to_string();
                break;
            }

            if let Some(text) = token_stream.next_token(next_token)? {
                // Stream the token to the callback
                on_token(&text);
                generated_text.push_str(&text);

                let mut found_stop = false;
                for stop_seq in &config.stop {
                    if generated_text.contains(stop_seq) {
                        if let Some(pos) = generated_text.find(stop_seq) {
                            generated_text.truncate(pos);
                        }
                        stop_reason = "stop".to_string();
                        found_stop = true;
                        break;
                    }
                }
                if found_stop {
                    break;
                }
            }

            index_pos += current_tokens.len();
            current_tokens = vec![next_token];
        }

        if let Some(rest) = token_stream.decode_rest()? {
            on_token(&rest);
            generated_text.push_str(&rest);
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

impl LLMBackend for CandleLLM {
    fn chat(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        _tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        self.chat(messages, config)
    }

    fn chat_streaming(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        _tools: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        let prompt = chat_template::format_chat(messages);
        let mut inner = self.inner.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        Self::generate_streaming_inner(&mut inner, &prompt, config, on_token)
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        self.count_tokens(text)
    }

    fn backend_name(&self) -> &str {
        "candle"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generation_config_default() {
        let config = GenerationConfig::default();
        assert_eq!(config.max_tokens, 512);
        assert!((config.temperature - 0.7).abs() < 0.001);
        assert_eq!(config.top_p, Some(0.9));
    }

    #[test]
    fn test_generation_config_greedy() {
        let config = GenerationConfig::greedy();
        assert!((config.temperature - 0.0).abs() < 0.001);
        assert_eq!(config.top_p, None);
    }
}
