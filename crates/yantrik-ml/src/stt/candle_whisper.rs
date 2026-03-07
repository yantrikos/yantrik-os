//! Candle-based Whisper STT backend — speech-to-text via candle, in-process.
//!
//! Uses candle_transformers::models::whisper.
//! Default model: openai/whisper-tiny (~75MB, safetensors).
//!
//! ```rust,ignore
//! let stt = CandleWhisper::from_hub("openai/whisper-tiny")?;
//! let text = stt.transcribe(&pcm_16khz_mono)?;
//! ```

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::whisper::{self as m, audio, Config};
use tokenizers::Tokenizer;

use crate::traits::STTBackend;
use crate::types::TranscribeResult;

/// Whisper-based speech-to-text engine backed by candle.
///
/// Thread-safe via internal Mutex (same pattern as CandleLLM).
pub struct CandleWhisper {
    inner: Mutex<CandleWhisperInner>,
}

struct CandleWhisperInner {
    model: m::model::Whisper,
    tokenizer: Tokenizer,
    mel_filters: Vec<f32>,
    device: Device,
    suppress_tokens: Vec<u32>,
}

// Safety: Mutex serializes all access. CPU tensors safe when access is serialized.
unsafe impl Send for CandleWhisper {}
unsafe impl Sync for CandleWhisper {}

impl CandleWhisper {
    /// Load Whisper from HuggingFace Hub (downloads and caches).
    ///
    /// Example: `CandleWhisper::from_hub("openai/whisper-tiny")`
    pub fn from_hub(model_id: &str) -> Result<Self> {
        use hf_hub::api::sync::Api;
        use hf_hub::Repo;

        tracing::info!(model_id, "Downloading Whisper model from HuggingFace Hub");

        let api = Api::new().context("Failed to create HF Hub API")?;
        let repo = api.repo(Repo::model(model_id.to_string()));

        let config_path = repo
            .get("config.json")
            .context("downloading config.json")?;
        let weights_path = repo
            .get("model.safetensors")
            .context("downloading model.safetensors")?;
        let tokenizer_path = repo
            .get("tokenizer.json")
            .context("downloading tokenizer.json")?;

        Self::load(&config_path, &weights_path, &tokenizer_path)
    }

    /// Load Whisper from a local directory.
    ///
    /// Expects: config.json, model.safetensors, tokenizer.json
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let config_path = dir.join("config.json");
        let weights_path = dir.join("model.safetensors");
        let tokenizer_path = dir.join("tokenizer.json");
        Self::load(&config_path, &weights_path, &tokenizer_path)
    }

    fn load(config_path: &Path, weights_path: &Path, tokenizer_path: &Path) -> Result<Self> {
        let device = Device::Cpu;

        // Load config
        let config_str =
            std::fs::read_to_string(config_path).context("reading Whisper config.json")?;
        let config: Config =
            serde_json::from_str(&config_str).context("parsing Whisper config.json")?;

        let suppress_tokens = config.suppress_tokens.clone();
        let num_mel_bins = config.num_mel_bins;

        // Compute mel filterbank
        let mel_filters = compute_mel_filters(num_mel_bins, m::N_FFT, m::SAMPLE_RATE as f64);

        tracing::info!(
            num_mel_bins,
            mel_filters_len = mel_filters.len(),
            "Computed mel filterbank"
        );

        // Load model
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)
                .context("loading Whisper model.safetensors")?
        };
        let model = m::model::Whisper::load(&vb, config).context("building Whisper model")?;

        // Load tokenizer
        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("loading Whisper tokenizer: {e}"))?;

        tracing::info!("CandleWhisper loaded");

        Ok(Self {
            inner: Mutex::new(CandleWhisperInner {
                model,
                tokenizer,
                mel_filters,
                device,
                suppress_tokens,
            }),
        })
    }

    /// Transcribe 16kHz mono f32 PCM audio to text.
    pub fn transcribe(&self, pcm: &[f32]) -> Result<TranscribeResult> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        Self::transcribe_inner(&mut inner, pcm)
    }

    fn transcribe_inner(inner: &mut CandleWhisperInner, pcm: &[f32]) -> Result<TranscribeResult> {
        if pcm.is_empty() {
            return Ok(TranscribeResult {
                text: String::new(),
                tokens: 0,
            });
        }

        let config = &inner.model.config;

        // Convert PCM to mel spectrogram
        let mel = audio::pcm_to_mel(config, pcm, &inner.mel_filters);
        let mel_len = mel.len();
        let n_mels = config.num_mel_bins;
        let n_frames = mel_len / n_mels;
        let mel =
            Tensor::from_vec(mel, (1, n_mels, n_frames), &inner.device)?;

        // Reset KV cache for fresh transcription
        inner.model.reset_kv_cache();

        // Encode audio
        let audio_features = inner.model.encoder.forward(&mel, true)?;

        // Build initial decoder tokens: [SOT, LANG, TRANSCRIBE, NO_TIMESTAMPS]
        let sot = token_id(&inner.tokenizer, m::SOT_TOKEN).unwrap_or(50258);
        let lang_en = token_id(&inner.tokenizer, "<|en|>").unwrap_or(50259);
        let transcribe = token_id(&inner.tokenizer, m::TRANSCRIBE_TOKEN).unwrap_or(50359);
        let no_timestamps = token_id(&inner.tokenizer, m::NO_TIMESTAMPS_TOKEN).unwrap_or(50363);
        let eot = token_id(&inner.tokenizer, m::EOT_TOKEN).unwrap_or(50257);

        let mut tokens: Vec<u32> = vec![sot, lang_en, transcribe, no_timestamps];
        let mut result_tokens: Vec<u32> = Vec::new();

        tracing::info!(
            sot, lang_en, transcribe, no_timestamps, eot,
            suppress_count = inner.suppress_tokens.len(),
            "Whisper decoder token IDs"
        );

        let max_decode_len: usize = 224;

        // Greedy decoding loop
        for i in 0..max_decode_len {
            let token_t =
                Tensor::new(tokens.as_slice(), &inner.device)?.unsqueeze(0)?;

            let logits =
                inner
                    .model
                    .decoder
                    .forward(&token_t, &audio_features, i == 0)?;
            let logits = inner.model.decoder.final_linear(&logits)?;

            // Get logits for the last token position
            let seq_len = logits.dim(1)?;
            let next_logits = logits.i((0, seq_len - 1))?;

            // Suppress special tokens
            let next_logits = suppress_tokens_mask(&next_logits, &inner.suppress_tokens, eot)?;

            // Greedy: pick argmax
            let next_token = next_logits.argmax(0)?.to_scalar::<u32>()?;

            if i < 5 {
                tracing::info!(step = i, next_token, "Whisper decode step");
            }

            if next_token == eot {
                tracing::info!(step = i, "Whisper hit EOT");
                break;
            }

            result_tokens.push(next_token);
            tokens = vec![next_token];
        }

        tracing::info!(
            result_token_count = result_tokens.len(),
            result_tokens = ?&result_tokens[..result_tokens.len().min(20)],
            "Whisper decode complete"
        );

        // Decode token IDs to text
        let text = inner
            .tokenizer
            .decode(&result_tokens, true)
            .map_err(|e| anyhow::anyhow!("decode: {e}"))?;

        tracing::info!(decoded_text = %text, "Whisper raw decoded text");

        Ok(TranscribeResult {
            text: text.trim().to_string(),
            tokens: result_tokens.len(),
        })
    }

    /// Get the expected sample rate (always 16000 for Whisper).
    pub fn sample_rate(&self) -> u32 {
        m::SAMPLE_RATE as u32
    }
}

impl STTBackend for CandleWhisper {
    fn transcribe(&self, pcm_16khz_mono: &[f32]) -> Result<TranscribeResult> {
        self.transcribe(pcm_16khz_mono)
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate()
    }

    fn backend_name(&self) -> &str {
        "candle-whisper"
    }
}

/// Look up a special token ID by its string representation.
fn token_id(tokenizer: &Tokenizer, token: &str) -> Option<u32> {
    tokenizer.token_to_id(token)
}

/// Apply suppress_tokens by setting their logits to -inf.
fn suppress_tokens_mask(logits: &Tensor, suppress: &[u32], eot: u32) -> Result<Tensor> {
    if suppress.is_empty() {
        return Ok(logits.clone());
    }

    let vocab_size = logits.dim(0)?;
    let mut mask = vec![0.0f32; vocab_size];
    for &token_id in suppress {
        if (token_id as usize) < vocab_size && token_id != eot {
            mask[token_id as usize] = f32::NEG_INFINITY;
        }
    }
    let mask_t = Tensor::from_vec(mask, vocab_size, logits.device())?;
    Ok((logits + mask_t)?)
}

// --- Mel filterbank computation (Slaney scale, matches librosa/OpenAI Whisper) ---

const F_SP: f64 = 200.0 / 3.0;
const MIN_LOG_HZ: f64 = 1000.0;
const MIN_LOG_MEL: f64 = 15.0; // 1000.0 / (200.0 / 3.0)
const LOG_STEP: f64 = 0.068_751_777_420_949_12; // ln(6.4) / 27.0

fn hz_to_mel(hz: f64) -> f64 {
    if hz < MIN_LOG_HZ {
        hz / F_SP
    } else {
        MIN_LOG_MEL + (hz / MIN_LOG_HZ).ln() / LOG_STEP
    }
}

fn mel_to_hz(mel: f64) -> f64 {
    if mel < MIN_LOG_MEL {
        mel * F_SP
    } else {
        MIN_LOG_HZ * ((mel - MIN_LOG_MEL) * LOG_STEP).exp()
    }
}

/// Compute mel filterbank matching librosa's Slaney normalization.
///
/// Returns a flat array of shape [num_mel_bins, n_fft/2+1].
fn compute_mel_filters(num_mel_bins: usize, n_fft: usize, sample_rate: f64) -> Vec<f32> {
    let n_freqs = n_fft / 2 + 1;

    // Mel scale endpoints
    let mel_min = hz_to_mel(0.0);
    let mel_max = hz_to_mel(sample_rate / 2.0);

    // n_mels + 2 points for triangular filters
    let n_points = num_mel_bins + 2;
    let mel_points: Vec<f64> = (0..n_points)
        .map(|i| mel_min + (mel_max - mel_min) * i as f64 / (n_points - 1) as f64)
        .collect();

    let hz_points: Vec<f64> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();

    // FFT bin center frequencies
    let fft_freqs: Vec<f64> = (0..n_freqs)
        .map(|i| i as f64 * sample_rate / n_fft as f64)
        .collect();

    let mut filters = vec![0.0f32; num_mel_bins * n_freqs];

    for i in 0..num_mel_bins {
        let f_left = hz_points[i];
        let f_center = hz_points[i + 1];
        let f_right = hz_points[i + 2];

        // Slaney normalization factor
        let enorm = 2.0 / (f_right - f_left);

        for j in 0..n_freqs {
            let f = fft_freqs[j];
            let val = if f >= f_left && f <= f_center && f_center > f_left {
                (f - f_left) / (f_center - f_left)
            } else if f > f_center && f <= f_right && f_right > f_center {
                (f_right - f) / (f_right - f_center)
            } else {
                0.0
            };
            filters[i * n_freqs + j] = (val * enorm) as f32;
        }
    }

    filters
}
