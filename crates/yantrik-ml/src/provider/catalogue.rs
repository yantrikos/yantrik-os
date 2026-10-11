//! The provider catalogue: every LLM provider the OS knows, in one list.
//!
//! Settings' preset grid, first-boot onboarding, the installer and key
//! validation all derive from [`KNOWN_PROVIDERS`]. Before this list was the
//! only one there were four, and they disagreed (Gemini's URL, Anthropic's
//! default model, whether onboarding's "google" was a provider at all).
//!
//! Order matters: a preset grid shows providers in this order within each
//! [`ProviderKind`] group.

use super::descriptor::{AuthScheme, ProviderDescriptor, ProviderKind, SetupTier};

/// A hosted, OpenAI-compatible API that takes a bearer key. Entries override
/// what differs.
const CLOUD: ProviderDescriptor = ProviderDescriptor {
    id: "",
    display_name: "",
    short_name: "",
    kind: ProviderKind::Cloud,
    default_base_url: "",
    openai_compat_base_url: None,
    auth_scheme: AuthScheme::Bearer,
    setup_tier: SetupTier::Advanced,
    openai_compatible: true,
    supports_streaming: true,
    supports_tools: true,
    default_model: "",
    key_placeholder: "Your API key",
    key_env: "",
    description: "",
};

/// One key, many vendors' models.
const AGGREGATOR: ProviderDescriptor = ProviderDescriptor { kind: ProviderKind::Aggregator, ..CLOUD };

/// A runtime the person hosts: no key, and the URL is a localhost guess.
const LOCAL: ProviderDescriptor = ProviderDescriptor {
    kind: ProviderKind::Local,
    auth_scheme: AuthScheme::None,
    key_placeholder: "",
    ..CLOUD
};

/// Static registry of all known LLM providers.
pub static KNOWN_PROVIDERS: &[ProviderDescriptor] = &[
    // ── Cloud ──
    ProviderDescriptor {
        id: "openai",
        display_name: "OpenAI",
        short_name: "OpenAI",
        default_base_url: "https://api.openai.com/v1",
        setup_tier: SetupTier::PrimaryOnboarding,
        default_model: "gpt-4o-mini",
        key_placeholder: "sk-…",
        key_env: "OPENAI_API_KEY",
        description: "GPT-4o, o1, and more. Requires API key.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "anthropic",
        display_name: "Anthropic",
        short_name: "Anthropic",
        // Native Messages API; AnthropicBackend appends /v1/messages.
        default_base_url: "https://api.anthropic.com",
        openai_compat_base_url: Some("https://api.anthropic.com/v1"),
        auth_scheme: AuthScheme::XApiKey,
        setup_tier: SetupTier::PrimaryOnboarding,
        openai_compatible: false,
        default_model: "claude-sonnet-5-5",
        key_placeholder: "sk-ant-…",
        key_env: "ANTHROPIC_API_KEY",
        description: "Claude Haiku, Sonnet, and Opus. Requires API key.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "gemini",
        display_name: "Google Gemini",
        short_name: "Google",
        // Native generateContent API; GoogleGeminiBackend appends /v1beta/models.
        default_base_url: "https://generativelanguage.googleapis.com",
        openai_compat_base_url: Some("https://generativelanguage.googleapis.com/v1beta/openai"),
        auth_scheme: AuthScheme::QueryParam,
        setup_tier: SetupTier::PrimaryOnboarding,
        openai_compatible: false,
        default_model: "gemini-3.8-flash",
        key_placeholder: "AIza…",
        key_env: "GEMINI_API_KEY",
        description: "Gemini Flash and Pro. Requires API key.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "deepseek",
        display_name: "DeepSeek",
        short_name: "DeepSeek",
        default_base_url: "https://api.deepseek.com/v1",
        setup_tier: SetupTier::PrimaryOnboarding,
        default_model: "deepseek-flash",
        key_placeholder: "sk-…",
        description: "DeepSeek V3 and R1. Affordable cloud inference.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "groq",
        display_name: "Groq",
        short_name: "Groq",
        default_base_url: "https://api.groq.com/openai/v1",
        default_model: "llama-3.3-70b-versatile",
        key_placeholder: "gsk_…",
        description: "Ultra-fast inference on LPU hardware.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "mistral",
        display_name: "Mistral AI",
        short_name: "Mistral",
        default_base_url: "https://api.mistral.ai/v1",
        default_model: "mistral-small-2603",
        description: "Mistral, Mixtral, and Codestral models.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "xai",
        display_name: "xAI Grok",
        short_name: "xAI Grok",
        default_base_url: "https://api.x.ai/v1",
        default_model: "grok-4.7",
        key_placeholder: "xai-…",
        description: "Grok models from xAI.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "perplexity",
        display_name: "Perplexity",
        short_name: "Perplexity",
        default_base_url: "https://api.perplexity.ai",
        supports_tools: false,
        default_model: "sonar",
        key_placeholder: "pplx-…",
        description: "Sonar models with live web search.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "cerebras",
        display_name: "Cerebras",
        short_name: "Cerebras",
        default_base_url: "https://api.cerebras.ai/v1",
        default_model: "gpt-oss-120b",
        key_placeholder: "csk-…",
        description: "Fast open-model inference on wafer-scale hardware.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "sambanova",
        display_name: "SambaNova",
        short_name: "SambaNova",
        default_base_url: "https://api.sambanova.ai/v1",
        default_model: "Meta-Llama-3.3-70B-Instruct",
        description: "Fast open-model inference on RDU hardware.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "qwen",
        display_name: "Qwen",
        short_name: "Qwen",
        default_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        default_model: "qwen-plus",
        key_placeholder: "sk-…",
        description: "Alibaba's Qwen models through DashScope.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "minimax",
        display_name: "MiniMax",
        short_name: "MiniMax",
        default_base_url: "https://api.minimax.io/v1",
        default_model: "MiniMax-M3",
        description: "MiniMax long-context models.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "kimi",
        display_name: "Kimi",
        short_name: "Kimi",
        // International endpoint; mainland China is https://api.moonshot.cn/v1.
        default_base_url: "https://api.moonshot.ai/v1",
        default_model: "kimi-k3",
        key_placeholder: "sk-…",
        description: "Moonshot AI's Kimi models.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "baidu",
        display_name: "Baidu",
        short_name: "Baidu",
        default_base_url: "https://qianfan.baidubce.com/v2",
        description: "ERNIE models through Baidu Qianfan.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "zhipu",
        display_name: "Zhipu GLM",
        short_name: "Zhipu GLM",
        default_base_url: "https://open.bigmodel.cn/api/paas/v4",
        description: "GLM models from Zhipu AI.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "nvidia-nim",
        display_name: "NVIDIA NIM",
        short_name: "NVIDIA NIM",
        default_base_url: "https://integrate.api.nvidia.com/v1",
        setup_tier: SetupTier::PrimaryOnboarding,
        default_model: "nvidia/nemotron-3-super-120b-a12b",
        key_placeholder: "nvapi-…",
        description: "NVIDIA-hosted open models. Self-hosted NIM: http://<host>:8000/v1, no key.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "ollama-cloud",
        display_name: "Ollama Cloud",
        short_name: "Ollama Cloud",
        default_base_url: "https://ollama.com/v1",
        key_env: "OLLAMA_API_KEY",
        description: "Ollama's hosted models, no local install. Requires API key.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "cohere",
        display_name: "Cohere",
        short_name: "Cohere",
        // Cohere's Compatibility API; its native v2 API is not OpenAI-shaped.
        default_base_url: "https://api.cohere.ai/compatibility/v1",
        default_model: "command-a-plus-05-2026",
        key_env: "COHERE_API_KEY",
        description: "Cohere's Command models through its OpenAI compatibility API.",
        ..CLOUD
    },
    ProviderDescriptor {
        id: "cloudflare",
        display_name: "Cloudflare Workers AI",
        short_name: "Cloudflare",
        // {account_id} is the person's own; Settings shows it to be filled in.
        default_base_url: "https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1",
        default_model: "@cf/meta/llama-3.3-70b-instruct-fp8-fast",
        key_placeholder: "Your API token",
        description: "Open models on Cloudflare's network. Put your account ID in the URL.",
        ..CLOUD
    },
    // ── Aggregators ──
    ProviderDescriptor {
        id: "openrouter",
        display_name: "OpenRouter",
        short_name: "OpenRouter",
        default_base_url: "https://openrouter.ai/api/v1",
        setup_tier: SetupTier::PrimaryOnboarding,
        default_model: "meta-llama/llama-3.3-70b-instruct",
        key_placeholder: "sk-or-…",
        description: "Access 100+ models through one API key.",
        ..AGGREGATOR
    },
    ProviderDescriptor {
        id: "together",
        display_name: "Together AI",
        short_name: "Together",
        default_base_url: "https://api.together.xyz/v1",
        default_model: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        description: "Open-source models at scale.",
        ..AGGREGATOR
    },
    ProviderDescriptor {
        id: "fireworks",
        display_name: "Fireworks AI",
        short_name: "Fireworks",
        default_base_url: "https://api.fireworks.ai/inference/v1",
        description: "Fast inference for open and fine-tuned models.",
        ..AGGREGATOR
    },
    ProviderDescriptor {
        id: "huggingface",
        display_name: "Hugging Face",
        short_name: "HuggingFace",
        default_base_url: "https://router.huggingface.co/v1",
        supports_tools: false,
        default_model: "meta-llama/Llama-3.3-70B-Instruct",
        key_placeholder: "hf_…",
        description: "Serverless inference for open models.",
        ..AGGREGATOR
    },
    ProviderDescriptor {
        id: "nanogpt",
        display_name: "NanoGPT",
        short_name: "NanoGPT",
        default_base_url: "https://api.nano-gpt.com/v1",
        description: "Pay-as-you-go access to many vendors' models.",
        ..AGGREGATOR
    },
    ProviderDescriptor {
        id: "nous",
        display_name: "Nous Portal",
        short_name: "Nous",
        default_base_url: "https://inference-api.nousresearch.com/v1",
        key_env: "NOUS_API_KEY",
        description: "Nous Research's Hermes and other models, with a Portal API key.",
        ..AGGREGATOR
    },
    ProviderDescriptor {
        id: "deepinfra",
        display_name: "DeepInfra",
        short_name: "DeepInfra",
        default_base_url: "https://api.deepinfra.com/v1/openai",
        default_model: "deepseek-ai/DeepSeek-V4-Flash-0731",
        key_env: "DEEPINFRA_API_KEY",
        description: "Hosted open models at low per-token prices.",
        ..AGGREGATOR
    },
    ProviderDescriptor {
        id: "vercel",
        display_name: "Vercel AI Gateway",
        short_name: "Vercel",
        default_base_url: "https://ai-gateway.vercel.sh/v1",
        default_model: "anthropic/claude-opus-5",
        key_env: "AI_GATEWAY_API_KEY",
        description: "Many vendors' models through Vercel's gateway and one key.",
        ..AGGREGATOR
    },
    ProviderDescriptor {
        id: "novita",
        display_name: "Novita AI",
        short_name: "Novita",
        default_base_url: "https://api.novita.ai/openai",
        default_model: "deepseek/deepseek-v4.1-flash",
        description: "Hosted open models, pay as you go.",
        ..AGGREGATOR
    },
    ProviderDescriptor {
        id: "nebius",
        display_name: "Nebius Token Factory",
        short_name: "Nebius",
        default_base_url: "https://api.tokenfactory.nebius.com/v1",
        default_model: "deepseek-ai/DeepSeek-R1-0528",
        key_env: "NEBIUS_API_KEY",
        description: "Open models on Nebius (formerly AI Studio).",
        ..AGGREGATOR
    },
    ProviderDescriptor {
        id: "chutes",
        display_name: "Chutes",
        short_name: "Chutes",
        default_base_url: "https://llm.chutes.ai/v1",
        default_model: "zai-org/GLM-5.2-TEE",
        key_placeholder: "cpk_…",
        description: "Open models on a decentralized GPU network.",
        ..AGGREGATOR
    },
    ProviderDescriptor {
        id: "requesty",
        display_name: "Requesty",
        short_name: "Requesty",
        default_base_url: "https://router.requesty.ai/v1",
        default_model: "openai/gpt-4o",
        key_placeholder: "sk-…",
        key_env: "REQUESTY_API_KEY",
        description: "A router across many vendors' models with one key.",
        ..AGGREGATOR
    },
    ProviderDescriptor {
        id: "venice",
        display_name: "Venice",
        short_name: "Venice",
        default_base_url: "https://api.venice.ai/api/v1",
        default_model: "zai-org-glm-5",
        key_env: "VENICE_API_KEY",
        description: "Privacy-focused hosting of open models.",
        ..AGGREGATOR
    },
    // ── Local ──
    ProviderDescriptor {
        id: "ollama",
        display_name: "Ollama",
        short_name: "Ollama",
        default_base_url: "http://localhost:11434/v1",
        setup_tier: SetupTier::PrimaryOnboarding,
        default_model: "nemotron-3-nano:4b",
        description: "Run open models locally. No API key needed.",
        ..LOCAL
    },
    ProviderDescriptor {
        id: "llamacpp",
        display_name: "llama.cpp",
        short_name: "llama.cpp",
        default_base_url: "http://localhost:8080/v1",
        description: "llama.cpp's llama-server on this machine. No API key needed.",
        ..LOCAL
    },
    ProviderDescriptor {
        id: "lmstudio",
        display_name: "LM Studio",
        short_name: "LM Studio",
        default_base_url: "http://localhost:1234/v1",
        description: "LM Studio's local server. No API key needed.",
        ..LOCAL
    },
    ProviderDescriptor {
        id: "vllm",
        display_name: "vLLM",
        short_name: "vLLM",
        default_base_url: "http://localhost:8000/v1",
        description: "A vLLM server you host. No API key needed.",
        ..LOCAL
    },
    ProviderDescriptor {
        id: "localai",
        display_name: "LocalAI",
        short_name: "LocalAI",
        default_base_url: "http://localhost:8080/v1",
        description: "A LocalAI server you host. No API key unless you set one.",
        ..LOCAL
    },
    ProviderDescriptor {
        id: "jan",
        display_name: "Jan",
        short_name: "Jan",
        default_base_url: "http://127.0.0.1:1337/v1",
        description: "Jan's local API server. No API key unless you set one in Jan.",
        ..LOCAL
    },
    ProviderDescriptor {
        id: "litellm",
        display_name: "LiteLLM Proxy",
        short_name: "LiteLLM",
        default_base_url: "http://localhost:4000/v1",
        // The proxy will not start without a master key, so one is always sent.
        auth_scheme: AuthScheme::Bearer,
        key_placeholder: "sk-…",
        description: "A LiteLLM proxy you host, in front of any providers. Needs its master or a virtual key.",
        ..LOCAL
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_provider_has_an_id_a_name_and_a_url() {
        for p in KNOWN_PROVIDERS {
            assert!(!p.id.is_empty(), "{p:?}");
            assert!(!p.display_name.is_empty(), "{}", p.id);
            assert!(!p.short_name.is_empty(), "{}", p.id);
            assert!(!p.default_base_url.is_empty(), "{}", p.id);
            assert!(!p.openai_base_url().is_empty(), "{}", p.id);
            assert!(!p.description.is_empty(), "{}", p.id);
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut seen = HashSet::new();
        for p in KNOWN_PROVIDERS {
            assert!(seen.insert(p.id), "duplicate id {}", p.id);
        }
    }

    #[test]
    fn by_id_finds_every_provider_and_nothing_else() {
        for p in KNOWN_PROVIDERS {
            assert_eq!(ProviderDescriptor::by_id(p.id).map(|d| d.id), Some(p.id));
        }
        assert!(ProviderDescriptor::by_id("custom").is_none());
        assert!(ProviderDescriptor::by_id("google").is_none(), "Gemini's id is gemini");
    }

    #[test]
    fn nvidia_nim_is_a_hosted_openai_compatible_cloud() {
        let nim = ProviderDescriptor::by_id("nvidia-nim").expect("NIM is in the catalogue");
        assert_eq!(nim.display_name, "NVIDIA NIM");
        assert_eq!(nim.kind, ProviderKind::Cloud);
        assert_eq!(nim.default_base_url, "https://integrate.api.nvidia.com/v1");
        assert_eq!(nim.openai_base_url(), "https://integrate.api.nvidia.com/v1");
        assert_eq!(nim.auth_scheme, AuthScheme::Bearer);
        assert!(nim.openai_compatible && nim.supports_streaming && nim.supports_tools);
        assert_eq!(nim.key_placeholder, "nvapi-…");
        assert_eq!(nim.setup_tier, SetupTier::PrimaryOnboarding);
    }

    #[test]
    fn a_placeholder_is_shown_exactly_when_a_key_is_needed() {
        for p in KNOWN_PROVIDERS {
            assert_eq!(p.key_placeholder.is_empty(), !p.needs_key(), "{}", p.id);
            // Everything hosted needs a key; a local runtime may (LiteLLM does).
            if p.kind != ProviderKind::Local {
                assert!(p.needs_key(), "{}", p.id);
            }
        }
    }

    #[test]
    fn a_native_api_that_is_not_openai_compatible_names_the_one_that_is() {
        // The shell talks chat completions to everything, so each provider
        // needs an OpenAI-compatible URL, whatever its native API is.
        for p in KNOWN_PROVIDERS {
            assert_eq!(p.openai_compat_base_url.is_some(), !p.openai_compatible, "{}", p.id);
        }
        let gemini = ProviderDescriptor::by_id("gemini").unwrap();
        assert_eq!(gemini.openai_base_url(), "https://generativelanguage.googleapis.com/v1beta/openai");
        assert_eq!(gemini.default_base_url, "https://generativelanguage.googleapis.com");
        let anthropic = ProviderDescriptor::by_id("anthropic").unwrap();
        assert_eq!(anthropic.openai_base_url(), "https://api.anthropic.com/v1");
        assert_eq!(anthropic.default_base_url, "https://api.anthropic.com");
    }

    #[test]
    fn the_major_clouds_are_offered_at_first_boot() {
        let ids: Vec<&str> = ProviderDescriptor::onboarding_providers().iter().map(|p| p.id).collect();
        for id in ["openai", "anthropic", "gemini", "nvidia-nim", "ollama"] {
            assert!(ids.contains(&id), "{id} missing from onboarding: {ids:?}");
        }
    }

    #[test]
    fn nous_portal_is_an_aggregator_used_with_an_api_key() {
        let nous = ProviderDescriptor::by_id("nous").expect("Nous Portal is in the catalogue");
        assert_eq!(nous.display_name, "Nous Portal");
        assert_eq!(nous.kind, ProviderKind::Aggregator);
        assert_eq!(nous.openai_base_url(), "https://inference-api.nousresearch.com/v1");
        assert_eq!(nous.auth_scheme, AuthScheme::Bearer);
        assert_eq!(nous.key_env, "NOUS_API_KEY");
    }

    #[test]
    fn only_a_url_the_person_must_complete_has_a_placeholder_in_it() {
        let filled: Vec<&str> = KNOWN_PROVIDERS.iter().filter(|p| p.url_needs_filling()).map(|p| p.id).collect();
        assert_eq!(filled, ["cloudflare"]);
    }

    #[test]
    fn retired_services_are_not_offered() {
        // GitHub Models retired 2026-07-30; Hyperbolic retired its serverless API.
        for id in ["github", "github-models", "hyperbolic"] {
            assert!(ProviderDescriptor::by_id(id).is_none(), "{id}");
        }
    }
}
