//! API key validation — verify provider credentials before persisting.
//!
//! Validates keys by calling a lightweight provider endpoint (model list or
//! a minimal inference request). Keys are only stored in the SecretStore
//! after successful validation.

use serde::{Deserialize, Serialize};

use super::descriptor::{AuthScheme, ProviderDescriptor, ProviderKind};

// ── Validation errors ────────────────────────────────────────────────────

/// Specific reasons an API key validation can fail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyValidationError {
    /// Key is malformed or rejected by the provider (401/403).
    InvalidKey,
    /// Key is valid but has no billing/credits (402).
    NoBilling,
    /// Key is valid but currently rate-limited (429).
    RateLimited,
    /// Key works but the requested model is not available.
    ModelUnavailable,
    /// Could not reach the provider (DNS, TCP, timeout).
    NetworkTimeout,
    /// TLS handshake or certificate error.
    TLSError,
    /// Provider returned a region/country block (451 or geo-restriction).
    RegionBlocked,
    /// An unexpected error occurred.
    Unknown(String),
}

impl std::fmt::Display for KeyValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyValidationError::InvalidKey => write!(f, "Invalid API key — check your key and try again"),
            KeyValidationError::NoBilling => write!(f, "API key has no billing or credits remaining"),
            KeyValidationError::RateLimited => write!(f, "API key is rate-limited — try again later"),
            KeyValidationError::ModelUnavailable => write!(f, "Requested model is not available for this key"),
            KeyValidationError::NetworkTimeout => write!(f, "Could not reach the provider — check your network"),
            KeyValidationError::TLSError => write!(f, "TLS/SSL error — certificate or handshake failure"),
            KeyValidationError::RegionBlocked => write!(f, "Provider access blocked in your region"),
            KeyValidationError::Unknown(msg) => write!(f, "Validation error: {}", msg),
        }
    }
}

// ── Validation result ────────────────────────────────────────────────────

/// Result of validating an API key against a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyValidationResult {
    /// Whether the key is valid and usable.
    pub is_valid: bool,
    /// The validation error (if any).
    pub error: Option<KeyValidationError>,
    /// Latency of the validation request in milliseconds.
    pub latency_ms: u64,
    /// Models available with this key (if the provider returned them).
    pub available_models: Vec<String>,
    /// Timestamp of this validation (seconds since UNIX epoch).
    pub validated_at: u64,
}

impl KeyValidationResult {
    fn success(latency_ms: u64, models: Vec<String>) -> Self {
        Self {
            is_valid: true,
            error: None,
            latency_ms,
            available_models: models,
            validated_at: now_epoch(),
        }
    }

    fn failure(error: KeyValidationError, latency_ms: u64) -> Self {
        Self {
            is_valid: false,
            error: Some(error),
            latency_ms,
            available_models: Vec::new(),
            validated_at: now_epoch(),
        }
    }
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Validator ────────────────────────────────────────────────────────────

/// Validates API keys against provider endpoints.
pub struct KeyValidator {
    /// Timeout for validation requests.
    timeout: std::time::Duration,
}

impl Default for KeyValidator {
    fn default() -> Self {
        Self {
            timeout: std::time::Duration::from_secs(15),
        }
    }
}

impl KeyValidator {
    /// Create a validator with a custom timeout.
    pub fn with_timeout(timeout: std::time::Duration) -> Self {
        Self { timeout }
    }

    /// Validate an API key for a known provider.
    ///
    /// Calls the provider's model list endpoint (or equivalent lightweight endpoint)
    /// to verify that the key is valid and has access.
    pub fn validate(
        &self,
        provider: &ProviderDescriptor,
        api_key: &str,
        base_url: Option<&str>,
    ) -> KeyValidationResult {
        let start = std::time::Instant::now();
        let url = base_url.unwrap_or(provider.default_base_url);

        // Local providers (Ollama) don't need key validation
        if provider.kind == ProviderKind::Local {
            return self.validate_local(url, start);
        }

        match provider.id {
            "anthropic" => self.validate_anthropic(api_key, url, start),
            "gemini" => self.validate_gemini(api_key, url, start),
            _ if provider.openai_compatible => self.validate_openai_compatible(api_key, url, provider.auth_scheme, start),
            _ => self.validate_openai_compatible(api_key, url, provider.auth_scheme, start),
        }
    }

    /// Validate a local provider (just check reachability).
    fn validate_local(&self, base_url: &str, start: std::time::Instant) -> KeyValidationResult {
        let url = format!("{}/api/tags", base_url.trim_end_matches("/v1").trim_end_matches('/'));
        let agent = self.build_agent();

        match agent.get(&url).call() {
            Ok(resp) => {
                let latency = start.elapsed().as_millis() as u64;
                let mut models = Vec::new();
                if let Ok(body) = resp.into_body().read_json::<serde_json::Value>() {
                    if let Some(model_list) = body["models"].as_array() {
                        models = model_list
                            .iter()
                            .filter_map(|m| m["name"].as_str().map(String::from))
                            .collect();
                    }
                }
                KeyValidationResult::success(latency, models)
            }
            Err(e) => {
                let latency = start.elapsed().as_millis() as u64;
                KeyValidationResult::failure(classify_error(&e.to_string()), latency)
            }
        }
    }

    /// Validate an OpenAI-compatible provider via /models endpoint.
    fn validate_openai_compatible(
        &self,
        api_key: &str,
        base_url: &str,
        auth_scheme: AuthScheme,
        start: std::time::Instant,
    ) -> KeyValidationResult {
        let url = format!("{}/models", base_url.trim_end_matches('/'));
        let agent = self.build_agent();
        let mut req = agent.get(&url);

        req = match auth_scheme {
            AuthScheme::Bearer => req.header("Authorization", &format!("Bearer {}", api_key)),
            AuthScheme::XApiKey => req.header("x-api-key", api_key),
            AuthScheme::QueryParam => {
                // Append key as query param
                let url_with_key = format!("{}?key={}", url, api_key);
                return self.validate_url_get(&url_with_key, start);
            }
            AuthScheme::None => req,
        };

        match req.call() {
            Ok(resp) => {
                let latency = start.elapsed().as_millis() as u64;
                let mut models = Vec::new();
                if let Ok(body) = resp.into_body().read_json::<serde_json::Value>() {
                    if let Some(data) = body["data"].as_array() {
                        models = data
                            .iter()
                            .filter_map(|m| m["id"].as_str().map(String::from))
                            .collect();
                    }
                }
                KeyValidationResult::success(latency, models)
            }
            Err(e) => {
                let latency = start.elapsed().as_millis() as u64;
                KeyValidationResult::failure(classify_ureq_error(&e), latency)
            }
        }
    }

    /// Validate Anthropic via a minimal messages request.
    fn validate_anthropic(
        &self,
        api_key: &str,
        base_url: &str,
        start: std::time::Instant,
    ) -> KeyValidationResult {
        // Anthropic doesn't have a /models endpoint in the standard API.
        // Use a minimal messages request with max_tokens=1 to validate the key.
        let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
        let agent = self.build_agent();

        let body = serde_json::json!({
            "model": "claude-3-5-haiku-20241022",
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "hi"}],
        });

        let body_str = serde_json::to_string(&body).unwrap_or_default();

        match agent
            .post(&url)
            .header("Content-Type", "application/json")
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .send(body_str.as_bytes())
        {
            Ok(_) => {
                let latency = start.elapsed().as_millis() as u64;
                // Key is valid — we don't get a model list from this endpoint
                KeyValidationResult::success(latency, vec![
                    "claude-3-5-haiku-20241022".to_string(),
                    "claude-sonnet-4-20250514".to_string(),
                ])
            }
            Err(e) => {
                let latency = start.elapsed().as_millis() as u64;
                KeyValidationResult::failure(classify_ureq_error(&e), latency)
            }
        }
    }

    /// Validate Google Gemini via a models list request.
    fn validate_gemini(
        &self,
        api_key: &str,
        base_url: &str,
        start: std::time::Instant,
    ) -> KeyValidationResult {
        let url = format!(
            "{}/v1beta/models?key={}",
            base_url.trim_end_matches('/'),
            api_key
        );
        self.validate_url_get(&url, start)
    }

    /// Simple GET validation against a URL.
    fn validate_url_get(&self, url: &str, start: std::time::Instant) -> KeyValidationResult {
        let agent = self.build_agent();
        match agent.get(url).call() {
            Ok(resp) => {
                let latency = start.elapsed().as_millis() as u64;
                let mut models = Vec::new();
                if let Ok(body) = resp.into_body().read_json::<serde_json::Value>() {
                    // Gemini returns { models: [{name: "models/gemini-pro", ...}] }
                    if let Some(model_list) = body["models"].as_array() {
                        models = model_list
                            .iter()
                            .filter_map(|m| m["name"].as_str().map(String::from))
                            .collect();
                    }
                    // OpenAI-style
                    if let Some(data) = body["data"].as_array() {
                        models = data
                            .iter()
                            .filter_map(|m| m["id"].as_str().map(String::from))
                            .collect();
                    }
                }
                KeyValidationResult::success(latency, models)
            }
            Err(e) => {
                let latency = start.elapsed().as_millis() as u64;
                KeyValidationResult::failure(classify_ureq_error(&e), latency)
            }
        }
    }

    fn build_agent(&self) -> ureq::Agent {
        ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(self.timeout))
                .build(),
        )
    }
}

// ── Error classification ─────────────────────────────────────────────────

/// Classify a ureq error into a KeyValidationError.
fn classify_ureq_error(err: &ureq::Error) -> KeyValidationError {
    let msg = err.to_string();
    classify_error(&msg)
}

/// Classify an error message string into a KeyValidationError.
fn classify_error(msg: &str) -> KeyValidationError {
    let lower = msg.to_lowercase();

    if lower.contains("401") || lower.contains("403") || lower.contains("unauthorized") || lower.contains("forbidden") || lower.contains("invalid api key") || lower.contains("invalid x-api-key") {
        KeyValidationError::InvalidKey
    } else if lower.contains("402") || lower.contains("payment") || lower.contains("billing") || lower.contains("insufficient") {
        KeyValidationError::NoBilling
    } else if lower.contains("429") || lower.contains("rate limit") || lower.contains("too many requests") {
        KeyValidationError::RateLimited
    } else if lower.contains("404") || lower.contains("model_not_found") || lower.contains("model not found") {
        KeyValidationError::ModelUnavailable
    } else if lower.contains("451") || lower.contains("geo") || lower.contains("region") || lower.contains("country") {
        KeyValidationError::RegionBlocked
    } else if lower.contains("tls") || lower.contains("ssl") || lower.contains("certificate") || lower.contains("handshake") {
        KeyValidationError::TLSError
    } else if lower.contains("timeout") || lower.contains("timed out") || lower.contains("connect") || lower.contains("dns") || lower.contains("resolve") {
        KeyValidationError::NetworkTimeout
    } else {
        KeyValidationError::Unknown(msg.to_string())
    }
}

// ── Validate-then-store helper ───────────────────────────────────────────

/// Validate an API key and store it in the SecretStore only on success.
///
/// Returns the validation result. If valid, the key is persisted in the store
/// under the provider's secret ref key.
pub fn validate_and_store(
    provider: &ProviderDescriptor,
    api_key: &str,
    base_url: Option<&str>,
    store: &dyn super::secret_store::SecretStore,
) -> KeyValidationResult {
    let validator = KeyValidator::default();
    let result = validator.validate(provider, api_key, base_url);

    if result.is_valid {
        let ref_key = format!("providers/{}", provider.id);
        if let Err(e) = store.put(&ref_key, api_key) {
            tracing::error!(provider = provider.id, error = %e, "failed to store validated key");
            return KeyValidationResult::failure(
                KeyValidationError::Unknown(format!("key valid but storage failed: {}", e)),
                result.latency_ms,
            );
        }
        tracing::info!(
            provider = provider.id,
            latency_ms = result.latency_ms,
            models = result.available_models.len(),
            "API key validated and stored"
        );
    }

    result
}
