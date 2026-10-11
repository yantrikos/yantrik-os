//! Provider descriptors — static metadata about known LLM providers.
//!
//! Each descriptor provides display info, default URLs, auth schemes,
//! default model, key hint and onboarding tier for the UI. The data itself
//! lives in `catalogue.rs`.

use serde::{Deserialize, Serialize};

/// Kind of provider (affects UX and trust model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderKind {
    /// Runs on user's machine (Ollama, llama.cpp).
    Local,
    /// Direct cloud API (OpenAI, Anthropic, Google, etc.).
    Cloud,
    /// Aggregator that proxies to multiple models (OpenRouter).
    Aggregator,
}

/// How the provider authenticates requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthScheme {
    /// No auth needed (e.g. local Ollama).
    None,
    /// HTTP Bearer token (`Authorization: Bearer <key>`).
    Bearer,
    /// Anthropic-style header (`x-api-key: <key>`).
    XApiKey,
    /// API key in query parameter (`?key=<key>`).
    QueryParam,
}

/// When this provider should be shown during setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SetupTier {
    /// Always shown in onboarding — primary providers most users will want.
    PrimaryOnboarding,
    /// Shown in settings but not during initial onboarding.
    Advanced,
    /// Hidden — only for power users who manually edit config.
    Expert,
}

/// Static metadata describing a known LLM provider.
///
/// [`KNOWN_PROVIDERS`] is the one list of providers the OS knows. Settings'
/// preset grid, first-boot onboarding, the installer and key validation all
/// read it; none of them keeps a list of its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    /// Canonical identifier (e.g. "ollama", "openai", "anthropic"). Stored as
    /// `provider_type` in providers.yaml, so an id never changes once shipped.
    pub id: &'static str,
    /// Human-readable display name, used as the saved provider's name.
    pub display_name: &'static str,
    /// Short label for a preset button, where the full name does not fit.
    pub short_name: &'static str,
    /// Kind of provider. Also the group a preset grid shows it under.
    pub kind: ProviderKind,
    /// Base URL of the provider's own API, which the native backends and key
    /// validation extend (with /v1 for OpenAI-compatible providers).
    pub default_base_url: &'static str,
    /// The provider's OpenAI-compatible endpoint, when its native API
    /// (`default_base_url`) is not one. The shell talks to every provider
    /// through chat completions, so this is what Settings and onboarding save.
    /// `None` when `default_base_url` already is that endpoint.
    #[serde(borrow)]
    pub openai_compat_base_url: Option<&'static str>,
    /// Authentication scheme of the native API.
    pub auth_scheme: AuthScheme,
    /// When to show this provider during setup.
    pub setup_tier: SetupTier,
    /// Whether `default_base_url` is an OpenAI-compatible chat completions API.
    pub openai_compatible: bool,
    /// Whether the provider supports streaming.
    pub supports_streaming: bool,
    /// Whether the provider supports native tool calling.
    pub supports_tools: bool,
    /// Model a new setup starts on. Empty when there is no sensible default
    /// (a local runtime serves whatever the person pulled).
    pub default_model: &'static str,
    /// What goes in the API key field before anything is typed ("sk-…").
    /// Empty exactly when the provider needs no key.
    pub key_placeholder: &'static str,
    /// The environment variable the vendor's own tools read the key from
    /// ("OPENAI_API_KEY"). Empty when there is no settled convention.
    pub key_env: &'static str,
    /// Brief description for the UI.
    pub description: &'static str,
}

pub use super::catalogue::KNOWN_PROVIDERS;

impl ProviderDescriptor {
    /// Look up a known provider by its canonical ID.
    pub fn by_id(id: &str) -> Option<&'static ProviderDescriptor> {
        KNOWN_PROVIDERS.iter().find(|p| p.id == id)
    }

    /// The OpenAI-compatible base URL: what a chat-completions client uses.
    pub fn openai_base_url(&self) -> &'static str {
        self.openai_compat_base_url.unwrap_or(self.default_base_url)
    }

    /// Whether the URL carries a part only the person knows, such as
    /// Cloudflare's `{account_id}`, to be filled in before it can work.
    pub fn url_needs_filling(&self) -> bool {
        self.openai_base_url().contains('{')
    }

    /// Whether requests need an API key.
    pub fn needs_key(&self) -> bool {
        self.auth_scheme != AuthScheme::None
    }

    /// Providers a person can pick in Settings: everything but the Expert tier.
    pub fn settings_providers() -> impl Iterator<Item = &'static ProviderDescriptor> {
        KNOWN_PROVIDERS.iter().filter(|p| p.setup_tier != SetupTier::Expert)
    }

    /// Return all providers suitable for onboarding.
    pub fn onboarding_providers() -> Vec<&'static ProviderDescriptor> {
        KNOWN_PROVIDERS
            .iter()
            .filter(|p| p.setup_tier == SetupTier::PrimaryOnboarding)
            .collect()
    }
}
