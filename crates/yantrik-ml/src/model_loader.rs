//! Model file resolution — find config.json, tokenizer.json, and weights
//! from a local directory or download from HuggingFace Hub.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// Resolved paths to the three files needed for a BERT/sentence-transformer model.
pub struct ModelFiles {
    pub config: PathBuf,
    pub tokenizer: PathBuf,
    pub weights: PathBuf,
}

impl ModelFiles {
    /// Load model files from a local directory.
    ///
    /// Expects:
    /// - `config.json`
    /// - `tokenizer.json`
    /// - `model.safetensors` (preferred) or `pytorch_model.bin`
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let config = dir.join("config.json");
        if !config.exists() {
            bail!("config.json not found in {}", dir.display());
        }

        let tokenizer = dir.join("tokenizer.json");
        if !tokenizer.exists() {
            bail!("tokenizer.json not found in {}", dir.display());
        }

        let weights = if dir.join("model.safetensors").exists() {
            dir.join("model.safetensors")
        } else {
            bail!(
                "model.safetensors not found in {}",
                dir.display()
            );
        };

        Ok(Self {
            config,
            tokenizer,
            weights,
        })
    }

    /// Download model files from HuggingFace Hub.
    ///
    /// Uses `hf-hub` crate to resolve and cache files locally.
    pub fn from_hub(model_id: &str, revision: Option<&str>) -> Result<Self> {
        use hf_hub::api::sync::Api;
        use hf_hub::{Repo, RepoType};

        let repo = match revision {
            Some(rev) => Repo::with_revision(model_id.to_string(), RepoType::Model, rev.to_string()),
            None => Repo::model(model_id.to_string()),
        };

        let api = Api::new().context("Failed to create HF Hub API")?;
        let api = api.repo(repo);

        let config = api.get("config.json").context("Failed to download config.json")?;
        let tokenizer = api.get("tokenizer.json").context("Failed to download tokenizer.json")?;

        // Try safetensors first, fall back to pytorch
        let weights = api
            .get("model.safetensors")
            .context("Failed to download model.safetensors")?;

        Ok(Self {
            config,
            tokenizer,
            weights,
        })
    }
}

/// Resolved paths for a GGUF LLM model + tokenizer.
pub struct GGUFFiles {
    pub gguf: PathBuf,
    pub tokenizer: PathBuf,
}

impl GGUFFiles {
    /// Load from a local directory containing a .gguf file and tokenizer.json.
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let gguf = std::fs::read_dir(dir)
            .with_context(|| format!("reading dir: {}", dir.display()))?
            .filter_map(|e| e.ok())
            .find(|e| e.path().extension().map_or(false, |ext| ext == "gguf"))
            .map(|e| e.path())
            .ok_or_else(|| anyhow::anyhow!("no .gguf file found in {}", dir.display()))?;

        let tokenizer = dir.join("tokenizer.json");
        if !tokenizer.exists() {
            bail!("tokenizer.json not found in {}", dir.display());
        }

        Ok(Self { gguf, tokenizer })
    }

    /// Download GGUF model and tokenizer from HuggingFace Hub.
    ///
    /// Since GGUF repos often don't include tokenizer.json, this takes
    /// two repo IDs: one for the GGUF weights, one for the tokenizer.
    ///
    /// Example:
    /// ```rust,ignore
    /// GGUFFiles::from_hub(
    ///     "Qwen/Qwen2.5-0.5B-Instruct-GGUF",
    ///     "qwen2.5-0.5b-instruct-q4_k_m.gguf",
    ///     "Qwen/Qwen2.5-0.5B-Instruct",  // for tokenizer.json
    /// )
    /// ```
    pub fn from_hub(
        gguf_repo: &str,
        gguf_filename: &str,
        tokenizer_repo: &str,
    ) -> Result<Self> {
        use hf_hub::api::sync::Api;
        use hf_hub::Repo;

        let api = Api::new().context("Failed to create HF Hub API")?;

        // Download GGUF
        let gguf_api = api.repo(Repo::model(gguf_repo.to_string()));
        let gguf = gguf_api
            .get(gguf_filename)
            .with_context(|| format!("downloading {gguf_filename} from {gguf_repo}"))?;

        // Download tokenizer
        let tok_api = api.repo(Repo::model(tokenizer_repo.to_string()));
        let tokenizer = tok_api
            .get("tokenizer.json")
            .with_context(|| format!("downloading tokenizer.json from {tokenizer_repo}"))?;

        Ok(Self { gguf, tokenizer })
    }
}
