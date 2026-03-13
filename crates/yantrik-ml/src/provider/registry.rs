//! ProviderRegistry — manages multiple LLM providers with routing and failover.
//!
//! Central registry that holds all configured providers, routes requests
//! by task type, monitors health, and manages fallback chains.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::traits::LLMBackend;
use crate::types::{ChatMessage, GenerationConfig, LLMResponse};

use super::generic_openai::GenericOpenAIBackend;
use super::anthropic::AnthropicBackend;
use super::gemini::GoogleGeminiBackend;
use super::health::{HealthStatus, ProviderHealth};
use super::routing::{TaskRoutes, TaskType};

/// Unique identifier for a provider instance.
pub type ProviderId = String;

/// Usage statistics for a provider.
#[derive(Debug, Clone, Default)]
pub struct UsageStats {
    /// Total number of requests sent to this provider.
    pub total_requests: u64,
    /// Number of successful requests.
    pub successful_requests: u64,
    /// Number of failed requests.
    pub failed_requests: u64,
    /// Total tokens consumed (prompt + completion).
    pub total_tokens: u64,
    /// Timestamp of the last request.
    pub last_request: Option<Instant>,
}

/// A registered provider instance with its backend, health, and stats.
pub struct ProviderEntry {
    /// The LLM backend implementation.
    pub backend: Arc<dyn LLMBackend>,
    /// Static metadata about this provider type.
    pub descriptor_id: String,
    /// Display name for this instance.
    pub display_name: String,
    /// Current health status.
    pub health: Mutex<ProviderHealth>,
    /// Usage statistics.
    pub stats: Mutex<UsageStats>,
}

impl ProviderEntry {
    /// Create a new ProviderEntry.
    pub fn new(
        backend: Arc<dyn LLMBackend>,
        descriptor_id: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Self {
        Self {
            backend,
            descriptor_id: descriptor_id.into(),
            display_name: display_name.into(),
            health: Mutex::new(ProviderHealth::default()),
            stats: Mutex::new(UsageStats::default()),
        }
    }

    /// Record a successful request.
    pub fn record_success(&self, tokens: u64) {
        if let Ok(mut stats) = self.stats.lock() {
            stats.total_requests += 1;
            stats.successful_requests += 1;
            stats.total_tokens += tokens;
            stats.last_request = Some(Instant::now());
        }
    }

    /// Record a failed request.
    pub fn record_failure(&self) {
        if let Ok(mut stats) = self.stats.lock() {
            stats.total_requests += 1;
            stats.failed_requests += 1;
            stats.last_request = Some(Instant::now());
        }
    }

    /// Check if this provider is currently usable.
    pub fn is_usable(&self) -> bool {
        self.health
            .lock()
            .map(|h| h.status.is_usable())
            .unwrap_or(false)
    }
}

/// Central registry managing multiple LLM providers.
///
/// Provides:
/// - Provider registration and lookup
/// - Task-based routing (Fast/Balanced/Powerful)
/// - Health monitoring and automatic failover
/// - Hot-swap support via atomic Arc swap
pub struct ProviderRegistry {
    /// All registered providers, keyed by ProviderId.
    providers: RwLock<HashMap<ProviderId, Arc<ProviderEntry>>>,
    /// The active primary provider ID.
    active_primary: RwLock<Option<ProviderId>>,
    /// Task-type routing configuration.
    task_routes: RwLock<TaskRoutes>,
    /// Ordered fallback chain: when primary fails, try these in order.
    fallback_chain: RwLock<Vec<ProviderId>>,
    /// How often to run health checks (default: 60s).
    health_check_interval: Duration,
}

impl ProviderRegistry {
    /// Create a new empty ProviderRegistry.
    pub fn new() -> Self {
        Self {
            providers: RwLock::new(HashMap::new()),
            active_primary: RwLock::new(None),
            task_routes: RwLock::new(TaskRoutes::default()),
            fallback_chain: RwLock::new(Vec::new()),
            health_check_interval: Duration::from_secs(60),
        }
    }

    /// Register a provider.
    pub fn register(
        &self,
        id: impl Into<ProviderId>,
        entry: ProviderEntry,
    ) {
        let id = id.into();
        tracing::info!(provider_id = %id, name = %entry.display_name, "Registering provider");
        if let Ok(mut providers) = self.providers.write() {
            providers.insert(id, Arc::new(entry));
        }
    }

    /// Set the active primary provider.
    pub fn set_primary(&self, id: impl Into<ProviderId>) {
        let id = id.into();
        tracing::info!(provider_id = %id, "Setting primary provider");
        if let Ok(mut primary) = self.active_primary.write() {
            *primary = Some(id);
        }
    }

    /// Set task routing configuration.
    pub fn set_task_routes(&self, routes: TaskRoutes) {
        if let Ok(mut tr) = self.task_routes.write() {
            *tr = routes;
        }
    }

    /// Set the fallback chain (ordered list of provider IDs to try on failure).
    pub fn set_fallback_chain(&self, chain: Vec<ProviderId>) {
        if let Ok(mut fc) = self.fallback_chain.write() {
            *fc = chain;
        }
    }

    /// Get a provider by ID.
    pub fn get(&self, id: &str) -> Option<Arc<ProviderEntry>> {
        self.providers
            .read()
            .ok()?
            .get(id)
            .cloned()
    }

    /// Get the active primary provider.
    pub fn primary(&self) -> Option<Arc<ProviderEntry>> {
        let id = self.active_primary.read().ok()?.clone()?;
        self.get(&id)
    }

    /// Get the primary provider's ID.
    pub fn primary_id(&self) -> Option<ProviderId> {
        self.active_primary.read().ok()?.clone()
    }

    /// List all registered provider IDs.
    pub fn provider_ids(&self) -> Vec<ProviderId> {
        self.providers
            .read()
            .map(|p| p.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Get the number of registered providers.
    pub fn len(&self) -> usize {
        self.providers.read().map(|p| p.len()).unwrap_or(0)
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Route a task to the appropriate provider.
    ///
    /// Checks task-specific routing first, then falls back to the primary provider.
    /// If the routed provider is unhealthy, falls back through the fallback chain.
    pub fn route_task(&self, task: TaskType) -> Option<Arc<ProviderEntry>> {
        // 1. Check task-specific routing
        let routed_id = self.task_routes.read().ok()
            .and_then(|tr| tr.get(task).map(|s| s.to_string()));

        if let Some(ref id) = routed_id {
            if let Some(entry) = self.get(id) {
                if entry.is_usable() {
                    return Some(entry);
                }
                tracing::warn!(provider = %id, task = ?task, "Routed provider unhealthy, falling back");
            }
        }

        // 2. Try primary
        if let Some(entry) = self.primary() {
            if entry.is_usable() {
                return Some(entry);
            }
            tracing::warn!("Primary provider unhealthy, checking fallback chain");
        }

        // 3. Walk the fallback chain
        if let Ok(chain) = self.fallback_chain.read() {
            for id in chain.iter() {
                if let Some(entry) = self.get(id) {
                    if entry.is_usable() {
                        tracing::info!(provider = %id, "Using fallback provider");
                        return Some(entry);
                    }
                }
            }
        }

        // 4. Last resort — return any usable provider
        if let Ok(providers) = self.providers.read() {
            for (id, entry) in providers.iter() {
                if entry.is_usable() {
                    tracing::warn!(provider = %id, "Using last-resort provider");
                    return Some(entry.clone());
                }
            }
        }

        tracing::error!("No usable providers available");
        None
    }

    /// Execute a chat request through the registry with automatic failover.
    ///
    /// Routes to the appropriate provider based on task type, retries on failure
    /// through the fallback chain.
    pub fn chat(
        &self,
        task: TaskType,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        // Try routed/primary provider first
        if let Some(entry) = self.route_task(task) {
            match entry.backend.chat(messages, config, tools) {
                Ok(resp) => {
                    let tokens = (resp.prompt_tokens + resp.completion_tokens) as u64;
                    entry.record_success(tokens);
                    if let Ok(mut h) = entry.health.lock() {
                        h.record_success(0); // We don't track latency per-request
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    entry.record_failure();
                    if let Ok(mut h) = entry.health.lock() {
                        h.record_failure(&e.to_string());
                    }
                    tracing::warn!(
                        provider = %entry.display_name,
                        error = %e,
                        "Provider failed, trying fallback chain"
                    );
                }
            }
        }

        // Try fallback chain
        if let Ok(chain) = self.fallback_chain.read() {
            for id in chain.iter() {
                if let Some(entry) = self.get(id) {
                    match entry.backend.chat(messages, config, tools) {
                        Ok(resp) => {
                            let tokens = (resp.prompt_tokens + resp.completion_tokens) as u64;
                            entry.record_success(tokens);
                            if let Ok(mut h) = entry.health.lock() {
                                h.record_success(0);
                            }
                            return Ok(resp);
                        }
                        Err(e) => {
                            entry.record_failure();
                            if let Ok(mut h) = entry.health.lock() {
                                h.record_failure(&e.to_string());
                            }
                            tracing::warn!(provider = %id, error = %e, "Fallback provider also failed");
                        }
                    }
                }
            }
        }

        anyhow::bail!("All providers failed")
    }

    /// Execute a streaming chat request through the registry with automatic failover.
    pub fn chat_streaming(
        &self,
        task: TaskType,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        if let Some(entry) = self.route_task(task) {
            match entry.backend.chat_streaming(messages, config, tools, on_token) {
                Ok(resp) => {
                    let tokens = (resp.prompt_tokens + resp.completion_tokens) as u64;
                    entry.record_success(tokens);
                    if let Ok(mut h) = entry.health.lock() {
                        h.record_success(0);
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    entry.record_failure();
                    if let Ok(mut h) = entry.health.lock() {
                        h.record_failure(&e.to_string());
                    }
                    tracing::warn!(
                        provider = %entry.display_name,
                        error = %e,
                        "Streaming provider failed, trying fallback chain"
                    );
                }
            }
        }

        // Fallback chain for streaming
        if let Ok(chain) = self.fallback_chain.read() {
            for id in chain.iter() {
                if let Some(entry) = self.get(id) {
                    match entry.backend.chat_streaming(messages, config, tools, on_token) {
                        Ok(resp) => {
                            let tokens = (resp.prompt_tokens + resp.completion_tokens) as u64;
                            entry.record_success(tokens);
                            if let Ok(mut h) = entry.health.lock() {
                                h.record_success(0);
                            }
                            return Ok(resp);
                        }
                        Err(e) => {
                            entry.record_failure();
                            if let Ok(mut h) = entry.health.lock() {
                                h.record_failure(&e.to_string());
                            }
                        }
                    }
                }
            }
        }

        anyhow::bail!("All providers failed (streaming)")
    }

    /// Run health checks on all registered providers.
    ///
    /// Probes each provider and updates its health status.
    /// Only checks providers whose health data is stale.
    pub fn run_health_checks(&self) {
        let ids = self.provider_ids();

        for id in &ids {
            let Some(entry) = self.get(id) else { continue };

            // Skip if health data is fresh
            if let Ok(h) = entry.health.lock() {
                if !h.is_stale(self.health_check_interval) {
                    continue;
                }
            }

            tracing::debug!(provider = %id, "Running health check");

            // Probe based on backend type
            let result = Self::probe_provider(&entry);

            match result {
                Ok(latency_ms) => {
                    if let Ok(mut h) = entry.health.lock() {
                        h.record_success(latency_ms);
                    }
                    tracing::debug!(provider = %id, latency_ms, "Health check passed");
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    if let Ok(mut h) = entry.health.lock() {
                        h.record_failure(&err_msg);
                    }
                    tracing::warn!(provider = %id, error = %err_msg, "Health check failed");
                }
            }
        }
    }

    /// Probe a specific provider for health.
    fn probe_provider(entry: &ProviderEntry) -> Result<u64> {
        let start = Instant::now();

        // Send a minimal chat to verify the provider works end-to-end
        let messages = vec![ChatMessage::user("ping")];
        let config = GenerationConfig {
            max_tokens: 1,
            temperature: 0.0,
            ..Default::default()
        };

        entry.backend.chat(&messages, &config, None)?;
        Ok(start.elapsed().as_millis() as u64)
    }

    /// Hot-swap a provider's backend (atomic replacement).
    ///
    /// Used for runtime reconfiguration without restarting.
    pub fn hot_swap(
        &self,
        id: &str,
        new_backend: Arc<dyn LLMBackend>,
        display_name: impl Into<String>,
        descriptor_id: impl Into<String>,
    ) {
        let entry = ProviderEntry::new(new_backend, descriptor_id, display_name);
        tracing::info!(provider_id = %id, "Hot-swapping provider");
        if let Ok(mut providers) = self.providers.write() {
            providers.insert(id.to_string(), Arc::new(entry));
        }
    }

    /// Remove a provider from the registry.
    pub fn unregister(&self, id: &str) {
        if let Ok(mut providers) = self.providers.write() {
            providers.remove(id);
        }
    }

    /// Create a provider backend from config values.
    ///
    /// Determines the appropriate backend type based on provider_type and creates it.
    pub fn create_backend(
        provider_type: &str,
        base_url: &str,
        api_key: Option<&str>,
        model: &str,
    ) -> Arc<dyn LLMBackend> {
        match provider_type {
            "anthropic" => {
                let key = api_key.unwrap_or("");
                Arc::new(AnthropicBackend::with_base_url(key, base_url, model))
            }
            "gemini" => {
                let key = api_key.unwrap_or("");
                Arc::new(GoogleGeminiBackend::with_base_url(key, base_url, model))
            }
            // All OpenAI-compatible providers
            _ => {
                Arc::new(GenericOpenAIBackend::for_provider(
                    provider_type,
                    base_url,
                    api_key.map(|s| s.to_string()),
                    model,
                ))
            }
        }
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// ProviderRegistry as an LLMBackend — delegates to the primary provider.
///
/// This allows the registry to be used as a drop-in replacement for a single
/// LLMBackend in existing code (e.g. CompanionService).
impl LLMBackend for ProviderRegistry {
    fn chat(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LLMResponse> {
        self.chat(TaskType::Balanced, messages, config, tools)
    }

    fn chat_streaming(
        &self,
        messages: &[ChatMessage],
        config: &GenerationConfig,
        tools: Option<&[serde_json::Value]>,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<LLMResponse> {
        self.chat_streaming(TaskType::Balanced, messages, config, tools, on_token)
    }

    fn count_tokens(&self, text: &str) -> Result<usize> {
        if let Some(entry) = self.primary() {
            entry.backend.count_tokens(text)
        } else {
            Ok(text.len() / 4)
        }
    }

    fn backend_name(&self) -> &str {
        "provider-registry"
    }

    fn is_degraded(&self) -> bool {
        self.primary()
            .map(|e| {
                e.health
                    .lock()
                    .map(|h| h.status != HealthStatus::Healthy)
                    .unwrap_or(true)
            })
            .unwrap_or(true)
    }

    fn model_id(&self) -> &str {
        // Return the primary provider's model ID
        // Can't return a reference to a temporary, so use a static fallback
        // The caller should use primary().backend.model_id() for the real value
        "registry"
    }
}
