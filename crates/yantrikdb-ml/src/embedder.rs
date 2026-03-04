//! CandleEmbedder — MiniLM/BERT sentence embeddings via candle.
//!
//! Implements `yantrikdb_core::Embedder` so it can be plugged directly into YantrikDB:
//! ```rust,ignore
//! let embedder = CandleEmbedder::from_dir("/path/to/all-MiniLM-L6-v2")?;
//! db.set_embedder(Box::new(embedder));
//! db.record_text("Hello world", ...)?; // auto-embeds
//! ```

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

use crate::model_loader::ModelFiles;

/// Sentence embedding model backed by candle (BERT/MiniLM).
///
/// Thread-safe via internal `Mutex` on the model (candle models are `!Send`).
/// For single-threaded use (Yantrik companion), the mutex is uncontended.
pub struct CandleEmbedder {
    inner: Mutex<EmbedderInner>,
    dim: usize,
}

struct EmbedderInner {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

// Safety: The Mutex ensures only one thread accesses the model at a time.
// candle tensors on CPU are safe to share across threads when access is serialized.
unsafe impl Send for CandleEmbedder {}
unsafe impl Sync for CandleEmbedder {}

impl CandleEmbedder {
    /// Load a BERT/MiniLM model from a local directory.
    ///
    /// The directory must contain: `config.json`, `tokenizer.json`, `model.safetensors`.
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let files = ModelFiles::from_dir(dir)?;
        Self::load(files)
    }

    /// Load a model from HuggingFace Hub (downloads and caches).
    ///
    /// Example: `CandleEmbedder::from_hub("sentence-transformers/all-MiniLM-L6-v2", None)`
    pub fn from_hub(model_id: &str, revision: Option<&str>) -> Result<Self> {
        let files = ModelFiles::from_hub(model_id, revision)?;
        Self::load(files)
    }

    fn load(files: ModelFiles) -> Result<Self> {
        let device = Device::Cpu;

        // Load config
        let config_str =
            std::fs::read_to_string(&files.config).context("reading config.json")?;
        let config: BertConfig =
            serde_json::from_str(&config_str).context("parsing config.json")?;

        let dim = config.hidden_size;

        // Load tokenizer with padding
        let mut tokenizer =
            Tokenizer::from_file(&files.tokenizer).map_err(|e| anyhow::anyhow!("{e}"))?;
        tokenizer.with_padding(Some(PaddingParams {
            ..Default::default()
        }));
        tokenizer.with_truncation(Some(TruncationParams {
            max_length: 512,
            ..Default::default()
        })).map_err(|e| anyhow::anyhow!("{e}"))?;

        // Load model weights from safetensors
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[&files.weights], DType::F32, &device)
                .context("loading model.safetensors")?
        };

        let model = BertModel::load(vb, &config).context("building BertModel")?;

        tracing::info!(
            dim,
            model_path = %files.weights.display(),
            "CandleEmbedder loaded"
        );

        Ok(Self {
            inner: Mutex::new(EmbedderInner {
                model,
                tokenizer,
                device,
            }),
            dim,
        })
    }

    /// Embed a single text, returning a normalized f32 vector.
    fn embed_inner(inner: &EmbedderInner, text: &str) -> Result<Vec<f32>> {
        let encoding = inner
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("tokenizer encode: {e}"))?;

        let token_ids = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();

        let token_ids_t =
            Tensor::new(token_ids, &inner.device)?.unsqueeze(0)?;
        let attention_mask_t =
            Tensor::new(attention_mask, &inner.device)?.unsqueeze(0)?;
        let token_type_ids =
            token_ids_t.zeros_like()?;

        // Forward pass
        let embeddings = inner
            .model
            .forward(&token_ids_t, &token_type_ids, Some(&attention_mask_t))?;

        // Mask-aware mean pooling (matches Python sentence-transformers)
        let mask_f32 = attention_mask_t.to_dtype(DType::F32)?.unsqueeze(2)?;
        let sum_mask = mask_f32.sum(1)?;
        let pooled = embeddings.broadcast_mul(&mask_f32)?.sum(1)?;
        let pooled = pooled.broadcast_div(&sum_mask)?;

        // L2 normalize
        let norm = pooled.sqr()?.sum_keepdim(1)?.sqrt()?;
        let normalized = pooled.broadcast_div(&norm)?;

        // Extract as Vec<f32>
        let result = normalized.squeeze(0)?.to_vec1::<f32>()?;
        Ok(result)
    }
}

impl yantrikdb_core::types::Embedder for CandleEmbedder {
    fn embed(
        &self,
        text: &str,
    ) -> std::result::Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let inner = self.inner.lock().map_err(|e| format!("lock poisoned: {e}"))?;
        Self::embed_inner(&inner, text)
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })
    }

    fn embed_batch(
        &self,
        texts: &[&str],
    ) -> std::result::Result<Vec<Vec<f32>>, Box<dyn std::error::Error + Send + Sync>> {
        let inner = self.inner.lock().map_err(|e| format!("lock poisoned: {e}"))?;

        // For batch, we could batch-encode all at once for efficiency.
        // For now, sequential is fine — MiniLM embeds in ~5ms per text.
        texts
            .iter()
            .map(|t| {
                Self::embed_inner(&inner, t)
                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })
            })
            .collect()
    }

    fn dim(&self) -> usize {
        self.dim
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedder_trait_object() {
        // Verify CandleEmbedder can be used as a trait object
        fn assert_embedder<T: yantrikdb_core::types::Embedder>() {}
        assert_embedder::<CandleEmbedder>();
    }
}
