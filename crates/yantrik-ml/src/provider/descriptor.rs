//! Provider descriptors — static metadata about known LLM providers.
//!
//! Each descriptor provides display info, default URLs, auth schemes,
//! and onboarding tier for the UI.

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    /// Canonical identifier (e.g. "ollama", "openai", "anthropic").
    pub id: &'static str,
    /// Human-readable display name.
    pub display_name: &'static str,
    /// Kind of provider.
    pub kind: ProviderKind,
    /// Default base URL (with /v1 suffix for OpenAI-compatible providers).
    pub default_base_url: &'static str,
    /// Authentication scheme.
    pub auth_scheme: AuthScheme,
    /// When to show this provider during setup.
    pub setup_tier: SetupTier,
    /// Whether this provider uses the OpenAI-compatible chat completions API.
    pub openai_compatible: bool,
    /// Whether the provider supports streaming.
    pub supports_streaming: bool,
    /// Whether the provider supports native tool calling.
    pub supports_tools: bool,
    /// Brief description for the UI.
    pub description: &'static str,
}

/// Static registry of all known LLM providers.
pub static KNOWN_PROVIDERS: &[ProviderDescriptor] = &[
    ProviderDescriptor {
        id: "ollama",
        display_name: "Ollama (Local)",
        kind: ProviderKind::Local,
        default_base_url: "http://localhost:11434/v1",
        auth_scheme: AuthScheme::None,
        setup_tier: SetupTier::PrimaryOnboarding,
        openai_compatible: true,
        supports_streaming: true,
        supports_tools: true,
        description: "Run open models locally. No API key needed.",
    },
    ProviderDescriptor {
        id: "openai",
        display_name: "OpenAI",
        kind: ProviderKind::Cloud,
        default_base_url: "https://api.openai.com/v1",
        auth_scheme: AuthScheme::Bearer,
        setup_tier: SetupTier::PrimaryOnboarding,
        openai_compatible: true,
        supports_streaming: true,
        supports_tools: true,
        description: "GPT-4o, o1, and more. Requires API key.",
    },
    ProviderDescriptor {
        id: "anthropic",
        display_name: "Anthropic",
        kind: ProviderKind::Cloud,
        default_base_url: "https://api.anthropic.com",
        auth_scheme: AuthScheme::XApiKey,
        setup_tier: SetupTier::PrimaryOnboarding,
        openai_compatible: false,
        supports_streaming: true,
        supports_tools: true,
        description: "Claude Haiku, Sonnet, and Opus. Requires API key.",
    },
    ProviderDescriptor {
        id: "gemini",
        display_name: "Google Gemini",
        kind: ProviderKind::Cloud,
        default_base_url: "https://generativelanguage.googleapis.com",
        auth_scheme: AuthScheme::QueryParam,
        setup_tier: SetupTier::PrimaryOnboarding,
        openai_compatible: false,
        supports_streaming: true,
        supports_tools: true,
        description: "Gemini Flash, Pro, and Ultra. Requires API key.",
    },
    ProviderDescriptor {
        id: "deepseek",
        display_name: "DeepSeek",
        kind: ProviderKind::Cloud,
        default_base_url: "https://api.deepseek.com/v1",
        auth_scheme: AuthScheme::Bearer,
        setup_tier: SetupTier::PrimaryOnboarding,
        openai_compatible: true,
        supports_streaming: true,
        supports_tools: true,
        description: "DeepSeek V3 and R1. Affordable cloud inference.",
    },
    ProviderDescriptor {
        id: "openrouter",
        display_name: "OpenRouter",
        kind: ProviderKind::Aggregator,
        default_base_url: "https://openrouter.ai/api/v1",
        auth_scheme: AuthScheme::Bearer,
        setup_tier: SetupTier::PrimaryOnboarding,
        openai_compatible: true,
        supports_streaming: true,
        supports_tools: true,
        description: "Access 100+ models through one API key.",
    },
    ProviderDescriptor {
        id: "groq",
        display_name: "Groq",
        kind: ProviderKind::Cloud,
        default_base_url: "https://api.groq.com/openai/v1",
        auth_scheme: AuthScheme::Bearer,
        setup_tier: SetupTier::Advanced,
        openai_compatible: true,
        supports_streaming: true,
        supports_tools: true,
        description: "Ultra-fast inference on LPU hardware.",
    },
    ProviderDescriptor {
        id: "together",
        display_name: "Together AI",
        kind: ProviderKind::Cloud,
        default_base_url: "https://api.together.xyz/v1",
        auth_scheme: AuthScheme::Bearer,
        setup_tier: SetupTier::Advanced,
        openai_compatible: true,
        supports_streaming: true,
        supports_tools: true,
        description: "Open-source models at scale.",
    },
    ProviderDescriptor {
        id: "fireworks",
        display_name: "Fireworks AI",
        kind: ProviderKind::Cloud,
        default_base_url: "https://api.fireworks.ai/inference/v1",
        auth_scheme: AuthScheme::Bearer,
        setup_tier: SetupTier::Advanced,
        openai_compatible: true,
        supports_streaming: true,
        supports_tools: true,
        description: "Fast inference for open and fine-tuned models.",
    },
    ProviderDescriptor {
        id: "mistral",
        display_name: "Mistral AI",
        kind: ProviderKind::Cloud,
        default_base_url: "https://api.mistral.ai/v1",
        auth_scheme: AuthScheme::Bearer,
        setup_tier: SetupTier::Advanced,
        openai_compatible: true,
        supports_streaming: true,
        supports_tools: true,
        description: "Mistral, Mixtral, and Codestral models.",
    },
    ProviderDescriptor {
        id: "huggingface",
        display_name: "Hugging Face",
        kind: ProviderKind::Cloud,
        default_base_url: "https://api-inference.huggingface.co/v1",
        auth_scheme: AuthScheme::Bearer,
        setup_tier: SetupTier::Advanced,
        openai_compatible: true,
        supports_streaming: true,
        supports_tools: false,
        description: "Serverless inference for open models.",
    },
    ProviderDescriptor {
        id: "xai",
        display_name: "xAI (Grok)",
        kind: ProviderKind::Cloud,
        default_base_url: "https://api.x.ai/v1",
        auth_scheme: AuthScheme::Bearer,
        setup_tier: SetupTier::Advanced,
        openai_compatible: true,
        supports_streaming: true,
        supports_tools: true,
        description: "Grok models from xAI.",
    },
];

impl ProviderDescriptor {
    /// Look up a known provider by its canonical ID.
    pub fn by_id(id: &str) -> Option<&'static ProviderDescriptor> {
        KNOWN_PROVIDERS.iter().find(|p| p.id == id)
    }

    /// Return all providers suitable for onboarding.
    pub fn onboarding_providers() -> Vec<&'static ProviderDescriptor> {
        KNOWN_PROVIDERS
            .iter()
            .filter(|p| p.setup_tier == SetupTier::PrimaryOnboarding)
            .collect()
    }
}
